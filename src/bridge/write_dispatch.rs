//! Private, unconnected paired-UI send capability.
use crate::frontend::{
    data::{ComposedBody, MessageId, MessageSnapshot, MessageStatus, OwnedMessageBody, RoomId},
    envelopes::{FrontendError, FrontendIntent, FrontendResult, WriteOutcome},
};
use std::future::Future;
use webex_bridge::{
    client::{ui::UiClient, ClientError},
    dto,
};

pub(crate) trait UiWriteBackend: Send {
    fn send(
        &mut self,
        room: &str,
        parent: Option<&str>,
        body: &dto::MessageBodyInput,
    ) -> impl Future<Output = Result<dto::Message, ClientError>> + Send;
    fn edit(
        &mut self,
        message: &str,
        room: &str,
        body: &dto::MessageBodyInput,
    ) -> impl Future<Output = Result<dto::Message, ClientError>> + Send;
    fn delete(&mut self, message: &str) -> impl Future<Output = Result<(), ClientError>> + Send;
    fn leave(&mut self, room: &str) -> impl Future<Output = Result<(), ClientError>> + Send;
}
struct BridgeUiWriteBackend {
    client: UiClient,
}
impl UiWriteBackend for BridgeUiWriteBackend {
    async fn send(
        &mut self,
        room: &str,
        parent: Option<&str>,
        body: &dto::MessageBodyInput,
    ) -> Result<dto::Message, ClientError> {
        self.client.create_message_body(room, parent, body).await
    }
    async fn edit(
        &mut self,
        message: &str,
        room: &str,
        body: &dto::MessageBodyInput,
    ) -> Result<dto::Message, ClientError> {
        self.client.edit_message_body(message, room, body).await
    }
    async fn delete(&mut self, message: &str) -> Result<(), ClientError> {
        self.client.delete_message(message).await
    }
    async fn leave(&mut self, room: &str) -> Result<(), ClientError> {
        self.client.leave_room(room).await
    }
}
/// Holds the mutation capability privately; it exposes neither client nor bearer.
pub(crate) struct WriteDispatcher<B: UiWriteBackend> {
    backend: B,
}
impl<B: UiWriteBackend> WriteDispatcher<B> {
    pub(crate) fn new(backend: B) -> Self {
        Self { backend }
    }
    pub(crate) async fn dispatch(&mut self, intent: FrontendIntent) -> FrontendResult {
        match intent {
            FrontendIntent::SendMessage {
                room_id,
                parent_id,
                body,
            } => {
                let input = input(body);
                FrontendResult::SendMessage(message_outcome(
                    self.backend
                        .send(
                            room_id.as_str(),
                            parent_id.as_ref().map(MessageId::as_str),
                            &input,
                        )
                        .await,
                    room_id.as_str(),
                ))
            }
            FrontendIntent::EditMessage {
                message_id,
                room_id,
                body,
            } => {
                let input = input(body);
                FrontendResult::EditMessage(message_outcome(
                    self.backend
                        .edit(message_id.as_str(), room_id.as_str(), &input)
                        .await,
                    room_id.as_str(),
                ))
            }
            FrontendIntent::DeleteMessage {
                message_id,
                room_id,
            } => FrontendResult::DeleteMessage(unit_outcome(
                self.backend.delete(message_id.as_str()).await,
                message_id,
                room_id,
            )),
            FrontendIntent::LeaveRoom(room_id) => FrontendResult::LeaveRoom(unit_outcome(
                self.backend.leave(room_id.as_str()).await,
                room_id.clone(),
                room_id,
            )),
            _ => FrontendResult::SendMessage(WriteOutcome::Rejected(error("invalid_intent"))),
        }
    }
}
fn input(body: ComposedBody) -> dto::MessageBodyInput {
    match body {
        ComposedBody::PlainText(content) => dto::MessageBodyInput {
            kind: "plainText".into(),
            content,
        },
        ComposedBody::Markdown(content) => dto::MessageBodyInput {
            kind: "markdown".into(),
            content,
        },
    }
}
fn message_outcome(
    reply: Result<dto::Message, ClientError>,
    room: &str,
) -> WriteOutcome<MessageSnapshot> {
    match reply {
        Ok(message) => project(message, room)
            .map(WriteOutcome::Success)
            .unwrap_or_else(|| WriteOutcome::DeliveryUnknown(error("invalid_response"))),
        Err(error_value) => failure(error_value),
    }
}
fn unit_outcome<T>(reply: Result<(), ClientError>, value: T, _room: RoomId) -> WriteOutcome<T> {
    match reply {
        Ok(()) => WriteOutcome::Success(value),
        Err(error_value) => match failure::<MessageSnapshot>(error_value) {
            WriteOutcome::Rejected(error_value) => WriteOutcome::Rejected(error_value),
            WriteOutcome::DeliveryUnknown(error_value) => {
                WriteOutcome::DeliveryUnknown(error_value)
            }
            WriteOutcome::Success(_) => unreachable!(),
        },
    }
}
fn failure<T>(error_value: ClientError) -> WriteOutcome<T> {
    if error_value.is_definitely_rejected() {
        WriteOutcome::Rejected(error("webex_rejected"))
    } else {
        WriteOutcome::DeliveryUnknown(error("delivery_unknown"))
    }
}
fn error(code: &str) -> FrontendError {
    FrontendError {
        code: code.into(),
        message: "Write outcome requires review".into(),
    }
}
fn project(value: dto::Message, room: &str) -> Option<MessageSnapshot> {
    if value.id.is_empty()
        || value.room_id != room
        || value.status != "active"
        || value.revision < 0
    {
        return None;
    }
    let body = value.body.and_then(|body| match body.kind.as_str() {
        "plainText" => Some(OwnedMessageBody::Forms {
            text: Some(body.content),
            markdown: None,
            html: None,
        }),
        "markdown" => Some(OwnedMessageBody::Forms {
            text: None,
            markdown: Some(body.content),
            html: None,
        }),
        _ => None,
    });
    Some(MessageSnapshot {
        id: MessageId::new(value.id),
        room_id: RoomId::new(value.room_id),
        revision: Some(value.revision as u64),
        parent_id: value.parent_id.map(MessageId::new),
        sender_id: value.sender_id.map(crate::frontend::data::PersonId::new),
        sender_email: None,
        created_at: value.created_at,
        updated_at: value.updated_at,
        status: MessageStatus::Active,
        body,
        attachment_count: Some(value.attachment_count as usize),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use webex_bridge::client::ClientError;
    struct Fake {
        send: Option<Result<dto::Message, ClientError>>,
        edit: Option<Result<dto::Message, ClientError>>,
        delete: Option<Result<(), ClientError>>,
        leave: Option<Result<(), ClientError>>,
        calls: usize,
    }
    impl UiWriteBackend for Fake {
        async fn send(
            &mut self,
            _: &str,
            _: Option<&str>,
            _: &dto::MessageBodyInput,
        ) -> Result<dto::Message, ClientError> {
            self.calls += 1;
            self.send.take().unwrap()
        }
        async fn edit(
            &mut self,
            _: &str,
            _: &str,
            _: &dto::MessageBodyInput,
        ) -> Result<dto::Message, ClientError> {
            self.calls += 1;
            self.edit.take().unwrap()
        }
        async fn delete(&mut self, _: &str) -> Result<(), ClientError> {
            self.calls += 1;
            self.delete.take().unwrap()
        }
        async fn leave(&mut self, _: &str) -> Result<(), ClientError> {
            self.calls += 1;
            self.leave.take().unwrap()
        }
    }
    fn msg() -> dto::Message {
        dto::Message {
            id: "m".into(),
            room_id: "r".into(),
            revision: 1,
            parent_id: None,
            sender_id: None,
            created_at: None,
            updated_at: None,
            status: "active".into(),
            markdown: None,
            body: Some(dto::MessageBody {
                kind: "plainText".into(),
                content: "x".into(),
            }),
            attachment_count: 0,
        }
    }
    fn fake(
        send: Result<dto::Message, ClientError>,
        edit: Result<dto::Message, ClientError>,
        delete: Result<(), ClientError>,
        leave: Result<(), ClientError>,
    ) -> WriteDispatcher<Fake> {
        WriteDispatcher::new(Fake {
            send: Some(send),
            edit: Some(edit),
            delete: Some(delete),
            leave: Some(leave),
            calls: 0,
        })
    }
    fn unknown() -> ClientError {
        ClientError::Api {
            status: reqwest::StatusCode::ACCEPTED,
            code: "delivery_unknown".into(),
            message: "safe".into(),
        }
    }
    fn transport() -> ClientError {
        ClientError::Transport(std::io::Error::other("transport").into())
    }
    fn rejected() -> ClientError {
        ClientError::Api {
            status: reqwest::StatusCode::BAD_REQUEST,
            code: "webex_rejected".into(),
            message: "safe".into(),
        }
    }
    fn send() -> FrontendIntent {
        FrontendIntent::SendMessage {
            room_id: RoomId::new("r"),
            parent_id: None,
            body: ComposedBody::PlainText("body".into()),
        }
    }
    fn edit() -> FrontendIntent {
        FrontendIntent::EditMessage {
            message_id: MessageId::new("m"),
            room_id: RoomId::new("r"),
            body: ComposedBody::Markdown("body".into()),
        }
    }
    #[tokio::test]
    async fn send_success_and_wrong_intent_do_not_leak_capability() {
        let mut d = fake(Ok(msg()), Ok(msg()), Ok(()), Ok(()));
        assert!(matches!(
            d.dispatch(send()).await,
            FrontendResult::SendMessage(WriteOutcome::Success(_))
        ));
        assert!(matches!(
            d.dispatch(FrontendIntent::WhoAmI).await,
            FrontendResult::SendMessage(WriteOutcome::Rejected(_))
        ));
        assert_eq!(d.backend.calls, 1);
    }
    #[tokio::test]
    async fn each_write_dispatches_once_and_projects_success() {
        let mut d = fake(Ok(msg()), Ok(msg()), Ok(()), Ok(()));
        assert!(matches!(
            d.dispatch(send()).await,
            FrontendResult::SendMessage(WriteOutcome::Success(_))
        ));
        assert!(matches!(
            d.dispatch(edit()).await,
            FrontendResult::EditMessage(WriteOutcome::Success(_))
        ));
        assert!(matches!(
            d.dispatch(FrontendIntent::DeleteMessage {
                message_id: MessageId::new("m"),
                room_id: RoomId::new("r")
            })
            .await,
            FrontendResult::DeleteMessage(WriteOutcome::Success(_))
        ));
        assert!(matches!(
            d.dispatch(FrontendIntent::LeaveRoom(RoomId::new("r")))
                .await,
            FrontendResult::LeaveRoom(WriteOutcome::Success(_))
        ));
        assert_eq!(d.backend.calls, 4);
    }
    #[tokio::test]
    async fn rejection_and_uncertainty_are_not_retried_or_leaked() {
        let mut rejected_dispatcher = fake(Err(rejected()), Ok(msg()), Ok(()), Ok(()));
        assert!(matches!(
            rejected_dispatcher.dispatch(send()).await,
            FrontendResult::SendMessage(WriteOutcome::Rejected(_))
        ));
        assert_eq!(rejected_dispatcher.backend.calls, 1);
        let mut unknown_dispatcher = fake(Err(unknown()), Ok(msg()), Ok(()), Ok(()));
        let result = unknown_dispatcher.dispatch(send()).await;
        assert!(matches!(
            result,
            FrontendResult::SendMessage(WriteOutcome::DeliveryUnknown(_))
        ));
        assert_eq!(unknown_dispatcher.backend.calls, 1);
    }
    #[tokio::test]
    async fn malformed_success_is_unknown_without_second_call() {
        let mut bad = msg();
        bad.room_id = "other".into();
        let mut d = fake(Ok(bad), Ok(msg()), Ok(()), Ok(()));
        assert!(matches!(
            d.dispatch(send()).await,
            FrontendResult::SendMessage(WriteOutcome::DeliveryUnknown(_))
        ));
        assert_eq!(d.backend.calls, 1);
    }
    #[tokio::test]
    async fn transport_uncertainty_is_once_for_each_write() {
        let mut send_d = fake(Err(transport()), Ok(msg()), Ok(()), Ok(()));
        assert!(matches!(
            send_d.dispatch(send()).await,
            FrontendResult::SendMessage(WriteOutcome::DeliveryUnknown(_))
        ));
        assert_eq!(send_d.backend.calls, 1);
        let mut edit_d = fake(Ok(msg()), Err(transport()), Ok(()), Ok(()));
        assert!(matches!(
            edit_d.dispatch(edit()).await,
            FrontendResult::EditMessage(WriteOutcome::DeliveryUnknown(_))
        ));
        assert_eq!(edit_d.backend.calls, 1);
        let mut delete_d = fake(Ok(msg()), Ok(msg()), Err(transport()), Ok(()));
        assert!(matches!(
            delete_d
                .dispatch(FrontendIntent::DeleteMessage {
                    message_id: MessageId::new("m"),
                    room_id: RoomId::new("r")
                })
                .await,
            FrontendResult::DeleteMessage(WriteOutcome::DeliveryUnknown(_))
        ));
        assert_eq!(delete_d.backend.calls, 1);
        let mut leave_d = fake(Ok(msg()), Ok(msg()), Ok(()), Err(transport()));
        assert!(matches!(
            leave_d
                .dispatch(FrontendIntent::LeaveRoom(RoomId::new("r")))
                .await,
            FrontendResult::LeaveRoom(WriteOutcome::DeliveryUnknown(_))
        ));
        assert_eq!(leave_d.backend.calls, 1);
    }
    #[tokio::test]
    async fn accepted_uncertainty_is_once_for_each_write() {
        let mut edit_d = fake(Ok(msg()), Err(unknown()), Ok(()), Ok(()));
        assert!(matches!(
            edit_d.dispatch(edit()).await,
            FrontendResult::EditMessage(WriteOutcome::DeliveryUnknown(_))
        ));
        assert_eq!(edit_d.backend.calls, 1);
        let mut delete_d = fake(Ok(msg()), Ok(msg()), Err(unknown()), Ok(()));
        assert!(matches!(
            delete_d
                .dispatch(FrontendIntent::DeleteMessage {
                    message_id: MessageId::new("m"),
                    room_id: RoomId::new("r")
                })
                .await,
            FrontendResult::DeleteMessage(WriteOutcome::DeliveryUnknown(_))
        ));
        assert_eq!(delete_d.backend.calls, 1);
        let mut leave_d = fake(Ok(msg()), Ok(msg()), Ok(()), Err(unknown()));
        assert!(matches!(
            leave_d
                .dispatch(FrontendIntent::LeaveRoom(RoomId::new("r")))
                .await,
            FrontendResult::LeaveRoom(WriteOutcome::DeliveryUnknown(_))
        ));
        assert_eq!(leave_d.backend.calls, 1);
    }
}
