//! Pure conversion of real bridge DTOs/client results to frontend-owned values.
//! The caller supplies the original intent; no API method is called here.

use super::text::{body_text, label_text};
use crate::frontend::{
    data::{
        ComposedBody, Initialization, MessageId, MessagePage, MessageSnapshot, MessageStatus,
        OwnedMessageBody, PageCursor, PersonId, PersonSnapshot, RoomId, RoomKind, RoomSnapshot,
        TeamId, TeamSnapshot,
    },
    envelopes::{
        FrontendError, FrontendIntent, FrontendResult, PartialReason, ReadOutcome, RequestEnvelope,
        ResultEnvelope, WriteOutcome,
    },
    events::{ConnectionLayer, ConnectionState, LifecycleEvent, SyncProgress},
    threads::{ThreadPage, ThreadParent},
};
use std::collections::HashSet;
use webex_bridge::{client::ClientError, dto};

type Reply<T> = Result<T, ClientError>;
type Project<T> = Result<T, ProjectionError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProjectionError {
    Identity,
    Metadata,
    Timestamp,
    BodyKind,
    Page,
    ResponseKind,
}

/// Raw typed results from existing client methods. Contains no client/session
/// handle and deliberately has no Debug implementation that could log content.
pub(crate) enum ClientResponse {
    Initialize(Reply<dto::Person>),
    WhoAmI(Reply<dto::Person>),
    ListAllRooms(Reply<Vec<dto::Room>>),
    UpdateRoom(Reply<dto::Room>),
    UpdateTeam(Reply<dto::Team>),
    UpdatePerson(Reply<dto::PersonRead>),
    ListMessagesInRoom(Reply<dto::MessagePage>),
    UpdateMessage(Reply<dto::MessageRead>),
    UpdateChildrenMessages(Reply<dto::ThreadChildren>),
    SendMessage(Reply<dto::Message>),
    EditMessage(Reply<dto::Message>),
    DeleteMessage(Reply<()>),
    LeaveRoom(Reply<()>),
}

pub(super) fn valid_id(value: &str) -> Project<()> {
    if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
        Err(ProjectionError::Identity)
    } else {
        Ok(())
    }
}

fn same_id(actual: &str, expected: &str) -> Project<()> {
    valid_id(actual)?;
    if actual == expected {
        Ok(())
    } else {
        Err(ProjectionError::Identity)
    }
}

fn timestamp(value: &Option<String>) -> Project<()> {
    if value
        .as_ref()
        .is_some_and(|value| chrono::DateTime::parse_from_rfc3339(value).is_err())
    {
        Err(ProjectionError::Timestamp)
    } else {
        Ok(())
    }
}

fn optional_id(value: &Option<String>) -> Project<()> {
    if let Some(value) = value {
        valid_id(value)?;
    }
    Ok(())
}

fn person(value: dto::Person) -> Project<PersonSnapshot> {
    valid_id(&value.id)?;
    Ok(PersonSnapshot {
        id: PersonId::new(value.id),
        display_name: value.display_name.map(label_text),
        emails: value
            .emails
            .map(|emails| emails.into_iter().map(label_text).collect()),
    })
}

fn team(value: dto::Team, expected: &TeamId) -> Project<TeamSnapshot> {
    same_id(&value.id, expected.as_str())?;
    Ok(TeamSnapshot {
        id: TeamId::new(value.id),
        name: value.name.map(label_text),
    })
}

fn room(value: dto::Room) -> Project<RoomSnapshot> {
    valid_id(&value.id)?;
    optional_id(&value.team_id)?;
    timestamp(&value.last_activity)?;
    Ok(RoomSnapshot {
        id: RoomId::new(value.id),
        title: value.title.map(label_text),
        kind: match value.room_type.as_str() {
            "direct" => RoomKind::Direct,
            "group" => RoomKind::Group,
            _ => RoomKind::Unknown,
        },
        team_id: value.team_id.map(TeamId::new),
        last_activity: value.last_activity,
        is_member: value.is_member,
        is_locked: value.is_locked,
    })
}

