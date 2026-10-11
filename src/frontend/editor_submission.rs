//! Dormant, pure submitted-write ledger; no App, queue, gate or I/O ownership.
//!
//! Preparation reserves one immutable request. The caller reports its synchronous
//! queue admission before processing terminal events. Losing that report is not
//! proof of non-dispatch: missing completion is conservatively delivery-unknown.
//! Current composition stays caller-owned; revisions must advance on every body,
//! format or target change. Focus changes do not invalidate a submitted write.
//! No body-bearing type implements Debug/Display or performs logging.

#[cfg(test)]
use super::write_rules::{
    MAX_OPAQUE_ID_BYTES as MAX_ID_BYTES, MAX_WRITE_BODY_BYTES as MAX_BODY_BYTES,
};
use super::{
    data::{ComposedBody, MessageId, MessageSnapshot, RoomId},
    editor_write::{EditorWriteState, EditorWriteStatus},
    envelopes::{
        FrontendError, FrontendIntent, FrontendResult, RequestContext, RequestEnvelope,
        ResultEnvelope, WriteOutcome,
    },
    write_rules::{valid_opaque_id as valid_id, valid_write_body, write_success_matches},
};

#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
pub(crate) struct DraftRevision(u64);

impl DraftRevision {
    pub(crate) fn next(self) -> Result<Self, SubmissionError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(SubmissionError::CounterExhausted)
    }
}

/// Anchors carry their own room; a focused room is never used to retarget them.
#[derive(Clone, Eq, PartialEq)]
pub(crate) enum DraftTarget {
    Send {
        room_id: RoomId,
    },
    Reply {
        room_id: RoomId,
        parent_id: MessageId,
    },
    Edit {
        room_id: RoomId,
        message_id: MessageId,
    },
}

