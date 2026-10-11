//! Dormant single-write mailbox. No App, runtime loop, socket or startup wiring.
//!
//! The frontend endpoint only submits/drains synchronously. The worker alone owns
//! HostController. One slot spans queued, reserved/in-flight and undrained terminal
//! work; reservation happens before a gate future is polled. No retry/replay path.
//!
//! A future runtime MUST retain/pin handle_one across ordinary UI/lifecycle select
//! branches, rather than recreating it each tick. Its mutable controller borrow
//! serializes lifecycle work; the runtime must bound writes and service lifecycle
//! afterward. Only an explicit interruption/deadline may drop an attempted future,
//! which leaves Uncertain in the slot. No timeout value or scheduling policy here.
//! Known terminal results are stored synchronously before returning and survive
//! backpressure, UI endpoint drop and shutdown. Volatile process death is not
//! recoverable, and a new mailbox cannot recover or replay an old command.
//! All payload containers are non-Debug/non-Display and never logged/serialized.

use super::{
    controller::{HostController, PresentationHost},
    projection::valid_id,
    session::{SessionBackend, SessionClock, SessionWriteError, WriteAdmissionStamp},
    write_dispatch::DispatchError,
};
use crate::frontend::{
    data::ComposedBody,
    editor_submission::{NotDispatchedReason, SubmissionTerminal, UncertainReason},
    envelopes::{FrontendIntent, FrontendResult, RequestContext, RequestEnvelope, WriteOutcome},
};
use std::sync::{Arc, Mutex, MutexGuard};

/// Conclusive pre-ownership decisions only. A lost worker/result after Accepted
/// must be reduced as missing completion, NEVER reported as a queue refusal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommandAdmission {
    Accepted,
    Full,
    Closed,
    NotReady,
    InvalidRequest,
}

struct QueuedCommand {
    request: RequestEnvelope,
    stamp: WriteAdmissionStamp,
}

enum Slot {
    Empty,
    Queued(QueuedCommand),
    Reserved,
    Terminal(SubmissionTerminal),
}

struct Mailbox {
    open: bool,
    published: WriteAdmissionStamp,
    slot: Slot,
}

type Shared = Arc<Mutex<Mailbox>>;

fn lock(shared: &Shared) -> MutexGuard<'_, Mailbox> {
    // No user callbacks/await/logging under this short internal state lock.
    shared.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// No clone, waiter, cancellation, lifecycle or capability accessor.
pub(crate) struct CommandMailbox {
    shared: Shared,
}

impl CommandMailbox {
    pub(crate) fn try_submit(&self, request: RequestEnvelope) -> CommandAdmission {
        // Validate before ownership and before any gate call. The caller retains
        // its original envelope/draft; this method consumes only its owned copy.
        if !valid_request(&request) {
            return CommandAdmission::InvalidRequest;
        }
        let mut mailbox = lock(&self.shared);
        if !mailbox.open {
            return CommandAdmission::Closed;
        }
        if !matches!(mailbox.slot, Slot::Empty) {
            return CommandAdmission::Full;
        }
        if !mailbox.published.ready() {
            return CommandAdmission::NotReady;
        }
        let stamp = mailbox.published;
        mailbox.slot = Slot::Queued(QueuedCommand { request, stamp });
        CommandAdmission::Accepted
    }

    /// At most one event; queued/in-flight work cannot be drained or cancelled.
    pub(crate) fn drain(&self) -> Option<SubmissionTerminal> {
        let mut mailbox = lock(&self.shared);
        if !matches!(mailbox.slot, Slot::Terminal(_)) {
            return None;
        }
        let Slot::Terminal(terminal) = std::mem::replace(&mut mailbox.slot, Slot::Empty) else {
            unreachable!("private terminal slot")
        };
        Some(terminal)
    }
}

pub(crate) struct CommandWorker<B: SessionBackend, C: SessionClock, H: PresentationHost> {
    controller: HostController<B, C, H>,
    shared: Shared,
}