fn message(value: dto::Message, expected_room: &RoomId) -> Project<MessageSnapshot> {
    valid_id(&value.id)?;
    same_id(&value.room_id, expected_room.as_str())?;
    optional_id(&value.parent_id)?;
    optional_id(&value.sender_id)?;
    timestamp(&value.created_at)?;
    timestamp(&value.updated_at)?;
    let revision = u64::try_from(value.revision).map_err(|_| ProjectionError::Metadata)?;
    let status = match value.status.as_str() {
        "active" => MessageStatus::Active,
        "deleted" => MessageStatus::Deleted,
        _ => return Err(ProjectionError::Metadata),
    };
    let body = if status == MessageStatus::Deleted {
        None
    } else {
        match value.body {
            Some(body) => Some(match body.kind.as_str() {
                "plainText" => OwnedMessageBody::Forms {
                    text: Some(body_text(&body.content)),
                    markdown: None,
                    html: None,
                },
                "markdown" => OwnedMessageBody::Forms {
                    text: None,
                    markdown: Some(body_text(&body.content)),
                    html: None,
                },
                "legacyUnknown" => OwnedMessageBody::LegacyUnknown(body_text(&body.content)),
                _ => return Err(ProjectionError::BodyKind),
            }),
            // Compatibility metadata cannot prove its historic source format.
            None => value
                .markdown
                .map(|text| OwnedMessageBody::LegacyUnknown(body_text(&text))),
        }
    };
    Ok(MessageSnapshot {
        id: MessageId::new(value.id),
        room_id: RoomId::new(value.room_id),
        revision: Some(revision),
        parent_id: value.parent_id.map(MessageId::new),
        sender_id: value.sender_id.map(PersonId::new),
        sender_email: None,
        created_at: value.created_at,
        updated_at: value.updated_at,
        status,
        body,
        attachment_count: if status == MessageStatus::Active {
            Some(usize::try_from(value.attachment_count).map_err(|_| ProjectionError::Metadata)?)
        } else {
            None
        },
    })
}

pub(crate) fn composed_body(body: &ComposedBody) -> dto::MessageBodyInput {
    let (kind, content) = match body {
        ComposedBody::PlainText(content) => ("plainText", content),
        ComposedBody::Markdown(content) => ("markdown", content),
    };
    // User-composed outbound text remains exact. Incoming display sanitization
    // must not silently change a deliberate user write.
    dto::MessageBodyInput {
        kind: kind.into(),
        content: content.clone(),
    }
}

fn projection_error() -> FrontendError {
    FrontendError {
        code: "invalid_response".into(),
        message: "Bridge response did not match the expected resource metadata".into(),
    }
}

fn unavailable(state: &str) -> FrontendError {
    FrontendError {
        code: state.into(),
        message: "Resource is not presently available; retain its placeholder".into(),
    }
}

fn client_error(error: &ClientError) -> FrontendError {
    let code = match error {
        ClientError::Api { code, .. }
            if !code.is_empty()
                && code.len() <= 64
                && code
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') =>
        {
            code.as_str()
        }
        ClientError::Transport(_) => "bridge_transport",
        ClientError::InvalidResponse(_) => "invalid_response",
        _ => "bridge_error",
    };
    // Never echo raw transport/decoder errors or daemon error text into logs/UI.
    FrontendError {
        code: code.into(),
        message: "Bridge operation outcome requires review".into(),
    }
}

fn read<T, U>(reply: Reply<T>, project: impl FnOnce(T) -> Project<U>) -> ReadOutcome<U> {
    match reply {
        Ok(value) => match project(value) {
            Ok(value) => ReadOutcome::Success(value),
            Err(_) => ReadOutcome::Rejected(projection_error()),
        },
        Err(error) => ReadOutcome::Rejected(client_error(&error)),
    }
}

/// Narrow canonical write projection shared by the standalone mutation seam.
pub(crate) fn project_write_message(
    value: dto::Message,
    expected_room: &RoomId,
) -> Project<MessageSnapshot> {
    message(value, expected_room)
}

