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
        let FrontendIntent::SendMessage {
            room_id,
            parent_id,
            body,
        } = intent
        else {
            return FrontendResult::SendMessage(WriteOutcome::Rejected(error("invalid_intent")));
        };
        let input = match body {
            ComposedBody::PlainText(content) => dto::MessageBodyInput {
                kind: "plainText".into(),
                content,
            },
            ComposedBody::Markdown(content) => dto::MessageBodyInput {
                kind: "markdown".into(),
                content,
            },
        };
        match self
            .backend
            .send(
                room_id.as_str(),
                parent_id.as_ref().map(MessageId::as_str),
                &input,
            )
            .await
        {
            Ok(message) => match project(message, room_id.as_str()) {
                Some(message) => FrontendResult::SendMessage(WriteOutcome::Success(message)),
                None => FrontendResult::SendMessage(WriteOutcome::DeliveryUnknown(error(
                    "invalid_response",
                ))),
            },
            Err(error_value) => {
                FrontendResult::SendMessage(if error_value.is_definitely_rejected() {
                    WriteOutcome::Rejected(error("webex_rejected"))
                } else {
                    WriteOutcome::DeliveryUnknown(error("delivery_unknown"))
                })
            }
        }
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
        reply: Option<Result<dto::Message, ClientError>>,
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
            self.reply.take().unwrap()
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
    #[tokio::test]
    async fn send_success_and_wrong_intent_do_not_leak_capability() {
        let mut d = WriteDispatcher::new(Fake {
            reply: Some(Ok(msg())),
            calls: 0,
        });
        assert!(matches!(
            d.dispatch(FrontendIntent::SendMessage {
                room_id: RoomId::new("r"),
                parent_id: None,
                body: ComposedBody::PlainText("x".into())
            })
            .await,
            FrontendResult::SendMessage(WriteOutcome::Success(_))
        ));
        assert!(matches!(
            d.dispatch(FrontendIntent::WhoAmI).await,
            FrontendResult::SendMessage(WriteOutcome::Rejected(_))
        ));
    }
}