impl<B: SessionBackend, C: SessionClock, H: PresentationHost> CommandWorker<B, C, H> {
    pub(crate) fn new(controller: HostController<B, C, H>) -> (CommandMailbox, Self) {
        let shared = Arc::new(Mutex::new(Mailbox {
            open: true,
            published: controller.write_admission_stamp(),
            slot: Slot::Empty,
        }));
        (
            CommandMailbox {
                shared: Arc::clone(&shared),
            },
            Self { controller, shared },
        )
    }

    /// Worker-side publication after lifecycle/snapshot work. This is only a
    /// pre-admission hint; actual readiness/generation is checked again at dequeue.
    pub(crate) fn publish_admission(&mut self) {
        lock(&self.shared).published = self.controller.write_admission_stamp();
    }

    /// Explicit fake-callable single operation, not a spawned task/runtime loop.
    /// UI handle/waiter drop does not drop this worker-owned future. Dropping this
    /// future after polling the gate is an explicit interruption, never a retry.
    pub(crate) async fn handle_one(&mut self) -> bool {
        let Some((command, mut reservation)) = reserve(&self.shared) else {
            return false;
        };
        // Check the actual lease-aware state, not just the published hint. Even
        // if a later generation is ready, an old command must refuse immediately.
        let actual = self.controller.write_admission_stamp();
        if command.stamp != actual || !actual.ready() {
            reservation.finish(SubmissionTerminal::NotDispatched {
                context: command.request.context,
                reason: NotDispatchedReason::NotReady,
            });
            self.publish_admission();
            return true;
        }
        let terminal =
            dispatch_envelope(&mut self.controller, command.request, &mut reservation).await;
        reservation.finish(terminal);
        self.publish_admission();
        true
    }

    /// Stop admission and settle unstarted work. An active handle_one must first
    /// finish or be explicitly interrupted by its owner; its RAII guard preserves
    /// that terminal before shutdown can acquire the worker again. No event is
    /// overwritten, and the frontend may still drain after shutdown/worker drop.
    pub(crate) fn shutdown(&mut self) {
        close(&self.shared);
    }
}

impl<B: SessionBackend, C: SessionClock, H: PresentationHost> Drop for CommandWorker<B, C, H> {
    fn drop(&mut self) {
        close(&self.shared);
    }
}

fn close(shared: &Shared) {
    let mut mailbox = lock(shared);
    mailbox.open = false;
    if matches!(mailbox.slot, Slot::Queued(_)) {
        let Slot::Queued(command) = std::mem::replace(&mut mailbox.slot, Slot::Empty) else {
            unreachable!("private queued slot")
        };
        mailbox.slot = Slot::Terminal(SubmissionTerminal::NotDispatched {
            context: command.request.context,
            reason: NotDispatchedReason::UnstartedShutdown,
        });
    }
    // Empty/reserved/terminal state stays exact, including already known success.
}

struct Reservation {
    shared: Shared,
    context: RequestContext,
    attempted: bool,
    finished: bool,
}

fn reserve(shared: &Shared) -> Option<(QueuedCommand, Reservation)> {
    let mut mailbox = lock(shared);
    if !matches!(mailbox.slot, Slot::Queued(_)) {
        return None;
    }
    let Slot::Queued(command) = std::mem::replace(&mut mailbox.slot, Slot::Reserved) else {
        unreachable!("private queued slot")
    };
    let reservation = Reservation {
        shared: Arc::clone(shared),
        context: command.request.context.clone(),
        attempted: false,
        finished: false,
    };
    Some((command, reservation))
}

impl Reservation {
    fn finish(&mut self, terminal: SubmissionTerminal) {
        let mut mailbox = lock(&self.shared);
        if matches!(mailbox.slot, Slot::Reserved) {
            mailbox.slot = Slot::Terminal(terminal);
        }
        self.finished = true;
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.finished {
            let terminal = if self.attempted {
                SubmissionTerminal::Uncertain {
                    context: self.context.clone(),
                    reason: UncertainReason::AttemptInterrupted,
                }
            } else {
                SubmissionTerminal::NotDispatched {
                    context: self.context.clone(),
                    reason: NotDispatchedReason::UnstartedShutdown,
                }
            };
            self.finish(terminal);
        }
    }
}