/// Classify every typed write failure without reflecting backend detail.
pub(crate) fn write_failure<T>(error: ClientError) -> WriteOutcome<T> {
    let rejected = matches!(&error, ClientError::Api { status, .. } if status.is_client_error())
        && !error.is_delivery_unknown();
    if rejected {
        WriteOutcome::Rejected(client_error(&error))
    } else {
        WriteOutcome::DeliveryUnknown(client_error(&error))
    }
}

fn write<T, U>(reply: Reply<T>, project: impl FnOnce(T) -> Project<U>) -> WriteOutcome<U> {
    match reply {
        Ok(value) => match project(value) {
            Ok(value) => WriteOutcome::Success(value),
            Err(_) => WriteOutcome::DeliveryUnknown(projection_error()),
        },
        Err(error) => write_failure(error),
    }
}

fn point_person(reply: Reply<dto::PersonRead>, expected: &PersonId) -> ReadOutcome<PersonSnapshot> {
    match reply {
        Err(error) => ReadOutcome::Rejected(client_error(&error)),
        Ok(value) => {
            if same_id(&value.id, expected.as_str()).is_err() {
                return ReadOutcome::Rejected(projection_error());
            }
            match (value.state.as_str(), value.person) {
                ("mirrored" | "hydrated", Some(value)) if value.id == expected.as_str() => {
                    read(Ok(value), person)
                }
                (state @ ("not_found" | "offline" | "unavailable"), None) => {
                    ReadOutcome::Rejected(unavailable(state))
                }
                _ => ReadOutcome::Rejected(projection_error()),
            }
        }
    }
}

fn point_message(
    reply: Reply<dto::MessageRead>,
    expected: &MessageId,
    room_id: &RoomId,
) -> ReadOutcome<MessageSnapshot> {
    match reply {
        Err(error) => ReadOutcome::Rejected(client_error(&error)),
        Ok(value) => {
            if same_id(&value.id, expected.as_str()).is_err() {
                return ReadOutcome::Rejected(projection_error());
            }
            match (value.state.as_str(), value.message) {
                ("mirrored" | "hydrated", Some(value))
                    if value.id == expected.as_str() && value.status == "active" =>
                {
                    read(Ok(value), |m| message(m, room_id))
                }
                ("deleted", Some(value))
                    if value.id == expected.as_str() && value.status == "deleted" =>
                {
                    read(Ok(value), |m| message(m, room_id))
                }
                (state @ ("not_found" | "offline" | "unavailable"), None) => {
                    ReadOutcome::Rejected(unavailable(state))
                }
                _ => ReadOutcome::Rejected(projection_error()),
            }
        }
    }
}

fn partial_reason(complete: bool, reason: Option<&str>) -> Project<Option<PartialReason>> {
    match (complete, reason) {
        (true, None) => Ok(None),
        (false, Some(reason)) => Ok(Some(match reason {
            "offline" => PartialReason::Offline,
            "not_mirrored" => PartialReason::NotMirrored,
            "parent_missing" | "parent_unavailable" => PartialReason::ParentUnavailable,
            "page_limit" => PartialReason::MorePages,
            "remote_unavailable" => PartialReason::RemoteUnavailable,
            "resource_mismatch" => PartialReason::ResourceMismatch,
            _ => return Err(ProjectionError::Page),
        })),
        _ => Err(ProjectionError::Page),
    }
}

fn messages_page(
    values: Vec<dto::Message>,
    next: Option<String>,
    complete: bool,
    room_id: &RoomId,
    max: u32,
) -> Project<MessagePage> {
    if !(1..=100).contains(&max) || values.len() > max as usize || (complete && next.is_some()) {
        return Err(ProjectionError::Page);
    }
    if let Some(next) = &next {
        valid_id(next)?;
        if values.last().map(|m| m.id.as_str()) != Some(next.as_str()) {
            return Err(ProjectionError::Page);
        }
    }
    let mut ids = HashSet::new();
    let messages = values
        .into_iter()
        .map(|m| {
            if !ids.insert(m.id.clone()) {
                return Err(ProjectionError::Page);
            }
            message(m, room_id)
        })
        .collect::<Project<Vec<_>>>()?;
    Ok(MessagePage {
        room_id: room_id.clone(),
        messages,
        next_cursor: next.map(PageCursor::new),
    })
}

