//! Pure, backend-neutral editor write state.
//!
//! This is intentionally unconnected: it allocates/correlates envelopes but
//! neither dispatches them nor retries a rejected or uncertain write. Body
//! values never implement `Debug` and this module performs no logging.

use super::{
    data::{ComposedBody, MessageId, RoomId},
    envelopes::{
        EnvelopeError, FrontendIntent, FrontendResult, RequestEnvelope, RequestId, RequestSequence,
        ResultEnvelope, WriteOutcome,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EditorWriteStatus {
    Ready,
    Pending,
    Rejected,
    DeliveryUnknown,
    /// Local lifecycle gate prevented dispatch; it is not a daemon rejection.
    Unavailable,
    Cancelled,
}

/// Composition is retained across every non-success outcome for user review.
pub(crate) struct EditorWriteState {
    sequence: RequestSequence,
    body: ComposedBody,
    pending: Option<RequestId>,
    status: EditorWriteStatus,
}

impl EditorWriteState {
    pub(crate) fn new(body: ComposedBody) -> Self {
        Self {
            sequence: RequestSequence::default(),
            body,
            pending: None,
            status: EditorWriteStatus::Ready,
        }
    }
    pub(crate) fn body(&self) -> &ComposedBody {
        &self.body
    }
    pub(crate) fn status(&self) -> EditorWriteStatus {
        self.status
    }
    pub(crate) fn replace_body(&mut self, body: ComposedBody) {
        self.body = body;
        self.pending = None;
        self.status = EditorWriteStatus::Ready;
    }
    pub(crate) fn send(
        &mut self,
        room_id: RoomId,
        parent_id: Option<MessageId>,
    ) -> Result<RequestEnvelope, EnvelopeError> {
        self.begin(FrontendIntent::SendMessage {
            room_id,
            parent_id,
            body: self.body.clone(),
        })
    }
    pub(crate) fn edit(
        &mut self,
        room_id: RoomId,
        message_id: MessageId,
    ) -> Result<RequestEnvelope, EnvelopeError> {
        self.begin(FrontendIntent::EditMessage {
            room_id,
            message_id,
            body: self.body.clone(),
        })
    }
    fn begin(&mut self, intent: FrontendIntent) -> Result<RequestEnvelope, EnvelopeError> {
        let request = self.sequence.begin(intent, None)?;
        self.pending = Some(request.context.id);
        self.status = EditorWriteStatus::Pending;
        Ok(request)
    }
    /// Clear only the matching pending request when it never reaches the daemon.
    pub(crate) fn unavailable(&mut self, request: &RequestEnvelope) {
        if self.pending == Some(request.context.id) {
            self.pending = None;
            self.status = EditorWriteStatus::Unavailable;
        }
    }
    pub(crate) fn cancel(&mut self) {
        self.pending = None;
        self.status = EditorWriteStatus::Cancelled;
    }
    /// Reads never alter write state. A stale or unrelated write is ignored.
    pub(crate) fn accept(&mut self, result: &ResultEnvelope) {
        if !result.context.operation.is_write() || self.pending != Some(result.context.id) {
            return;
        }
        self.status = match &result.result {
            FrontendResult::SendMessage(WriteOutcome::Success(_))
            | FrontendResult::EditMessage(WriteOutcome::Success(_)) => EditorWriteStatus::Ready,
            FrontendResult::SendMessage(WriteOutcome::Rejected(_))
            | FrontendResult::EditMessage(WriteOutcome::Rejected(_)) => EditorWriteStatus::Rejected,
            FrontendResult::SendMessage(WriteOutcome::DeliveryUnknown(_))
            | FrontendResult::EditMessage(WriteOutcome::DeliveryUnknown(_)) => {
                EditorWriteStatus::DeliveryUnknown
            }
            _ => return,
        };
        self.pending = None;
    }
}

/// Labels intentionally omit legacy person/resource types and terminal controls.
pub(crate) fn sanitized_label(display_name: Option<&str>, email: Option<&str>) -> String {
    display_name
        .or(email)
        .unwrap_or("Unknown")
        .chars()
        .filter(|character| !character.is_control())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::{
        data::{MessageSnapshot, MessageStatus},
        envelopes::{FrontendError, ReadOutcome},
    };
    fn body() -> ComposedBody {
        ComposedBody::PlainText("private body".into())
    }
    fn error() -> FrontendError {
        FrontendError {
            code: "rejected".into(),
            message: "safe".into(),
        }
    }
    fn snapshot() -> MessageSnapshot {
        MessageSnapshot {
            id: MessageId::new("message"),
            room_id: RoomId::new("room"),
            revision: None,
            parent_id: None,
            sender_id: None,
            sender_email: None,
            created_at: None,
            updated_at: None,
            status: MessageStatus::Active,
            body: None,
            attachment_count: None,
        }
    }
    #[test]
    fn send_reply_and_edit_allocate_typed_intents() {
        let mut state = EditorWriteState::new(body());
        assert!(matches!(
            state.send(RoomId::new("room"), None).unwrap().intent,
            FrontendIntent::SendMessage {
                parent_id: None,
                ..
            }
        ));
        assert!(matches!(
            state
                .send(RoomId::new("room"), Some(MessageId::new("parent")))
                .unwrap()
                .intent,
            FrontendIntent::SendMessage {
                parent_id: Some(_),
                ..
            }
        ));
        assert!(matches!(
            state
                .edit(RoomId::new("room"), MessageId::new("message"))
                .unwrap()
                .intent,
            FrontendIntent::EditMessage { .. }
        ));
    }
    #[test]
    fn rejection_restores_draft_without_retry() {
        let mut state = EditorWriteState::new(body());
        let request = state.send(RoomId::new("room"), None).unwrap();
        let result = request
            .complete(FrontendResult::SendMessage(WriteOutcome::Rejected(error())))
            .unwrap();
        state.accept(&result);
        assert_eq!(state.status(), EditorWriteStatus::Rejected);
        assert!(matches!(state.body(),ComposedBody::PlainText(value)if value=="private body"));
    }
    #[test]
    fn delivery_unknown_and_cancel_preserve_draft() {
        let mut state = EditorWriteState::new(body());
        let request = state.send(RoomId::new("room"), None).unwrap();
        let result = request
            .complete(FrontendResult::SendMessage(WriteOutcome::DeliveryUnknown(
                error(),
            )))
            .unwrap();
        state.accept(&result);
        assert_eq!(state.status(), EditorWriteStatus::DeliveryUnknown);
        state.cancel();
        assert_eq!(state.status(), EditorWriteStatus::Cancelled);
        assert!(matches!(state.body(),ComposedBody::PlainText(value)if value=="private body"));
    }
    #[test]
    fn stale_read_is_independent_of_pending_write() {
        let mut state = EditorWriteState::new(body());
        let request = state.send(RoomId::new("room"), None).unwrap();
        let mut reads = RequestSequence::default();
        let read = reads
            .begin(FrontendIntent::WhoAmI, None)
            .unwrap()
            .complete(FrontendResult::WhoAmI(ReadOutcome::Rejected(error())))
            .unwrap();
        state.accept(&read);
        assert_eq!(state.status(), EditorWriteStatus::Pending);
        let result = request
            .complete(FrontendResult::SendMessage(WriteOutcome::Success(
                snapshot(),
            )))
            .unwrap();
        state.accept(&result);
        assert_eq!(state.status(), EditorWriteStatus::Ready);
    }
    #[test]
    fn unavailable_matching_request_retains_send_reply_and_edit_drafts() {
        for request_kind in 0..3 {
            let mut state = EditorWriteState::new(body());
            let request = match request_kind {
                0 => state.send(RoomId::new("room"), None).unwrap(),
                1 => state
                    .send(RoomId::new("room"), Some(MessageId::new("parent")))
                    .unwrap(),
                _ => state
                    .edit(RoomId::new("room"), MessageId::new("message"))
                    .unwrap(),
            };
            state.unavailable(&request);
            assert_eq!(state.status(), EditorWriteStatus::Unavailable);
            assert!(
                matches!(state.body(), ComposedBody::PlainText(value) if value == "private body")
            );
            state.replace_body(ComposedBody::Markdown("next".into()));
            assert_eq!(state.status(), EditorWriteStatus::Ready);
        }
    }
    #[test]
    fn stale_unavailable_request_does_not_clear_current_pending() {
        let mut state = EditorWriteState::new(body());
        let stale = state.send(RoomId::new("room"), None).unwrap();
        let current = state
            .edit(RoomId::new("room"), MessageId::new("message"))
            .unwrap();
        state.unavailable(&stale);
        assert_eq!(state.status(), EditorWriteStatus::Pending);
        state.unavailable(&current);
        assert_eq!(state.status(), EditorWriteStatus::Unavailable);
    }
    #[test]
    fn labels_are_sanitized_and_body_has_no_debug_escape() {
        assert_eq!(sanitized_label(Some("A\nB"), None), "AB");
        assert_eq!(sanitized_label(None, None), "Unknown");
    }
}