/// Envelope-level dispatch; reducer-free, capability-private, at most one gate
/// invocation. The existing coordinator/ledger remain untouched. Only this
/// documented pre-I/O error channel can prove NotDispatched after gate polling.
async fn dispatch_envelope<B: SessionBackend, C: SessionClock, H: PresentationHost>(
    controller: &mut HostController<B, C, H>,
    request: RequestEnvelope,
    reservation: &mut Reservation,
) -> SubmissionTerminal {
    // Conservative boundary immediately before polling the one retained future.
    // Use the concrete controller future: the older WriteGate trait does not
    // promise Send. No reducer or controller/client capability crosses to App.
    reservation.attempted = true;
    let result = controller.dispatch_write(request.intent.clone()).await;
    complete(request, result)
}

fn complete(
    request: RequestEnvelope,
    result: Result<FrontendResult, SessionWriteError>,
) -> SubmissionTerminal {
    match result {
        Err(error) => SubmissionTerminal::NotDispatched {
            context: request.context,
            reason: match error {
                SessionWriteError::NotReady => NotDispatchedReason::NotReady,
                SessionWriteError::Expired => NotDispatchedReason::Expired,
                SessionWriteError::Failed => NotDispatchedReason::SessionFailed,
                SessionWriteError::Dispatch(DispatchError::WrongIntent) => {
                    NotDispatchedReason::WrongIntent
                }
            },
        },
        Ok(result) => {
            if !valid_success(&request.intent, &result) {
                return SubmissionTerminal::Uncertain {
                    context: request.context,
                    reason: UncertainReason::CompletionMismatch,
                };
            }
            let context = request.context.clone();
            match request.complete(result) {
                Ok(envelope) => SubmissionTerminal::result(envelope),
                Err(_) => SubmissionTerminal::Uncertain {
                    context,
                    reason: UncertainReason::CompletionMismatch,
                },
            }
        }
    }
}

fn valid_request(request: &RequestEnvelope) -> bool {
    if request.context.operation != request.intent.operation()
        || request.context.room_view.as_ref().is_some_and(|view| {
            request.intent.room_id().map(|id| id.as_str()) != Some(view.room_id.as_str())
        })
    {
        return false;
    }
    let (room, anchor, body) = match &request.intent {
        FrontendIntent::SendMessage {
            room_id,
            parent_id,
            body,
        } => (room_id, parent_id.as_ref(), body),
        FrontendIntent::EditMessage {
            room_id,
            message_id,
            body,
        } => (room_id, Some(message_id), body),
        _ => return false, // No reads/delete/leave or scope expansion.
    };
    let (ComposedBody::PlainText(content) | ComposedBody::Markdown(content)) = body;
    valid_id(room.as_str()).is_ok()
        && anchor.is_none_or(|id| valid_id(id.as_str()).is_ok())
        && !content.is_empty()
        && content.len() <= 7439
        && !content.contains('\0')
}

fn valid_success(intent: &FrontendIntent, result: &FrontendResult) -> bool {
    match (intent, result) {
        (
            FrontendIntent::SendMessage {
                room_id, parent_id, ..
            },
            FrontendResult::SendMessage(WriteOutcome::Success(message)),
        ) => {
            valid_id(message.id.as_str()).is_ok()
                && message.room_id == *room_id
                && message.parent_id == *parent_id
        }
        (
            FrontendIntent::EditMessage {
                room_id,
                message_id,
                ..
            },
            FrontendResult::EditMessage(WriteOutcome::Success(message)),
        ) => {
            valid_id(message.id.as_str()).is_ok()
                && message.room_id == *room_id
                && message.id == *message_id
        }
        _ => true, // RequestEnvelope::complete separately checks exact operations.
    }
}

#[cfg(test)]
mod tests;