fn page(reply: Reply<dto::MessagePage>, room_id: &RoomId, max: u32) -> ReadOutcome<MessagePage> {
    match reply {
        Err(error) => ReadOutcome::Rejected(client_error(&error)),
        Ok(value) => {
            match partial_reason(value.complete, value.partial_reason.as_deref()).and_then(
                |reason| {
                    messages_page(
                        value.messages,
                        value.next_before,
                        value.complete,
                        room_id,
                        max,
                    )
                    .map(|page| (reason, page))
                },
            ) {
                Ok((Some(reason), value)) => ReadOutcome::Partial { value, reason },
                Ok((None, value)) => ReadOutcome::Success(value),
                Err(_) => ReadOutcome::Rejected(projection_error()),
            }
        }
    }
}

fn thread_parent(
    value: dto::ThreadParent,
    expected: &MessageId,
    room_id: &RoomId,
) -> Project<ThreadParent> {
    same_id(&value.id, expected.as_str())?;
    match (value.state.as_str(), value.message) {
        ("active", Some(value)) if value.id == expected.as_str() && value.status == "active" => {
            Ok(ThreadParent::Active(message(value, room_id)?))
        }
        ("deleted", Some(value)) if value.id == expected.as_str() && value.status == "deleted" => {
            Ok(ThreadParent::Deleted(message(value, room_id)?))
        }
        ("missing", None) => Ok(ThreadParent::Missing(expected.clone())),
        ("unavailable", None) => Ok(ThreadParent::Unavailable(expected.clone())),
        _ => Err(ProjectionError::Metadata),
    }
}

fn children(
    reply: Reply<dto::ThreadChildren>,
    expected: &MessageId,
    room_id: &RoomId,
    max: u32,
) -> ReadOutcome<ThreadPage> {
    match reply {
        Err(error) => ReadOutcome::Rejected(client_error(&error)),
        Ok(value) => {
            let project = || -> Project<(Option<PartialReason>, ThreadPage)> {
                let reason = partial_reason(value.complete, value.partial_reason.as_deref())?;
                let parent = thread_parent(value.parent, expected, room_id)?;
                let replies = messages_page(
                    value.children,
                    value.next_before,
                    value.complete,
                    room_id,
                    max,
                )?;
                let page = ThreadPage::new(expected, room_id, parent, replies)
                    .map_err(|_| ProjectionError::Identity)?;
                Ok((reason, page))
            };
            match project() {
                Ok((Some(reason), value)) => ReadOutcome::Partial { value, reason },
                Ok((None, value)) => ReadOutcome::Success(value),
                Err(_) => ReadOutcome::Rejected(projection_error()),
            }
        }
    }
}

