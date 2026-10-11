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
    use crate::bridge::write_dispatch::DispatchError;
    use crate::frontend::{
        data::{ComposedBody, MessageSnapshot, MessageStatus},
        editor_write::EditorWriteStatus,
        envelopes::{FrontendError, WriteOutcome},
    };

    struct Gate {
        reply: Option<Result<FrontendResult, SessionWriteError>>,
        calls: usize,
        // Keep payloads private: neither this fake nor its intents implement Debug.
        received: Vec<FrontendIntent>,
    }

    impl WriteGate for Gate {
        async fn dispatch_write(
            &mut self,
            intent: FrontendIntent,
        ) -> Result<FrontendResult, SessionWriteError> {
            self.calls += 1;
            self.received.push(intent);
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
        coordinator_with_body(ComposedBody::PlainText("private body".into()), reply)
    }

    fn coordinator_with_body(
        body: ComposedBody,
        reply: Result<FrontendResult, SessionWriteError>,
    ) -> WriteCoordinator<Gate> {
        WriteCoordinator::new(
            EditorWriteState::new(body),
            Gate {
                reply: Some(reply),
                calls: 0,
                received: Vec::new(),
            },
        )
    }

    fn assert_dispatched_once(gate: &Gate, expected: FrontendIntent) {
        assert_eq!(gate.calls, 1);
        assert_eq!(gate.received.len(), 1);
        // Boolean equality checks every field without formatting either payload.
        assert!(gate.received[0] == expected);
        assert!(gate.reply.is_none());
    }

    fn draft_intents(body: &ComposedBody) -> [FrontendIntent; 3] {
        [
            FrontendIntent::SendMessage {
                room_id: RoomId::new("room"),
                parent_id: None,
                body: body.clone(),
            },
            FrontendIntent::SendMessage {
                room_id: RoomId::new("room"),
                parent_id: Some(MessageId::new("parent")),
                body: body.clone(),
            },
            FrontendIntent::EditMessage {
                room_id: RoomId::new("room"),
                message_id: MessageId::new("message"),
                body: body.clone(),
            },
        ]
    }

    async fn submit(
        coordinator: &mut WriteCoordinator<Gate>,
        expected: &FrontendIntent,
    ) -> Result<FrontendResult, CoordinatorError> {
        match expected {
            FrontendIntent::SendMessage {
                room_id, parent_id, ..
            } => coordinator.send(room_id.clone(), parent_id.clone()).await,
            FrontendIntent::EditMessage {
                room_id,
                message_id,
                ..
            } => coordinator.edit(room_id.clone(), message_id.clone()).await,
            _ => panic!("fixture must be an editor write"),
        }
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
        assert_dispatched_once(
            &success.gate,
            FrontendIntent::SendMessage {
                room_id: RoomId::new("room"),
                parent_id: None,
                body: ComposedBody::PlainText("private body".into()),
            },
        );

        let body = ComposedBody::Markdown("**private reply**".into());
        let mut unknown = coordinator_with_body(
            body.clone(),
            Ok(FrontendResult::SendMessage(WriteOutcome::DeliveryUnknown(
                error(),
            ))),
        );
        assert!(matches!(
            unknown
                .send(RoomId::new("room"), Some(MessageId::new("parent")))
                .await,
            Ok(FrontendResult::SendMessage(WriteOutcome::DeliveryUnknown(
                _
            )))
        ));
        assert_eq!(unknown.state().status(), EditorWriteStatus::DeliveryUnknown);
        assert!(unknown.state().body() == &body);
        assert_dispatched_once(
            &unknown.gate,
            FrontendIntent::SendMessage {
                room_id: RoomId::new("room"),
                parent_id: Some(MessageId::new("parent")),
                body,
            },
        );
    }

    #[tokio::test]
    async fn matching_edit_success_forwards_exact_markdown_intent_once() {
        let body = ComposedBody::Markdown("**private edit**".into());
        let expected = FrontendIntent::EditMessage {
            room_id: RoomId::new("edit-room"),
            message_id: MessageId::new("edit-message"),
            body: body.clone(),
        };
        let result = FrontendResult::EditMessage(WriteOutcome::Success(MessageSnapshot {
            id: MessageId::new("edit-message"),
            room_id: RoomId::new("edit-room"),
            ..message()
        }));
        let mut edit = coordinator_with_body(body, Ok(result.clone()));
        assert!(submit(&mut edit, &expected).await.unwrap() == result);
        assert_eq!(edit.state().status(), EditorWriteStatus::Ready);
        assert_dispatched_once(&edit.gate, expected);
    }

    #[tokio::test]
    async fn rejected_send_reply_and_edit_retain_exact_draft_without_retry() {
        for body in [
            ComposedBody::PlainText("private body".into()),
            ComposedBody::Markdown("**private body**".into()),
        ] {
            for expected in draft_intents(&body) {
                let result = match &expected {
                    FrontendIntent::SendMessage { .. } => {
                        FrontendResult::SendMessage(WriteOutcome::Rejected(error()))
                    }
                    FrontendIntent::EditMessage { .. } => {
                        FrontendResult::EditMessage(WriteOutcome::Rejected(error()))
                    }
                    _ => unreachable!(),
                };
                let mut rejected = coordinator_with_body(body.clone(), Ok(result.clone()));
                assert!(submit(&mut rejected, &expected).await.unwrap() == result);
                assert_eq!(rejected.state().status(), EditorWriteStatus::Rejected);
                assert!(rejected.state().body() == &body);
                assert_dispatched_once(&rejected.gate, expected);
            }
        }
    }

    #[tokio::test]
    async fn every_gate_failure_retains_send_reply_and_edit_drafts_without_retry() {
        let failures = [
            SessionWriteError::NotReady,
            SessionWriteError::Expired,
            SessionWriteError::Failed,
            SessionWriteError::Dispatch(DispatchError::WrongIntent),
        ];
        for failure in failures {
            for body in [
                ComposedBody::PlainText("private body".into()),
                ComposedBody::Markdown("**private body**".into()),
            ] {
                for expected in draft_intents(&body) {
                    let mut unavailable = coordinator_with_body(body.clone(), Err(failure));
                    assert!(matches!(
                        submit(&mut unavailable, &expected).await,
                        Err(CoordinatorError::Unavailable)
                    ));
                    assert_eq!(unavailable.state().status(), EditorWriteStatus::Unavailable);
                    assert!(unavailable.state().body() == &body);
                    assert_dispatched_once(&unavailable.gate, expected);
                }
            }
        }
    }

    #[tokio::test]
    async fn mismatched_result_retains_unavailable_draft_without_retry() {
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
        assert!(
            matches!(mismatch.state().body(), ComposedBody::PlainText(value) if value == "private body")
        );
        assert_dispatched_once(
            &mismatch.gate,
            FrontendIntent::EditMessage {
                room_id: RoomId::new("room"),
                message_id: MessageId::new("message"),
                body: ComposedBody::PlainText("private body".into()),
            },
        );
    }
}
