//! Dormant single-write mailbox. No App, runtime loop, socket or startup wiring.
//!
//! The frontend endpoint only submits/drains synchronously. The worker alone owns
//! HostController. One slot spans queued, reserved/in-flight and undrained terminal
//! work; reservation happens before a gate future is polled. No retry/replay path.
//!
//! A future runtime MUST retain/pin handle_one across ordinary UI/lifecycle select
//! branches, rather than recreating it each tick. Its mutable controller borrow
//! serializes lifecycle work (SSE, lease checks and renewal cannot run during a
//! write), so every attempted write has a hard deadline (`WRITE_DEADLINE`, well
//! below the 600 s UI lease and its 80% renewal point). On timeout the gate
//! future is dropped and the slot holds Uncertain/AttemptInterrupted, never a
//! retry. Expect SSE lag → resync after a long write. Only this deadline or an
//! explicit owner interruption may drop an attempted future.
//! Known terminal results are stored synchronously before returning and survive
//! backpressure, UI endpoint drop and shutdown. Volatile process death is not
//! recoverable, and a new mailbox cannot recover or replay an old command.
//! All payload containers are non-Debug/non-Display and never logged/serialized.

use super::{
    controller::{HostController, PresentationHost},
    session::{
        FrameAction, LeaseAction, SessionBackend, SessionClock, SessionFailure, SessionLifecycle,
        SessionPhase, SessionWriteError, SnapshotTicket, SubscriptionId, WriteAdmissionStamp,
    },
    write_dispatch::DispatchError,
};
use crate::frontend::{
    editor_submission::{NotDispatchedReason, QueueAdmission, SubmissionTerminal, UncertainReason},
    envelopes::{FrontendResult, RequestContext, RequestEnvelope},
    lifecycle::{LifecycleProjection, ProjectionEffects},
    write_rules::{valid_editor_write, write_success_matches},
};
use std::{
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

/// Hard per-write deadline. The UI lease is 600 s with renewal at 80%, and
/// the UDS client has no deadline of its own, so a stuck write must not keep
/// the controller (and therefore lease renewal and SSE) borrowed for longer.
pub(crate) const WRITE_DEADLINE: Duration = Duration::from_secs(30);
use webex_bridge::client::events::BridgeEvent;

/// Conclusive pre-ownership decisions only. This is the ledger's own report
/// type, so every mailbox refusal maps directly onto `EditorSubmission::admit`
/// and releases the prepared slot. A lost worker/result after Accepted must be
/// reduced as missing completion, NEVER reported as a queue refusal.
pub(crate) type CommandAdmission = QueueAdmission;

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
    /// `Accepted` only means this mailbox now owns the copy. It is not a
    /// readiness promise: a lease that expires silently before dequeue still
    /// yields `NotDispatched { NotReady | Expired }` with zero write I/O.
    pub(crate) fn try_submit(&self, request: RequestEnvelope) -> CommandAdmission {
        self.try_submit_at(request, Instant::now())
    }

    /// Same as `try_submit` with an explicit clock reading, for fake clocks.
    /// A published stamp past its lease deadline is refused as NotReady.
    pub(crate) fn try_submit_at(&self, request: RequestEnvelope, now: Instant) -> CommandAdmission {
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
        if !mailbox.published.admissible_at(now) {
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

/// Host wrapper owned inside the controller. Every non-Active phase publication,
/// including the one HostController makes before the first await of connect or
/// renewal, synchronously revokes mailbox admission, so a dropped lifecycle
/// future cannot leave a stale ready hint behind. Active is restored only by the
/// worker after a completed controller call, using the actual stamp.
struct AdmissionHost<H> {
    inner: H,
    shared: Shared,
}

impl<H: PresentationHost> PresentationHost for AdmissionHost<H> {
    type Snapshot = H::Snapshot;
    fn session_phase(&mut self, phase: SessionPhase) {
        if phase != SessionPhase::Active {
            let mut mailbox = lock(&self.shared);
            mailbox.published = mailbox.published.revoked();
        }
        self.inner.session_phase(phase);
    }
    fn lifecycle(&mut self, projection: &LifecycleProjection, effects: ProjectionEffects) {
        self.inner.lifecycle(projection, effects);
    }
    fn install_snapshot(&mut self, snapshot: Self::Snapshot) {
        self.inner.install_snapshot(snapshot);
    }
}

/// Sole owner of HostController. Lifecycle methods are explicit, caller-
/// scheduled forwards (no task, timer or reconnect policy) and republish the
/// actual admission stamp after every completed controller call. The published
/// stamp stays a hint: dequeue still checks the actual lease-aware state.
pub(crate) struct CommandWorker<B: SessionBackend, C: SessionClock, H: PresentationHost> {
    controller: HostController<B, C, AdmissionHost<H>>,
    shared: Shared,
    write_deadline: Duration,
}

impl<B: SessionBackend, C: SessionClock, H: PresentationHost> CommandWorker<B, C, H> {
    pub(crate) fn new(session: SessionLifecycle<B, C>, host: H) -> (CommandMailbox, Self) {
        let published = session.write_admission_stamp();
        let shared = Arc::new(Mutex::new(Mailbox {
            open: true,
            published,
            slot: Slot::Empty,
        }));
        let host = AdmissionHost {
            inner: host,
            shared: Arc::clone(&shared),
        };
        (
            CommandMailbox {
                shared: Arc::clone(&shared),
            },
            Self {
                controller: HostController::new(session, host),
                shared,
                write_deadline: WRITE_DEADLINE,
            },
        )
    }

    /// Shorter deadline for deterministic fake tests; never longer than the
    /// production bound.
    #[cfg(test)]
    pub(crate) fn set_write_deadline(&mut self, deadline: Duration) {
        self.write_deadline = deadline.min(WRITE_DEADLINE);
    }

    /// Worker-side publication after lifecycle/snapshot work. This is only a
    /// pre-admission hint; actual readiness/generation is checked again at dequeue.
    fn publish(&mut self) {
        lock(&self.shared).published = self.controller.write_admission_stamp();
    }

    pub(crate) fn phase(&self) -> SessionPhase {
        self.controller.phase()
    }
    pub(crate) fn subscription(&self) -> Option<SubscriptionId> {
        self.controller.subscription()
    }
    pub(crate) fn projection(&self) -> &LifecycleProjection {
        self.controller.projection()
    }
    pub(crate) fn host(&self) -> &H {
        &self.controller.host().inner
    }

    /// The caller borrows its own paired secret for this call only.
    pub(crate) async fn connect(
        &mut self,
        paired_secret: &str,
    ) -> Result<SubscriptionId, SessionFailure> {
        let result = self.controller.connect(paired_secret).await;
        self.publish();
        result
    }

    pub(crate) async fn renew_if_due(
        &mut self,
        paired_secret: &str,
    ) -> Result<bool, SessionFailure> {
        let result = self.controller.renew_if_due(paired_secret).await;
        self.publish();
        result
    }

    pub(crate) fn check_lease(&mut self) -> LeaseAction {
        let action = self.controller.check_lease();
        self.publish();
        action
    }

    pub(crate) fn disconnect(&mut self) {
        self.controller.disconnect();
        self.publish();
    }

    pub(crate) fn bridge_event(
        &mut self,
        subscription: SubscriptionId,
        event: &BridgeEvent,
    ) -> Result<FrameAction, SessionFailure> {
        let result = self.controller.bridge_event(subscription, event);
        self.publish();
        result
    }

    pub(crate) fn frame(
        &mut self,
        subscription: SubscriptionId,
        frame: crate::frontend::continuity::UiLifecycleFrame,
    ) -> Result<FrameAction, SessionFailure> {
        let result = self.controller.frame(subscription, frame);
        self.publish();
        result
    }

    pub(crate) fn stream_failed(
        &mut self,
        subscription: SubscriptionId,
        failure: SessionFailure,
    ) -> FrameAction {
        let action = self.controller.stream_failed(subscription, failure);
        self.publish();
        action
    }

    pub(crate) async fn begin_snapshot(&mut self) -> Result<SnapshotTicket, SessionFailure> {
        let result = self.controller.begin_snapshot().await;
        self.publish();
        result
    }

    /// Read-only, ticket-fenced facade; no write-capable client crosses here.
    pub(crate) fn snapshot_reader(
        &self,
        ticket: &SnapshotTicket,
    ) -> Result<B::SnapshotReader<'_>, SessionFailure> {
        self.controller.snapshot_reader(ticket)
    }

    pub(crate) fn commit_snapshot(
        &mut self,
        ticket: SnapshotTicket,
        snapshot: H::Snapshot,
    ) -> Result<usize, SessionFailure> {
        let result = self.controller.commit_snapshot(ticket, snapshot);
        self.publish();
        result
    }

    /// Lease-checked stream access. Publication precedes the returned borrow.
    pub(crate) fn stream_mut(&mut self) -> Option<&mut B::Stream> {
        self.check_lease();
        self.controller.stream_mut()
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
            self.publish();
            return true;
        }
        let context = command.request.context.clone();
        let terminal = match tokio::time::timeout(
            self.write_deadline,
            dispatch_envelope(&mut self.controller, command.request, &mut reservation),
        )
        .await
        {
            Ok(terminal) => terminal,
            // The gate future has been dropped after its attempt boundary: the
            // daemon may or may not have sent it. Never Unavailable, never retry.
            Err(_) => SubmissionTerminal::Uncertain {
                context,
                reason: UncertainReason::AttemptInterrupted,
            },
        };
        reservation.finish(terminal);
        self.publish();
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
            if !write_success_matches(&request.intent, &result) {
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
    request.context.operation == request.intent.operation()
        && request.context.room_view.as_ref().is_none_or(|view| {
            request.intent.room_id().map(|id| id.as_str()) == Some(view.room_id.as_str())
        })
        // Shared with the ledger: no reads/delete/leave or scope expansion.
        && valid_editor_write(&request.intent)
}

#[cfg(test)]
mod tests;