impl DraftTarget {
    fn valid(&self) -> bool {
        match self {
            Self::Send { room_id } => valid_id(room_id.as_str()),
            Self::Reply { room_id, parent_id } => {
                valid_id(room_id.as_str()) && valid_id(parent_id.as_str())
            }
            Self::Edit {
                room_id,
                message_id,
            } => valid_id(room_id.as_str()) && valid_id(message_id.as_str()),
        }
    }
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct DraftStamp {
    pub(crate) target: DraftTarget,
    pub(crate) revision: DraftRevision,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct DraftSnapshot {
    pub(crate) stamp: DraftStamp,
    pub(crate) body: ComposedBody,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SubmissionError {
    Busy,
    ReviewNeeded,
    InvalidBody,
    InvalidTarget,
    CounterExhausted,
    QueueFull,
    QueueClosed,
    QueueNotReady,
    QueueInvalidRequest,
    AdmissionMismatch,
    AlreadyAdmitted,
}

/// A pure report of the caller's actual queue decision, not a queue capability.
/// The single-write mailbox returns this exact type, so every conclusive refusal
/// it can make has a ledger mapping. Only Accepted transfers ownership; every
/// other variant is a pre-ownership refusal that proves zero write I/O.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QueueAdmission {
    Accepted,
    Full,
    Closed,
    /// Worker-published session/snapshot admission is not ready.
    NotReady,
    /// Shared write rules refused the owned copy before queue ownership.
    InvalidRequest,
}

/// Only an adapter that proves zero write I/O may report these reasons.
/// Timeout, cancellation after an attempt, and a closed result pipe are NOT proof.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NotDispatchedReason {
    NotReady,
    Expired,
    SessionFailed,
    WrongIntent,
    UnstartedShutdown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UncertainReason {
    MissingCompletion,
    AttemptInterrupted,
    CompletionMismatch,
}

pub(crate) enum SubmissionTerminal {
    Result(Box<ResultEnvelope>),
    NotDispatched {
        context: RequestContext,
        reason: NotDispatchedReason,
    },
    Uncertain {
        context: RequestContext,
        reason: UncertainReason,
    },
}

impl SubmissionTerminal {
    pub(crate) fn result(envelope: ResultEnvelope) -> Self {
        Self::Result(Box::new(envelope))
    }

    fn context(&self) -> &RequestContext {
        match self {
            Self::Result(result) => &result.context,
            Self::NotDispatched { context, .. } | Self::Uncertain { context, .. } => context,
        }
    }
}

struct PendingSubmission {
    request: RequestEnvelope,
    draft: DraftSnapshot,
    admitted: bool,
}

/// One retained failed submitted version, distinct from newly typed composition.
pub(crate) struct ReviewSnapshot {
    pub(crate) context: RequestContext,
    pub(crate) draft: DraftSnapshot,
    pub(crate) status: EditorWriteStatus,
}

/// Pure cache/editor hints. These do not install a message, select a room, clear
/// a textarea or dispatch reconciliation reads. Local cache failure cannot change
/// an already confirmed write into rejection/unavailability or cause a retry.
#[derive(Default)]
pub(crate) struct SubmissionEffects {
    pub(crate) settled: Option<RequestContext>,
    pub(crate) status: Option<EditorWriteStatus>,
    pub(crate) canonical: Option<MessageSnapshot>,
    /// Apply only to this exact current draft stamp; never to newer composition.
    pub(crate) clear_current: Option<DraftStamp>,
}

pub(crate) struct EditorSubmission {
    state: EditorWriteState,
    pending: Option<PendingSubmission>,
    review: Option<ReviewSnapshot>,
}

impl Default for EditorSubmission {
    fn default() -> Self {
        Self {
            state: EditorWriteState::new(ComposedBody::PlainText(String::new())),
            pending: None,
            review: None,
        }
    }
}

impl EditorSubmission {
    pub(crate) fn status(&self) -> EditorWriteStatus {
        self.state.status()
    }

    pub(crate) fn pending_context(&self) -> Option<&RequestContext> {
        self.pending
            .as_ref()
            .map(|pending| &pending.request.context)
    }

    pub(crate) fn review(&self) -> Option<&ReviewSnapshot> {
        self.review.as_ref()
    }

    /// Explicit local review/dismiss acknowledgement. It never returns a command
    /// or modifies current composition. A fresh write requires fresh preparation.
    pub(crate) fn dismiss_review(&mut self) -> bool {
        self.review.take().is_some()
    }

    /// Reserve a single slot and return an owned private correlation copy for
    /// queue submission. Busy/review/invalid failures do not allocate or mutate.
    /// A successfully allocated ID is never reused, even on queue refusal.
    pub(crate) fn prepare(
        &mut self,
        draft: &DraftSnapshot,
    ) -> Result<RequestEnvelope, SubmissionError> {
        if self.pending.is_some() {
            return Err(SubmissionError::Busy);
        }
        if self.review.is_some() {
            return Err(SubmissionError::ReviewNeeded);
        }
        if !valid_write_body(&draft.body) {
            return Err(SubmissionError::InvalidBody);
        }
        if !draft.stamp.target.valid() {
            return Err(SubmissionError::InvalidTarget);
        }
        self.state.replace_body(draft.body.clone());
        let request = match &draft.stamp.target {
            DraftTarget::Send { room_id } => self.state.send(room_id.clone(), None),
            DraftTarget::Reply { room_id, parent_id } => {
                self.state.send(room_id.clone(), Some(parent_id.clone()))
            }
            DraftTarget::Edit {
                room_id,
                message_id,
            } => self.state.edit(room_id.clone(), message_id.clone()),
        }
        .map_err(|_| SubmissionError::CounterExhausted)?;
        let command = RequestEnvelope {
            context: request.context.clone(),
            intent: request.intent.clone(),
        };
        self.pending = Some(PendingSubmission {
            request,
            draft: draft.clone(),
            admitted: false,
        });
        Ok(command)
    }

    /// Report synchronous try_send success/refusal using the exact prepared
    /// context. Refusal after acceptance is forbidden: it cannot prove non-I/O.
    /// Missing admission reports must be resolved through missing_completion.
    pub(crate) fn admit(
        &mut self,
        context: &RequestContext,
        admission: QueueAdmission,
    ) -> Result<(), SubmissionError> {
        let pending = self
            .pending
            .as_mut()
            .ok_or(SubmissionError::AdmissionMismatch)?;
        if pending.request.context != *context {
            return Err(SubmissionError::AdmissionMismatch);
        }
        if pending.admitted {
            return Err(SubmissionError::AlreadyAdmitted);
        }
        match admission {
            QueueAdmission::Accepted => {
                pending.admitted = true;
                Ok(())
            }
            refusal => {
                let pending = self.pending.take().expect("matched pending submission");
                self.state.unavailable(&pending.request);
                // The caller still owns the unchanged draft, so no review slot
                // is needed for a queue that conclusively refused ownership.
                Err(match refusal {
                    QueueAdmission::Full => SubmissionError::QueueFull,
                    QueueAdmission::Closed => SubmissionError::QueueClosed,
                    QueueAdmission::NotReady => SubmissionError::QueueNotReady,
                    QueueAdmission::InvalidRequest => SubmissionError::QueueInvalidRequest,
                    QueueAdmission::Accepted => unreachable!("handled above"),
                })
            }
        }
    }

    /// Settle globally before any room-focus policy. Different full contexts
    /// (including genuine reads) are inert. An exact write context with a wrong
    /// operation/resource payload is an uncertain completion, not fake success.
    pub(crate) fn reduce(
        &mut self,
        terminal: SubmissionTerminal,
        current: Option<&DraftStamp>,
    ) -> SubmissionEffects {
        let Some(pending) = &self.pending else {
            return SubmissionEffects::default();
        };
        if terminal.context() != &pending.request.context {
            return SubmissionEffects::default();
        }
        let pending = self.pending.take().expect("matched pending submission");
        let context = pending.request.context.clone();
        let result = match terminal {
            SubmissionTerminal::NotDispatched { reason, .. } => {
                let _ = reason; // metadata-only proof; never used as retry policy
                self.state.unavailable(&pending.request);
                return self.finish(pending, EditorWriteStatus::Unavailable, None, current);
            }
            SubmissionTerminal::Uncertain { reason, .. } => {
                let _ = reason;
                uncertain_result(&pending.request.intent)
            }
            SubmissionTerminal::Result(envelope) => {
                if envelope.result.operation() != pending.request.intent.operation()
                    || !write_success_matches(&pending.request.intent, &envelope.result)
                {
                    uncertain_result(&pending.request.intent)
                } else {
                    envelope.result
                }
            }
        };
        // Validate retained context/intent/result operations, not just request ID.
        let envelope = RequestEnvelope {
            context,
            intent: pending.request.intent.clone(),
        }
        .complete(result)
        .expect("private request and checked completion operations agree");
        self.state.accept(&envelope);
        let canonical = match envelope.result {
            FrontendResult::SendMessage(WriteOutcome::Success(message))
            | FrontendResult::EditMessage(WriteOutcome::Success(message)) => Some(message),
            _ => None,
        };
        self.finish(pending, self.state.status(), canonical, current)
    }

    /// Caller-observed worker/result loss is never evidence that a queued write
    /// did not execute, even when its admission report was lost. The exact
    /// context fences delayed loss notifications from a previous submission.
    /// No Drop hook, timer or reconnect can turn it into Unavailable or replay it.
    pub(crate) fn missing_completion(
        &mut self,
        context: &RequestContext,
        current: Option<&DraftStamp>,
    ) -> SubmissionEffects {
        self.reduce(
            SubmissionTerminal::Uncertain {
                context: context.clone(),
                reason: UncertainReason::MissingCompletion,
            },
            current,
        )
    }

    fn finish(
        &mut self,
        pending: PendingSubmission,
        status: EditorWriteStatus,
        canonical: Option<MessageSnapshot>,
        current: Option<&DraftStamp>,
    ) -> SubmissionEffects {
        let clear_current =
            if status == EditorWriteStatus::Ready && current == Some(&pending.draft.stamp) {
                Some(pending.draft.stamp.clone())
            } else {
                None
            };
        let context = pending.request.context;
        if status != EditorWriteStatus::Ready {
            self.review = Some(ReviewSnapshot {
                context: context.clone(),
                draft: pending.draft,
                status,
            });
        }
        SubmissionEffects {
            settled: Some(context),
            status: Some(status),
            canonical,
            clear_current,
        }
    }
}

fn uncertain_result(intent: &FrontendIntent) -> FrontendResult {
    let outcome = WriteOutcome::DeliveryUnknown(FrontendError {
        code: "completion_uncertain".into(),
        message: "Write completion is uncertain; review before a fresh submission".into(),
    });
    match intent {
        FrontendIntent::SendMessage { .. } => FrontendResult::SendMessage(outcome),
        FrontendIntent::EditMessage { .. } => FrontendResult::EditMessage(outcome),
        _ => unreachable!("only editor write intents can be prepared"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::{
        data::{MessageStatus, PersonSnapshot},
        envelopes::{Operation, ReadOutcome, RequestSequence},
        requests::{RoomRequestContext, RoomViewRequests},
    };

    fn targets() -> [DraftTarget; 3] {
        [
            DraftTarget::Send {
                room_id: RoomId::new("room/a?opaque"),
            },
            DraftTarget::Reply {
                room_id: RoomId::new("room/a?opaque"),
                parent_id: MessageId::new("parent/a?opaque"),
            },
            DraftTarget::Edit {
                room_id: RoomId::new("room/a?opaque"),
                message_id: MessageId::new("message/a?opaque"),
            },
        ]
    }

    fn draft(target: DraftTarget, body: ComposedBody) -> DraftSnapshot {
        DraftSnapshot {
            stamp: DraftStamp {
                target,
                revision: DraftRevision::default(),
            },
            body,
        }
    }

    fn plain_draft() -> DraftSnapshot {
        draft(
            targets()[0].clone(),
            ComposedBody::PlainText("private body".into()),
        )
    }

    fn accepted(ledger: &mut EditorSubmission, draft: &DraftSnapshot) -> RequestEnvelope {
        let request = ledger.prepare(draft).unwrap();
        ledger
            .admit(&request.context, QueueAdmission::Accepted)
            .unwrap();
        request
    }

    fn message(target: &DraftTarget) -> MessageSnapshot {
        let (room_id, id, parent_id) = match target {
            DraftTarget::Send { room_id } => (room_id.clone(), MessageId::new("new"), None),
            DraftTarget::Reply { room_id, parent_id } => (
                room_id.clone(),
                MessageId::new("new"),
                Some(parent_id.clone()),
            ),
            DraftTarget::Edit {
                room_id,
                message_id,
            } => (room_id.clone(), message_id.clone(), None),
        };
        MessageSnapshot {
            id,
            room_id,
            parent_id,
            revision: Some(3),
            sender_id: None,
            sender_email: None,
            created_at: None,
            updated_at: None,
            status: MessageStatus::Active,
            body: None,
            attachment_count: None,
        }
    }

    fn result(target: &DraftTarget, outcome: WriteOutcome<MessageSnapshot>) -> FrontendResult {
        match target {
            DraftTarget::Send { .. } | DraftTarget::Reply { .. } => {
                FrontendResult::SendMessage(outcome)
            }
            DraftTarget::Edit { .. } => FrontendResult::EditMessage(outcome),
        }
    }

    fn error() -> FrontendError {
        FrontendError {
            code: "safe".into(),
            message: "safe".into(),
        }
    }

    fn lose_pending(
        ledger: &mut EditorSubmission,
        current: Option<&DraftStamp>,
    ) -> SubmissionEffects {
        let Some(context) = ledger.pending_context().cloned() else {
            return SubmissionEffects::default();
        };
        ledger.missing_completion(&context, current)
    }

    fn assert_inert(effects: &SubmissionEffects) {
        assert!(effects.settled.is_none());
        assert!(effects.status.is_none());
        assert!(effects.canonical.is_none());
        assert!(effects.clear_current.is_none());
    }

    #[test]
    fn exact_send_reply_edit_anchors_and_formats_are_forwarded_privately() {
        for target in targets() {
            for body in [
                ComposedBody::PlainText("**literal**".into()),
                ComposedBody::Markdown("**markdown**".into()),
            ] {
                let draft = draft(target.clone(), body.clone());
                let mut ledger = EditorSubmission::default();
                let request = accepted(&mut ledger, &draft);
                let expected = match &target {
                    DraftTarget::Send { room_id } => FrontendIntent::SendMessage {
                        room_id: room_id.clone(),
                        parent_id: None,
                        body,
                    },
                    DraftTarget::Reply { room_id, parent_id } => FrontendIntent::SendMessage {
                        room_id: room_id.clone(),
                        parent_id: Some(parent_id.clone()),
                        body,
                    },
                    DraftTarget::Edit {
                        room_id,
                        message_id,
                    } => FrontendIntent::EditMessage {
                        room_id: room_id.clone(),
                        message_id: message_id.clone(),
                        body,
                    },
                };
                assert!(request.intent == expected);
                assert!(ledger.pending.as_ref().unwrap().draft == draft);
                assert_eq!(request.context.operation, expected.operation());
                assert_eq!(request.context.room_view, None);
                assert_eq!(ledger.status(), EditorWriteStatus::Pending);
            }
        }
    }

    #[test]
    fn outcomes_settle_and_clear_only_success_without_retry() {
        for target in targets() {
            for outcome_kind in 0..3 {
                let draft = draft(target.clone(), ComposedBody::Markdown("**private**".into()));
                let mut ledger = EditorSubmission::default();
                let request = accepted(&mut ledger, &draft);
                let context = request.context.clone();
                let canonical = message(&target);
                let (outcome, status) = match outcome_kind {
                    0 => (
                        WriteOutcome::Success(canonical.clone()),
                        EditorWriteStatus::Ready,
                    ),
                    1 => (WriteOutcome::Rejected(error()), EditorWriteStatus::Rejected),
                    _ => (
                        WriteOutcome::DeliveryUnknown(error()),
                        EditorWriteStatus::DeliveryUnknown,
                    ),
                };
                let envelope = request.complete(result(&target, outcome)).unwrap();
                let effects =
                    ledger.reduce(SubmissionTerminal::result(envelope), Some(&draft.stamp));
                assert_eq!(effects.settled, Some(context.clone()));
                assert_eq!(effects.status, Some(status));
                assert_eq!(ledger.status(), status);
                assert!(ledger.pending_context().is_none());
                assert!(ledger.state.body() == &draft.body);
                if outcome_kind == 0 {
                    assert!(effects.canonical.as_ref() == Some(&canonical));
                    assert!(effects.clear_current.as_ref() == Some(&draft.stamp));
                    assert!(ledger.review().is_none());
                } else {
                    assert!(effects.canonical.is_none());
                    assert!(effects.clear_current.is_none());
                    let review = ledger.review().unwrap();
                    assert_eq!(review.context, context);
                    assert_eq!(review.status, status);
                    assert!(review.draft == draft);
                    assert!(matches!(
                        ledger.prepare(&draft),
                        Err(SubmissionError::ReviewNeeded)
                    ));
                }
            }
        }
    }

    #[test]
    fn busy_and_review_gate_explicit_fresh_submission_with_persistent_ids() {
        let draft = plain_draft();
        let mut ledger = EditorSubmission::default();
        let first = accepted(&mut ledger, &draft);
        let first_context = first.context.clone();
        let mut newer = draft.clone();
        newer.stamp.revision = newer.stamp.revision.next().unwrap();
        newer.body = ComposedBody::PlainText("new text".into());
        assert!(matches!(ledger.prepare(&newer), Err(SubmissionError::Busy)));
        assert_eq!(ledger.pending_context(), Some(&first_context));
        assert!(!ledger.dismiss_review());
        lose_pending(&mut ledger, Some(&newer.stamp));
        assert!(matches!(
            ledger.prepare(&newer),
            Err(SubmissionError::ReviewNeeded)
        ));
        assert!(ledger.review().unwrap().draft == draft);
        assert!(ledger.dismiss_review());
        assert!(!ledger.dismiss_review());
        assert!(ledger.pending_context().is_none());
        let second = accepted(&mut ledger, &newer);
        assert_ne!(second.context.id, first_context.id);
        assert!(ledger.pending.as_ref().unwrap().draft == newer);
        assert_inert(
            &ledger.reduce(
                SubmissionTerminal::result(
                    first
                        .complete(result(
                            &draft.stamp.target,
                            WriteOutcome::Success(message(&draft.stamp.target)),
                        ))
                        .unwrap(),
                ),
                Some(&newer.stamp),
            ),
        );
        assert_eq!(ledger.pending_context(), Some(&second.context));
    }

    #[test]
    fn queue_refusal_preserves_draft_and_consumes_allocated_ids() {
        let draft = plain_draft();
        let original = draft.clone();
        let mut ledger = EditorSubmission::default();
        let mut previous = None;
        for (admission, error) in [
            (QueueAdmission::Full, SubmissionError::QueueFull),
            (QueueAdmission::Closed, SubmissionError::QueueClosed),
            (QueueAdmission::NotReady, SubmissionError::QueueNotReady),
            (
                QueueAdmission::InvalidRequest,
                SubmissionError::QueueInvalidRequest,
            ),
        ] {
            let request = ledger.prepare(&draft).unwrap();
            if let Some(previous) = previous {
                assert_ne!(request.context.id, previous);
            }
            previous = Some(request.context.id);
            assert_eq!(ledger.admit(&request.context, admission), Err(error));
            assert_eq!(ledger.status(), EditorWriteStatus::Unavailable);
            assert!(ledger.pending_context().is_none());
            assert!(ledger.review().is_none());
            assert!(draft == original);
            assert!(ledger.state.body() == &draft.body);
            assert_inert(&lose_pending(&mut ledger, Some(&draft.stamp)));
        }
        let request = accepted(&mut ledger, &draft);
        assert_ne!(Some(request.context.id), previous);
        assert_eq!(
            ledger.admit(&request.context, QueueAdmission::Full),
            Err(SubmissionError::AlreadyAdmitted)
        );
        assert_eq!(ledger.status(), EditorWriteStatus::Pending);
    }

    #[test]
    fn invalid_body_and_target_fail_before_allocation_and_preserve_draft() {
        for body in [
            ComposedBody::PlainText(String::new()),
            ComposedBody::Markdown("x".repeat(MAX_BODY_BYTES + 1)),
            ComposedBody::PlainText("before\0after".into()),
            ComposedBody::Markdown("é".repeat(3720)),
        ] {
            let draft = draft(targets()[0].clone(), body);
            let original = draft.clone();
            let mut ledger = EditorSubmission::default();
            assert!(matches!(
                ledger.prepare(&draft),
                Err(SubmissionError::InvalidBody)
            ));
            assert_eq!(ledger.status(), EditorWriteStatus::Ready);
            assert!(ledger.pending_context().is_none());
            assert!(draft == original);
        }
        for id in [
            String::new(),
            "x".repeat(MAX_ID_BYTES + 1),
            "id\ncontrol".into(),
        ] {
            for target in [
                DraftTarget::Send {
                    room_id: RoomId::new(&id),
                },
                DraftTarget::Reply {
                    room_id: RoomId::new("room"),
                    parent_id: MessageId::new(&id),
                },
                DraftTarget::Edit {
                    room_id: RoomId::new("room"),
                    message_id: MessageId::new(&id),
                },
            ] {
                let draft = draft(target, ComposedBody::PlainText("private".into()));
                let mut ledger = EditorSubmission::default();
                assert!(matches!(
                    ledger.prepare(&draft),
                    Err(SubmissionError::InvalidTarget)
                ));
                assert!(ledger.pending_context().is_none());
            }
        }
        let mut ledger = EditorSubmission::default();
        let invalid = draft(targets()[0].clone(), ComposedBody::PlainText(String::new()));
        assert!(ledger.prepare(&invalid).is_err());
        let request = ledger.prepare(&plain_draft()).unwrap();
        let fresh = EditorSubmission::default().prepare(&plain_draft()).unwrap();
        assert_eq!(request.context, fresh.context);
    }

    #[test]
    fn byte_and_opaque_id_boundaries_are_accepted_without_normalization() {
        for body in [
            ComposedBody::PlainText("x".repeat(MAX_BODY_BYTES)),
            ComposedBody::Markdown(format!("{}x", "é".repeat(3719))),
        ] {
            let draft = draft(
                DraftTarget::Send {
                    room_id: RoomId::new("x".repeat(MAX_ID_BYTES)),
                },
                body,
            );
            let mut ledger = EditorSubmission::default();
            let request = accepted(&mut ledger, &draft);
            assert!(
                matches!(request.intent, FrontendIntent::SendMessage { body, .. } if body == draft.body)
            );
        }
    }

    #[test]
    fn every_proven_non_dispatch_retains_review_and_never_clears_new_text() {
        for reason in [
            NotDispatchedReason::NotReady,
            NotDispatchedReason::Expired,
            NotDispatchedReason::SessionFailed,
            NotDispatchedReason::WrongIntent,
            NotDispatchedReason::UnstartedShutdown,
        ] {
            let draft = plain_draft();
            let mut ledger = EditorSubmission::default();
            let request = accepted(&mut ledger, &draft);
            let effects = ledger.reduce(
                SubmissionTerminal::NotDispatched {
                    context: request.context.clone(),
                    reason,
                },
                Some(&draft.stamp),
            );
            assert_eq!(effects.status, Some(EditorWriteStatus::Unavailable));
            assert_eq!(effects.settled, Some(request.context));
            assert!(effects.clear_current.is_none());
            assert!(effects.canonical.is_none());
            assert!(ledger.review().unwrap().draft == draft);
            assert!(matches!(
                ledger.prepare(&draft),
                Err(SubmissionError::ReviewNeeded)
            ));
        }
    }

    #[test]
    fn missing_interrupted_and_mismatched_attempts_are_unknown_not_unavailable() {
        for target in targets() {
            for reason in [
                UncertainReason::MissingCompletion,
                UncertainReason::AttemptInterrupted,
                UncertainReason::CompletionMismatch,
            ] {
                let draft = draft(target.clone(), ComposedBody::PlainText("private".into()));
                let mut ledger = EditorSubmission::default();
                let request = accepted(&mut ledger, &draft);
                let effects = ledger.reduce(
                    SubmissionTerminal::Uncertain {
                        context: request.context.clone(),
                        reason,
                    },
                    Some(&draft.stamp),
                );
                assert_eq!(effects.status, Some(EditorWriteStatus::DeliveryUnknown));
                assert_eq!(ledger.status(), EditorWriteStatus::DeliveryUnknown);
                assert!(effects.canonical.is_none());
                assert!(effects.clear_current.is_none());
                assert!(ledger.review().unwrap().draft == draft);
                assert!(matches!(
                    ledger.prepare(&draft),
                    Err(SubmissionError::ReviewNeeded)
                ));
                assert_inert(&lose_pending(&mut ledger, Some(&draft.stamp)));
            }
        }
        // Even a lost prepare/admission report cannot prove the queue refused it.
        let mut ledger = EditorSubmission::default();
        ledger.prepare(&plain_draft()).unwrap();
        let effects = lose_pending(&mut ledger, None);
        assert_eq!(effects.status, Some(EditorWriteStatus::DeliveryUnknown));
    }

    #[test]
    fn new_revision_target_cancel_and_focus_do_not_lose_completion() {
        let draft = plain_draft();
        for change in 0..4 {
            let mut ledger = EditorSubmission::default();
            let request = accepted(&mut ledger, &draft);
            let mut current = draft.stamp.clone();
            if change == 0 {
                current.revision = current.revision.next().unwrap();
            }
            if change == 1 {
                current.target = targets()[1].clone();
            }
            let current = if change == 2 { None } else { Some(&current) };
            let mut focus = RoomViewRequests::default();
            focus.select_room(Some("room/a?opaque".into())).unwrap();
            focus.start_read().unwrap();
            focus.select_room(Some("room-b".into())).unwrap();
            let room_b = focus.start_read().unwrap().unwrap();
            let canonical = message(&draft.stamp.target);
            let envelope = request
                .complete(result(
                    &draft.stamp.target,
                    WriteOutcome::Success(canonical.clone()),
                ))
                .unwrap();
            let effects = ledger.reduce(SubmissionTerminal::result(envelope), current);
            assert!(effects.canonical.as_ref() == Some(&canonical));
            assert_eq!(effects.status, Some(EditorWriteStatus::Ready));
            assert_eq!(effects.clear_current.is_some(), change == 3);
            assert!(focus.is_current(&room_b));
            assert!(ledger.pending_context().is_none());
        }
    }

    #[test]
    fn full_context_fences_admission_and_all_terminal_kinds() {
        let draft = plain_draft();
        let mut ledger = EditorSubmission::default();
        let request = ledger.prepare(&draft).unwrap();
        let mut wrong = request.context.clone();
        wrong.room_view = Some(RoomRequestContext {
            room_id: "room/a?opaque".into(),
            view_generation: 2,
            request_id: 1,
        });
        assert_eq!(
            ledger.admit(&wrong, QueueAdmission::Full),
            Err(SubmissionError::AdmissionMismatch)
        );
        ledger
            .admit(&request.context, QueueAdmission::Accepted)
            .unwrap();
        for terminal in [
            SubmissionTerminal::result(ResultEnvelope {
                context: wrong.clone(),
                result: result(
                    &draft.stamp.target,
                    WriteOutcome::Success(message(&draft.stamp.target)),
                ),
            }),
            SubmissionTerminal::NotDispatched {
                context: wrong.clone(),
                reason: NotDispatchedReason::Expired,
            },
            SubmissionTerminal::Uncertain {
                context: wrong,
                reason: UncertainReason::AttemptInterrupted,
            },
        ] {
            assert_inert(&ledger.reduce(terminal, Some(&draft.stamp)));
            assert_eq!(ledger.pending_context(), Some(&request.context));
        }
        let mut wrong_operation = request.context.clone();
        wrong_operation.operation = Operation::EditMessage;
        assert_inert(&ledger.reduce(
            SubmissionTerminal::result(ResultEnvelope {
                context: wrong_operation,
                result: FrontendResult::EditMessage(WriteOutcome::Rejected(error())),
            }),
            Some(&draft.stamp),
        ));
        assert_eq!(ledger.status(), EditorWriteStatus::Pending);
    }

    #[test]
    fn genuine_read_same_id_is_inert_but_exact_write_context_bad_operation_is_unknown() {
        let draft = plain_draft();
        let mut ledger = EditorSubmission::default();
        let request = accepted(&mut ledger, &draft);
        let read = RequestSequence::default()
            .begin(FrontendIntent::WhoAmI, None)
            .unwrap();
        assert_eq!(read.context.id, request.context.id);
        let read_result = FrontendResult::WhoAmI(ReadOutcome::Success(PersonSnapshot {
            id: crate::frontend::data::PersonId::new("person"),
            display_name: None,
            emails: None,
        }));
        assert_inert(&ledger.reduce(
            SubmissionTerminal::result(read.complete(read_result.clone()).unwrap()),
            Some(&draft.stamp),
        ));
        assert_eq!(ledger.status(), EditorWriteStatus::Pending);
        let effects = ledger.reduce(
            SubmissionTerminal::result(ResultEnvelope {
                context: request.context,
                result: read_result,
            }),
            Some(&draft.stamp),
        );
        assert_eq!(effects.status, Some(EditorWriteStatus::DeliveryUnknown));
        assert!(effects.canonical.is_none());
    }

    #[test]
    fn wrong_write_operations_and_resource_payloads_are_never_success() {
        for target in targets() {
            for mismatch in 0..4 {
                let draft = draft(target.clone(), ComposedBody::PlainText("private".into()));
                let mut ledger = EditorSubmission::default();
                let request = accepted(&mut ledger, &draft);
                let mut canonical = message(&target);
                let payload = match mismatch {
                    0 => FrontendResult::DeleteMessage(WriteOutcome::Success(MessageId::new(
                        "wrong",
                    ))),
                    1 => {
                        canonical.room_id = RoomId::new("wrong-room");
                        result(&target, WriteOutcome::Success(canonical))
                    }
                    2 => {
                        match target {
                            DraftTarget::Edit { .. } => {
                                canonical.id = MessageId::new("wrong-message")
                            }
                            _ => canonical.parent_id = Some(MessageId::new("wrong-parent")),
                        }
                        result(&target, WriteOutcome::Success(canonical))
                    }
                    _ => {
                        canonical.id = MessageId::new("");
                        result(&target, WriteOutcome::Success(canonical))
                    }
                };
                let effects = ledger.reduce(
                    SubmissionTerminal::result(ResultEnvelope {
                        context: request.context,
                        result: payload,
                    }),
                    Some(&draft.stamp),
                );
                assert_eq!(effects.status, Some(EditorWriteStatus::DeliveryUnknown));
                assert!(effects.canonical.is_none());
                assert!(effects.clear_current.is_none());
                assert!(ledger.review().unwrap().draft == draft);
            }
        }
    }

    #[test]
    fn duplicate_and_stale_terminals_are_inert_and_local_cache_failure_cannot_replay() {
        let draft = plain_draft();
        let mut ledger = EditorSubmission::default();
        let request = accepted(&mut ledger, &draft);
        let context = request.context.clone();
        let canonical = message(&draft.stamp.target);
        let payload = result(
            &draft.stamp.target,
            WriteOutcome::Success(canonical.clone()),
        );
        let effects = ledger.reduce(
            SubmissionTerminal::result(request.complete(payload.clone()).unwrap()),
            Some(&draft.stamp),
        );
        // The pure canonical effect can be retained if its consumer fails locally.
        assert!(effects.canonical.as_ref() == Some(&canonical));
        assert_inert(&ledger.reduce(
            SubmissionTerminal::result(ResultEnvelope {
                context: context.clone(),
                result: payload,
            }),
            Some(&draft.stamp),
        ));
        assert_inert(&ledger.reduce(
            SubmissionTerminal::NotDispatched {
                context,
                reason: NotDispatchedReason::SessionFailed,
            },
            None,
        ));
        assert_inert(&lose_pending(&mut ledger, None));
        assert_eq!(ledger.status(), EditorWriteStatus::Ready);
        assert!(ledger.pending_context().is_none());
        assert!(ledger.review().is_none());
    }

    #[test]
    fn failed_version_retained_when_new_composition_and_target_are_visible() {
        let draft = plain_draft();
        let mut newer = draft.clone();
        newer.stamp.revision = newer.stamp.revision.next().unwrap();
        newer.stamp.target = targets()[2].clone();
        newer.body = ComposedBody::Markdown("**new composition**".into());
        let original_newer = newer.clone();
        let mut ledger = EditorSubmission::default();
        let request = accepted(&mut ledger, &draft);
        let effects = ledger.reduce(
            SubmissionTerminal::result(
                request
                    .complete(result(&draft.stamp.target, WriteOutcome::Rejected(error())))
                    .unwrap(),
            ),
            Some(&newer.stamp),
        );
        assert!(effects.clear_current.is_none());
        assert!(newer == original_newer);
        assert!(ledger.review().unwrap().draft == draft);
        assert!(ledger.dismiss_review());
        assert!(newer == original_newer);
    }

    #[test]
    fn delayed_missing_completion_cannot_settle_a_fresh_write() {
        let draft = plain_draft();
        let mut ledger = EditorSubmission::default();
        let first = accepted(&mut ledger, &draft);
        let old_context = first.context.clone();
        ledger.reduce(
            SubmissionTerminal::result(
                first
                    .complete(result(
                        &draft.stamp.target,
                        WriteOutcome::Success(message(&draft.stamp.target)),
                    ))
                    .unwrap(),
            ),
            Some(&draft.stamp),
        );
        let fresh = accepted(&mut ledger, &draft);
        assert_inert(&ledger.missing_completion(&old_context, None));
        assert_eq!(ledger.pending_context(), Some(&fresh.context));
        assert_eq!(ledger.status(), EditorWriteStatus::Pending);
    }

    #[test]
    fn pending_request_has_no_mutable_command_alias() {
        let draft = plain_draft();
        let mut ledger = EditorSubmission::default();
        let mut command = accepted(&mut ledger, &draft);
        command.context.operation = Operation::EditMessage;
        if let FrontendIntent::SendMessage { room_id, body, .. } = &mut command.intent {
            *room_id = RoomId::new("different-room");
            *body = ComposedBody::Markdown("different body".into());
        }
        assert!(ledger.pending.as_ref().unwrap().draft == draft);
        assert_eq!(
            ledger.pending_context().unwrap().operation,
            Operation::SendMessage
        );
        assert_eq!(
            ledger.admit(&command.context, QueueAdmission::Closed),
            Err(SubmissionError::AdmissionMismatch)
        );
        assert_eq!(ledger.status(), EditorWriteStatus::Pending);
    }

    #[test]
    fn draft_revision_exhaustion_fails_closed() {
        assert_eq!(
            DraftRevision(u64::MAX).next(),
            Err(SubmissionError::CounterExhausted)
        );
        assert_eq!(DraftRevision::default().next(), Ok(DraftRevision(1)));
    }
}
