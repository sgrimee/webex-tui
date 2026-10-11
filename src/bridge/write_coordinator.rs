//! Unconnected correlation between the neutral editor reducer and write gate.
//!
//! This module owns neither a client, bearer, paired secret, task nor terminal
//! state. It only correlates a private intent clone with the retained request
//! envelope so an exact typed result can update the editor reducer.

use super::{
    controller::{HostController, PresentationHost},
    session::{SessionBackend, SessionClock, SessionWriteError},
};
use crate::frontend::{
    data::{MessageId, RoomId},
    editor_write::EditorWriteState,
    envelopes::{FrontendIntent, FrontendResult, RequestEnvelope},
};
use std::future::Future;

/// Narrow mutation gate. Implementors expose only typed results, never a
/// session, client, bearer, reader or generic request capability.
pub(crate) trait WriteGate {
    fn dispatch_write(
        &mut self,
        intent: FrontendIntent,
    ) -> impl Future<Output = Result<FrontendResult, SessionWriteError>>;
}

impl<B: SessionBackend, C: SessionClock, H: PresentationHost> WriteGate
    for HostController<B, C, H>
{
    async fn dispatch_write(
        &mut self,
        intent: FrontendIntent,
    ) -> Result<FrontendResult, SessionWriteError> {
        HostController::dispatch_write(self, intent).await
    }
}

/// Metadata-only coordinator failures. They intentionally contain no backend
/// detail, composition, resource IDs, client or session material.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoordinatorError {
    Unavailable,
    ResultMismatch,
}

/// Owns the neutral editor reducer and a narrow write gate. It never retries a
/// write: every submission produces at most one gate invocation.
pub(crate) struct WriteCoordinator<G: WriteGate> {
    state: EditorWriteState,
    gate: G,
}

impl<G: WriteGate> WriteCoordinator<G> {
    pub(crate) fn new(state: EditorWriteState, gate: G) -> Self {
        Self { state, gate }
    }

    pub(crate) fn state(&self) -> &EditorWriteState {
        &self.state
    }

    pub(crate) async fn send(
        &mut self,
        room_id: RoomId,
        parent_id: Option<MessageId>,
    ) -> Result<FrontendResult, CoordinatorError> {
        let request = self
            .state
            .send(room_id, parent_id)
            .map_err(|_| CoordinatorError::ResultMismatch)?;
        self.run(request).await
    }

    pub(crate) async fn edit(
        &mut self,
        room_id: RoomId,
        message_id: MessageId,
    ) -> Result<FrontendResult, CoordinatorError> {
        let request = self
            .state
            .edit(room_id, message_id)
            .map_err(|_| CoordinatorError::ResultMismatch)?;
        self.run(request).await
    }

    async fn run(&mut self, request: RequestEnvelope) -> Result<FrontendResult, CoordinatorError> {
        // Retain a private correlation copy because `complete` consumes the
        // request. Only the intent clone crosses the narrow gate.
        let fallback = RequestEnvelope {
            context: request.context.clone(),
            intent: request.intent.clone(),
        };
        let result = match self.gate.dispatch_write(request.intent.clone()).await {
            Ok(result) => result,
            Err(_) => {
                self.state.unavailable(&request);
                return Err(CoordinatorError::Unavailable);
            }
        };
        let envelope = match request.complete(result) {
            Ok(envelope) => envelope,
            Err(_) => {
                self.state.unavailable(&fallback);
                return Err(CoordinatorError::ResultMismatch);
            }
        };
        self.state.accept(&envelope);
        Ok(envelope.result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::{
        data::{ComposedBody, MessageSnapshot, MessageStatus},
        editor_write::EditorWriteStatus,
        envelopes::{FrontendError, WriteOutcome},
    };

    struct Gate {
        reply: Option<Result<FrontendResult, SessionWriteError>>,
        calls: usize,
    }

    impl WriteGate for Gate {
        async fn dispatch_write(
            &mut self,
            _: FrontendIntent,
        ) -> Result<FrontendResult, SessionWriteError> {
            self.calls += 1;
            self.reply.take().expect("one gate call")
        }
    }

    fn error() -> FrontendError {
        FrontendError {
            code: "safe".into(),
            message: "safe".into(),
        }
    }

    fn message() -> MessageSnapshot {
        MessageSnapshot {
            id: MessageId::new("message"),
            room_id: RoomId::new("room"),
            revision: Some(1),
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

    fn coordinator(reply: Result<FrontendResult, SessionWriteError>) -> WriteCoordinator<Gate> {
        WriteCoordinator::new(
            EditorWriteState::new(ComposedBody::PlainText("private body".into())),
            Gate {
                reply: Some(reply),
                calls: 0,
            },
        )
    }

    #[tokio::test]
    async fn success_and_unknown_complete_matching_editor_write_once() {
        let mut success = coordinator(Ok(FrontendResult::SendMessage(WriteOutcome::Success(
            message(),
        ))));
        assert!(matches!(
            success.send(RoomId::new("room"), None).await,
            Ok(FrontendResult::SendMessage(WriteOutcome::Success(_)))
        ));
        assert_eq!(success.state().status(), EditorWriteStatus::Ready);
        assert_eq!(success.gate.calls, 1);

        let mut unknown = coordinator(Ok(FrontendResult::SendMessage(
            WriteOutcome::DeliveryUnknown(error()),
        )));
        assert!(matches!(
            unknown
                .send(RoomId::new("room"), Some(MessageId::new("parent")))
                .await,
            Ok(FrontendResult::SendMessage(WriteOutcome::DeliveryUnknown(
                _
            )))
        ));
        assert_eq!(unknown.state().status(), EditorWriteStatus::DeliveryUnknown);
        assert_eq!(unknown.gate.calls, 1);
    }

    #[tokio::test]
    async fn gate_failure_and_mismatched_result_retain_unavailable_draft() {
        let mut unavailable = coordinator(Err(SessionWriteError::NotReady));
        assert!(matches!(
            unavailable.send(RoomId::new("room"), None).await,
            Err(CoordinatorError::Unavailable)
        ));
        assert_eq!(unavailable.state().status(), EditorWriteStatus::Unavailable);
        assert!(
            matches!(unavailable.state().body(), ComposedBody::PlainText(value) if value == "private body")
        );
        assert_eq!(unavailable.gate.calls, 1);

        let mut mismatch = coordinator(Ok(FrontendResult::DeleteMessage(WriteOutcome::Success(
            MessageId::new("message"),
        ))));
        assert!(matches!(
            mismatch
                .edit(RoomId::new("room"), MessageId::new("message"))
                .await,
            Err(CoordinatorError::ResultMismatch)
        ));
        assert_eq!(mismatch.state().status(), EditorWriteStatus::Unavailable);
        assert_eq!(mismatch.gate.calls, 1);
    }
}