/// Covers all 13 operations, but consumes *results*, never dispatches an intent.
/// Wrong response kinds are local correlation errors, not definitive write failure.
pub(crate) fn project_response(
    request: RequestEnvelope,
    response: ClientResponse,
) -> Project<ResultEnvelope> {
    let result = match (&request.intent, response) {
        (FrontendIntent::Initialize, ClientResponse::Initialize(Err(error)))
            if error.is_identity_not_ready() =>
        {
            FrontendResult::Initialize(ReadOutcome::Partial {
                value: Initialization { account: None },
                reason: PartialReason::IdentityUnavailable,
            })
        }
        (FrontendIntent::Initialize, ClientResponse::Initialize(reply)) => {
            FrontendResult::Initialize(read(reply, |p| {
                Ok(Initialization {
                    account: Some(person(p)?),
                })
            }))
        }
        (FrontendIntent::WhoAmI, ClientResponse::WhoAmI(reply)) => {
            FrontendResult::WhoAmI(read(reply, person))
        }
        (FrontendIntent::ListAllRooms, ClientResponse::ListAllRooms(reply)) => {
            FrontendResult::ListAllRooms(read(reply, |rooms| rooms.into_iter().map(room).collect()))
        }
        (FrontendIntent::UpdateRoom(id), ClientResponse::UpdateRoom(reply)) => {
            FrontendResult::UpdateRoom(read(reply, |r| {
                same_id(&r.id, id.as_str())?;
                room(r)
            }))
        }
        (FrontendIntent::UpdateTeam(id), ClientResponse::UpdateTeam(reply)) => {
            FrontendResult::UpdateTeam(read(reply, |t| team(t, id)))
        }
        (FrontendIntent::UpdatePerson(id), ClientResponse::UpdatePerson(reply)) => {
            FrontendResult::UpdatePerson(point_person(reply, id))
        }
        (
            FrontendIntent::ListMessagesInRoom {
                room_id,
                page: query,
            },
            ClientResponse::ListMessagesInRoom(reply),
        ) => FrontendResult::ListMessagesInRoom(page(reply, room_id, query.max)),
        (
            FrontendIntent::UpdateMessage {
                message_id,
                room_id,
            },
            ClientResponse::UpdateMessage(reply),
        ) => FrontendResult::UpdateMessage(point_message(reply, message_id, room_id)),
        (
            FrontendIntent::UpdateChildrenMessages {
                parent_id,
                room_id,
                page,
            },
            ClientResponse::UpdateChildrenMessages(reply),
        ) => FrontendResult::UpdateChildrenMessages(children(reply, parent_id, room_id, page.max)),
        (
            FrontendIntent::SendMessage {
                room_id, parent_id, ..
            },
            ClientResponse::SendMessage(reply),
        ) => FrontendResult::SendMessage(write(reply, |m| {
            if m.parent_id.as_deref() != parent_id.as_ref().map(MessageId::as_str) {
                return Err(ProjectionError::Identity);
            }
            message(m, room_id)
        })),
        (
            FrontendIntent::EditMessage {
                message_id,
                room_id,
                ..
            },
            ClientResponse::EditMessage(reply),
        ) => FrontendResult::EditMessage(write(reply, |m| {
            same_id(&m.id, message_id.as_str())?;
            message(m, room_id)
        })),
        (
            FrontendIntent::DeleteMessage { message_id, .. },
            ClientResponse::DeleteMessage(reply),
        ) => FrontendResult::DeleteMessage(write(reply, |()| Ok(message_id.clone()))),
        (FrontendIntent::LeaveRoom(room_id), ClientResponse::LeaveRoom(reply)) => {
            FrontendResult::LeaveRoom(write(reply, |()| Ok(room_id.clone())))
        }
        _ => return Err(ProjectionError::ResponseKind),
    };
    request
        .complete(result)
        .map_err(|_| ProjectionError::ResponseKind)
}

pub(super) fn connection(value: &str) -> Project<ConnectionState> {
    match value {
        "connected" => Ok(ConnectionState::Connected),
        "disconnected" => Ok(ConnectionState::Disconnected),
        "auth_required" => Ok(ConnectionState::AuthRequired),
        _ => Err(ProjectionError::Metadata),
    }
}

pub(crate) fn project_status(value: dto::Status) -> Project<Vec<LifecycleEvent>> {
    if value.api_version != 1 {
        return Err(ProjectionError::Metadata);
    }
    let count = |value| {
        u64::try_from(value)
            .map(Some)
            .map_err(|_| ProjectionError::Metadata)
    };
    Ok(vec![
        LifecycleEvent::Connection {
            layer: ConnectionLayer::UpstreamWebex,
            state: connection(&value.connection)?,
        },
        LifecycleEvent::Sync(SyncProgress {
            rooms: count(value.sync.rooms)?,
            revisions: count(value.sync.revisions)?,
            initial_pending: count(value.sync.initial_pending)?,
            gaps: count(value.sync.gaps)?,
            failures: count(value.sync.failures)?,
        }),
    ])
}
