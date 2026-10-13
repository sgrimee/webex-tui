//! Pure frontend intents/results for the 13-operation migration parity map.
//!
//! This is a transitional boundary, not a bridge wire protocol or dispatcher.
//! No backend/domain type crosses it. Payloads intentionally do not derive Debug:
//! bodies, names and emails must not accidentally become command/result traces.

use super::{
    data::{
        ComposedBody, Initialization, MessageId, MessagePage, MessageSnapshot, PageCursor,
        PersonId, PersonSnapshot, RoomId, RoomSnapshot, TeamId, TeamSnapshot,
    },
    requests::{CounterExhausted, RoomRequestContext, RoomViewRequests},
    threads::ThreadPage,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    Initialize,
    WhoAmI,
    ListAllRooms,
    UpdateRoom,
    UpdateTeam,
    UpdatePerson,
    ListMessagesInRoom,
    UpdateMessage,
    UpdateChildrenMessages,
    SendMessage,
    EditMessage,
    DeleteMessage,
    LeaveRoom,
}

impl Operation {
    pub(crate) fn is_write(self) -> bool {
        matches!(
            self,
            Self::SendMessage | Self::EditMessage | Self::DeleteMessage | Self::LeaveRoom
        )
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PageRequest {
    pub(crate) before: Option<PageCursor>,
    /// Frontend preference; authoritative size/cursor checks remain daemon-owned.
    pub(crate) max: u32,
}

#[derive(Clone, PartialEq, Eq)]
/// Cloneable only for private request correlation; intentionally no Debug,
/// Display, or serialization implementation because variants may carry bodies.
pub(crate) enum FrontendIntent {
    Initialize,
    WhoAmI,
    ListAllRooms,
    UpdateRoom(RoomId),
    UpdateTeam(TeamId),
    UpdatePerson(PersonId),
    ListMessagesInRoom {
        room_id: RoomId,
        page: PageRequest,
    },
    UpdateMessage {
        message_id: MessageId,
        room_id: RoomId,
    },
    UpdateChildrenMessages {
        parent_id: MessageId,
        room_id: RoomId,
        page: PageRequest,
    },
    SendMessage {
        room_id: RoomId,
        parent_id: Option<MessageId>,
        body: ComposedBody,
    },
    EditMessage {
        message_id: MessageId,
        room_id: RoomId,
        body: ComposedBody,
    },
    DeleteMessage {
        message_id: MessageId,
        room_id: RoomId,
    },
    LeaveRoom(RoomId),
}

impl FrontendIntent {
    pub(crate) fn operation(&self) -> Operation {
        match self {
            Self::Initialize => Operation::Initialize,
            Self::WhoAmI => Operation::WhoAmI,
            Self::ListAllRooms => Operation::ListAllRooms,
            Self::UpdateRoom(_) => Operation::UpdateRoom,
            Self::UpdateTeam(_) => Operation::UpdateTeam,
            Self::UpdatePerson(_) => Operation::UpdatePerson,
            Self::ListMessagesInRoom { .. } => Operation::ListMessagesInRoom,
            Self::UpdateMessage { .. } => Operation::UpdateMessage,
            Self::UpdateChildrenMessages { .. } => Operation::UpdateChildrenMessages,
            Self::SendMessage { .. } => Operation::SendMessage,
            Self::EditMessage { .. } => Operation::EditMessage,
            Self::DeleteMessage { .. } => Operation::DeleteMessage,
            Self::LeaveRoom(_) => Operation::LeaveRoom,
        }
    }

    pub(crate) fn room_id(&self) -> Option<&RoomId> {
        match self {
            Self::UpdateRoom(id) | Self::LeaveRoom(id) => Some(id),
            Self::ListMessagesInRoom { room_id, .. }
            | Self::UpdateMessage { room_id, .. }
            | Self::UpdateChildrenMessages { room_id, .. }
            | Self::SendMessage { room_id, .. }
            | Self::EditMessage { room_id, .. }
            | Self::DeleteMessage { room_id, .. } => Some(room_id),
            _ => None,
        }
    }
}

/// Stable classification and display text, already sanitized by the eventual
/// adapter. Never use this string as a command, identifier or retry policy.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct FrontendError {
    pub(crate) code: String,
    pub(crate) message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PartialReason {
    Offline,
    NotMirrored,
    IdentityUnavailable,
    ParentUnavailable,
    PermissionUnavailable,
    SyncInProgress,
    MorePages,
    RemoteUnavailable,
    ResourceMismatch,
}

/// A partial read still has usable data; pagination exhaustion is separate.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum ReadOutcome<T> {
    Success(T),
    Partial { value: T, reason: PartialReason },
    Rejected(FrontendError),
}

/// No retry queue or inferred success. Unknown is not rejection or confirmation.
/// Write outcomes cannot be represented as a partial read.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum WriteOutcome<T> {
    Success(T),
    Rejected(FrontendError),
    DeliveryUnknown(FrontendError),
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum FrontendResult {
    Initialize(ReadOutcome<Initialization>),
    WhoAmI(ReadOutcome<PersonSnapshot>),
    ListAllRooms(ReadOutcome<Vec<RoomSnapshot>>),
    UpdateRoom(ReadOutcome<RoomSnapshot>),
    UpdateTeam(ReadOutcome<TeamSnapshot>),
    UpdatePerson(ReadOutcome<PersonSnapshot>),
    ListMessagesInRoom(ReadOutcome<MessagePage>),
    UpdateMessage(ReadOutcome<MessageSnapshot>),
    UpdateChildrenMessages(ReadOutcome<ThreadPage>),
    SendMessage(WriteOutcome<MessageSnapshot>),
    EditMessage(WriteOutcome<MessageSnapshot>),
    DeleteMessage(WriteOutcome<MessageId>),
    LeaveRoom(WriteOutcome<RoomId>),
}

impl FrontendResult {
    pub(crate) fn operation(&self) -> Operation {
        match self {
            Self::Initialize(_) => Operation::Initialize,
            Self::WhoAmI(_) => Operation::WhoAmI,
            Self::ListAllRooms(_) => Operation::ListAllRooms,
            Self::UpdateRoom(_) => Operation::UpdateRoom,
            Self::UpdateTeam(_) => Operation::UpdateTeam,
            Self::UpdatePerson(_) => Operation::UpdatePerson,
            Self::ListMessagesInRoom(_) => Operation::ListMessagesInRoom,
            Self::UpdateMessage(_) => Operation::UpdateMessage,
            Self::UpdateChildrenMessages(_) => Operation::UpdateChildrenMessages,
            Self::SendMessage(_) => Operation::SendMessage,
            Self::EditMessage(_) => Operation::EditMessage,
            Self::DeleteMessage(_) => Operation::DeleteMessage,
            Self::LeaveRoom(_) => Operation::LeaveRoom,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RequestId(u64);

/// Immutable correlation captured once before work is dispatched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestContext {
    pub(crate) id: RequestId,
    pub(crate) operation: Operation,
    /// Existing room/read generation stamp, distinct from this global request ID.
    /// Used for focus/loading only; it must never suppress a write outcome.
    pub(crate) room_view: Option<RoomRequestContext>,
}

pub(crate) struct RequestEnvelope {
    pub(crate) context: RequestContext,
    pub(crate) intent: FrontendIntent,
}

pub(crate) struct ResultEnvelope {
    pub(crate) context: RequestContext,
    pub(crate) result: FrontendResult,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EnvelopeError {
    CounterExhausted,
    RoomContextMismatch,
    OperationMismatch,
}

impl From<CounterExhausted> for EnvelopeError {
    fn from(_: CounterExhausted) -> Self {
        Self::CounterExhausted
    }
}

#[derive(Default)]
pub(crate) struct RequestSequence {
    last_id: u64,
}

impl RequestSequence {
    /// Allocate IDs for every intent, not only message-page reads. An invalid
    /// context or exhausted sequence fails without changing allocation state.
    pub(crate) fn begin(
        &mut self,
        intent: FrontendIntent,
        room_view: Option<RoomRequestContext>,
    ) -> Result<RequestEnvelope, EnvelopeError> {
        if let Some(view) = &room_view {
            if intent.room_id().map(RoomId::as_str) != Some(view.room_id.as_str()) {
                return Err(EnvelopeError::RoomContextMismatch);
            }
        }
        let id = self
            .last_id
            .checked_add(1)
            .ok_or(EnvelopeError::CounterExhausted)?;
        let context = RequestContext {
            id: RequestId(id),
            operation: intent.operation(),
            room_view,
        };
        self.last_id = id;
        Ok(RequestEnvelope { context, intent })
    }
}

impl RequestEnvelope {
    /// Correlate a typed outcome without replaying/dispatching anything. The
    /// eventual adapter must also validate canonical resource IDs/room ownership;
    /// an operation mismatch is a local contract error, not proof of rejection.
    pub(crate) fn complete(self, result: FrontendResult) -> Result<ResultEnvelope, EnvelopeError> {
        if self.context.operation != self.intent.operation()
            || self.context.operation != result.operation()
        {
            return Err(EnvelopeError::OperationMismatch);
        }
        Ok(ResultEnvelope {
            context: self.context,
            result,
        })
    }
}

impl ResultEnvelope {
    /// View-specific hint only. A stale/unknown write result must still be
    /// processed; do not use this method as a global result-delivery filter.
    pub(crate) fn is_current_room_view(&self, requests: &RoomViewRequests) -> bool {
        self.context
            .room_view
            .as_ref()
            .is_some_and(|context| requests.is_current(context))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error() -> FrontendError {
        FrontendError {
            code: "fixture_error".into(),
            message: "Synthetic error".into(),
        }
    }

    fn page() -> PageRequest {
        PageRequest {
            before: None,
            max: 10,
        }
    }

    /// Every parity operation is constructible without any backend types.
    fn parity_cases() -> Vec<(FrontendIntent, FrontendResult)> {
        let room = || RoomId::new("room-a");
        let message = || MessageId::new("message-a");
        let body = || ComposedBody::PlainText("fixture".into());
        vec![
            (
                FrontendIntent::Initialize,
                FrontendResult::Initialize(ReadOutcome::Rejected(error())),
            ),
            (
                FrontendIntent::WhoAmI,
                FrontendResult::WhoAmI(ReadOutcome::Rejected(error())),
            ),
            (
                FrontendIntent::ListAllRooms,
                FrontendResult::ListAllRooms(ReadOutcome::Success(Vec::new())),
            ),
            (
                FrontendIntent::UpdateRoom(room()),
                FrontendResult::UpdateRoom(ReadOutcome::Rejected(error())),
            ),
            (
                FrontendIntent::UpdateTeam(TeamId::new("team-a")),
                FrontendResult::UpdateTeam(ReadOutcome::Rejected(error())),
            ),
            (
                FrontendIntent::UpdatePerson(PersonId::new("person-a")),
                FrontendResult::UpdatePerson(ReadOutcome::Rejected(error())),
            ),
            (
                FrontendIntent::ListMessagesInRoom {
                    room_id: room(),
                    page: page(),
                },
                FrontendResult::ListMessagesInRoom(ReadOutcome::Rejected(error())),
            ),
            (
                FrontendIntent::UpdateMessage {
                    message_id: message(),
                    room_id: room(),
                },
                FrontendResult::UpdateMessage(ReadOutcome::Rejected(error())),
            ),
            (
                FrontendIntent::UpdateChildrenMessages {
                    parent_id: message(),
                    room_id: room(),
                    page: page(),
                },
                FrontendResult::UpdateChildrenMessages(ReadOutcome::Rejected(error())),
            ),
            (
                FrontendIntent::SendMessage {
                    room_id: room(),
                    parent_id: None,
                    body: body(),
                },
                FrontendResult::SendMessage(WriteOutcome::DeliveryUnknown(error())),
            ),
            (
                FrontendIntent::EditMessage {
                    message_id: message(),
                    room_id: room(),
                    body: body(),
                },
                FrontendResult::EditMessage(WriteOutcome::DeliveryUnknown(error())),
            ),
            (
                FrontendIntent::DeleteMessage {
                    message_id: message(),
                    room_id: room(),
                },
                FrontendResult::DeleteMessage(WriteOutcome::Success(message())),
            ),
            (
                FrontendIntent::LeaveRoom(room()),
                FrontendResult::LeaveRoom(WriteOutcome::Rejected(error())),
            ),
        ]
    }

    #[test]
    fn all_thirteen_intents_have_matching_typed_results_and_unique_request_ids() {
        let cases = parity_cases();
        assert_eq!(cases.len(), 13);
        let mut sequence = RequestSequence::default();
        let mut operations = Vec::new();
        let mut last_id = 0;
        for (intent, result) in cases {
            let operation = intent.operation();
            assert!(!operations.contains(&operation));
            operations.push(operation);
            let request = sequence.begin(intent, None).unwrap();
            assert!(request.context.id.0 > last_id);
            last_id = request.context.id.0;
            let context = request.context.clone();
            let response = request.complete(result).unwrap();
            assert_eq!(response.context, context);
            assert_eq!(response.result.operation(), operation);
        }
        assert_eq!(operations.iter().filter(|op| op.is_write()).count(), 4);
    }

    #[test]
    fn unrelated_operation_result_is_rejected_without_an_envelope() {
        let request = RequestSequence::default()
            .begin(FrontendIntent::WhoAmI, None)
            .unwrap();
        let result = request.complete(FrontendResult::UpdatePerson(ReadOutcome::Rejected(error())));
        assert!(matches!(result, Err(EnvelopeError::OperationMismatch)));
    }

    #[test]
    fn partial_page_keeps_data_reason_and_cursor_separate() {
        let value = MessagePage {
            room_id: RoomId::new("room-a"),
            messages: Vec::new(),
            next_cursor: Some(PageCursor::new("opaque/cursor?boundary")),
        };
        let request = RequestSequence::default()
            .begin(
                FrontendIntent::ListMessagesInRoom {
                    room_id: RoomId::new("room-a"),
                    page: page(),
                },
                None,
            )
            .unwrap();
        let response = request
            .complete(FrontendResult::ListMessagesInRoom(ReadOutcome::Partial {
                value,
                reason: PartialReason::Offline,
            }))
            .unwrap();
        let FrontendResult::ListMessagesInRoom(ReadOutcome::Partial { value, reason }) =
            response.result
        else {
            panic!("partial page became success/rejection");
        };
        assert_eq!(reason, PartialReason::Offline);
        assert_eq!(
            value.next_cursor.as_ref().unwrap().as_str(),
            "opaque/cursor?boundary"
        );
    }

    #[test]
    fn confirmed_rejected_and_unknown_write_outcomes_are_distinct() {
        let success = WriteOutcome::Success(MessageId::new("message-a"));
        let rejected = WriteOutcome::<MessageId>::Rejected(error());
        let unknown = WriteOutcome::<MessageId>::DeliveryUnknown(error());
        assert!(success != rejected);
        assert!(rejected != unknown);
        assert!(unknown != success);
    }

    #[test]
    fn stale_delivery_unknown_write_still_has_a_correlated_result() {
        let mut views = RoomViewRequests::default();
        views.select_room(Some("room-a".into())).unwrap();
        let view = views.start_read().unwrap();
        let request = RequestSequence::default()
            .begin(FrontendIntent::LeaveRoom(RoomId::new("room-a")), view)
            .unwrap();
        let id = request.context.id;
        views.select_room(Some("room-b".into())).unwrap();
        let response = request
            .complete(FrontendResult::LeaveRoom(WriteOutcome::DeliveryUnknown(
                error(),
            )))
            .unwrap();
        assert_eq!(response.context.id, id);
        assert!(!response.is_current_room_view(&views));
        assert!(matches!(
            response.result,
            FrontendResult::LeaveRoom(WriteOutcome::DeliveryUnknown(_))
        ));
    }

    #[test]
    fn room_generation_prevents_aba_and_superseded_reads() {
        let mut views = RoomViewRequests::default();
        views.select_room(Some("room-a".into())).unwrap();
        let mut sequence = RequestSequence::default();
        let intent = || FrontendIntent::ListMessagesInRoom {
            room_id: RoomId::new("room-a"),
            page: page(),
        };
        let old = sequence
            .begin(intent(), views.start_read().unwrap())
            .unwrap();
        views.select_room(Some("room-b".into())).unwrap();
        views.select_room(Some("room-a".into())).unwrap();
        let current = sequence
            .begin(intent(), views.start_read().unwrap())
            .unwrap();
        let older = old
            .complete(FrontendResult::ListMessagesInRoom(ReadOutcome::Rejected(
                error(),
            )))
            .unwrap();
        let newer = current
            .complete(FrontendResult::ListMessagesInRoom(ReadOutcome::Rejected(
                error(),
            )))
            .unwrap();
        assert!(!older.is_current_room_view(&views));
        assert!(newer.is_current_room_view(&views));
        views.start_read().unwrap();
        assert!(!newer.is_current_room_view(&views));
    }

    #[test]
    fn wrong_room_context_does_not_consume_an_id() {
        let view = RoomRequestContext {
            room_id: "room-b".into(),
            view_generation: 1,
            request_id: 1,
        };
        let mut sequence = RequestSequence::default();
        let result = sequence.begin(
            FrontendIntent::UpdateRoom(RoomId::new("room-a")),
            Some(view),
        );
        assert!(matches!(result, Err(EnvelopeError::RoomContextMismatch)));
        assert_eq!(sequence.last_id, 0);
    }

    #[test]
    fn global_request_cannot_borrow_a_room_view_context() {
        let view = RoomRequestContext {
            room_id: "room-a".into(),
            view_generation: 1,
            request_id: 1,
        };
        let result = RequestSequence::default().begin(FrontendIntent::ListAllRooms, Some(view));
        assert!(matches!(result, Err(EnvelopeError::RoomContextMismatch)));
    }

    #[test]
    fn cloned_body_intents_preserve_opaque_ids_and_format_without_debug_surface() {
        let send = FrontendIntent::SendMessage {
            room_id: RoomId::new("room"),
            parent_id: Some(MessageId::new("parent")),
            body: ComposedBody::PlainText("body".into()),
        };
        let edit = FrontendIntent::EditMessage {
            message_id: MessageId::new("message"),
            room_id: RoomId::new("room"),
            body: ComposedBody::Markdown("body".into()),
        };
        assert!(
            matches!(send.clone(), FrontendIntent::SendMessage { parent_id: Some(id), body: ComposedBody::PlainText(body), .. } if id.as_str() == "parent" && body == "body")
        );
        assert!(
            matches!(edit.clone(), FrontendIntent::EditMessage { message_id, body: ComposedBody::Markdown(body), .. } if message_id.as_str() == "message" && body == "body")
        );
        // This test intentionally does not format the intent: it has no Debug/Display impl.
    }

    #[test]
    fn request_sequence_fails_closed_on_exhaustion() {
        let mut sequence = RequestSequence { last_id: u64::MAX };
        assert!(matches!(
            sequence.begin(FrontendIntent::WhoAmI, None),
            Err(EnvelopeError::CounterExhausted)
        ));
        assert_eq!(sequence.last_id, u64::MAX);
    }
}
