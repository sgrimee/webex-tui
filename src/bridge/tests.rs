//! Compile-time tests against the actual bridge client DTOs, no daemon/Webex IO.

use super::projection::{
    project_response, project_write_message, write_failure, ClientResponse, ProjectionError,
};
use crate::frontend::{
    data::{MessageId, MessageStatus, OwnedMessageBody, PersonId, RoomId, TeamId},
    envelopes::{
        FrontendIntent, FrontendResult, PageRequest, PartialReason, ReadOutcome, RequestSequence,
        WriteOutcome,
    },
    models::DisplayBody,
    threads::ThreadParent,
};
use webex_bridge::{client::ClientError, dto};

fn room_id() -> RoomId {
    RoomId::new("room-a")
}
fn message_id() -> MessageId {
    MessageId::new("message-a")
}
fn person() -> dto::Person {
    dto::Person {
        id: "person-a".into(),
        display_name: Some("Fixture\x1b[31m name\x1b[0m".into()),
        emails: None,
    }
}
fn room() -> dto::Room {
    dto::Room {
        id: "room-a".into(),
        title: Some("Fixture\nroom".into()),
        room_type: "group".into(),
        team_id: Some("team-a".into()),
        last_activity: None,
        is_member: true,
        is_locked: None,
    }
}
fn message(id: &str, parent: Option<&str>) -> dto::Message {
    dto::Message {
        id: id.into(),
        room_id: "room-a".into(),
        revision: 4,
        parent_id: parent.map(str::to_owned),
        sender_id: Some("person-a".into()),
        created_at: Some("2026-10-09T12:00:00Z".into()),
        updated_at: None,
        status: "active".into(),
        markdown: Some("compatibility body".into()),
        body: Some(dto::MessageBody {
            kind: "plainText".into(),
            content: "**literal**\x1b[31m\x1b[0m".into(),
        }),
        attachment_count: 2,
    }
}
fn query() -> PageRequest {
    PageRequest {
        before: None,
        max: 10,
    }
}
fn api_error(status: u16, code: &str) -> ClientError {
    ClientError::Api {
        status: status.try_into().unwrap(),
        code: code.into(),
        message: "do not echo secret-or-body\x1b[31m".into(),
    }
}
fn project(intent: FrontendIntent, response: ClientResponse) -> FrontendResult {
    let request = RequestSequence::default().begin(intent, None).unwrap();
    let context = request.context.clone();
    let reply = project_response(request, response).unwrap();
    assert_eq!(reply.context, context);
    reply.result
}

#[test]
fn all_eight_read_operations_accept_real_typed_client_results_and_preserve_context() {
    let cases = vec![
        (
            FrontendIntent::Initialize,
            ClientResponse::Initialize(Ok(person())),
        ),
        (
            FrontendIntent::ListAllRooms,
            ClientResponse::ListAllRooms(Ok(vec![room()])),
        ),
        (
            FrontendIntent::UpdateRoom(room_id()),
            ClientResponse::UpdateRoom(Ok(room())),
        ),
        (
            FrontendIntent::UpdateTeam(TeamId::new("team-a")),
            ClientResponse::UpdateTeam(Ok(dto::Team {
                id: "team-a".into(),
                name: None,
            })),
        ),
        (
            FrontendIntent::UpdatePerson(PersonId::new("person-a")),
            ClientResponse::UpdatePerson(Ok(dto::PersonRead {
                id: "person-a".into(),
                state: "hydrated".into(),
                person: Some(person()),
            })),
        ),
        (
            FrontendIntent::ListMessagesInRoom {
                room_id: room_id(),
                page: query(),
            },
            ClientResponse::ListMessagesInRoom(Ok(dto::MessagePage {
                messages: vec![message("message-a", None)],
                next_before: None,
                complete: true,
                partial_reason: None,
            })),
        ),
        (
            FrontendIntent::UpdateMessage {
                room_id: room_id(),
                message_id: message_id(),
            },
            ClientResponse::UpdateMessage(Ok(dto::MessageRead {
                id: "message-a".into(),
                state: "mirrored".into(),
                message: Some(message("message-a", None)),
            })),
        ),
        (
            FrontendIntent::UpdateChildrenMessages {
                room_id: room_id(),
                parent_id: MessageId::new("root-a"),
                page: query(),
            },
            ClientResponse::UpdateChildrenMessages(Ok(dto::ThreadChildren {
                parent: dto::ThreadParent {
                    id: "root-a".into(),
                    state: "missing".into(),
                    message: None,
                },
                children: vec![message("reply-a", Some("root-a"))],
                next_before: None,
                complete: false,
                partial_reason: Some("parent_missing".into()),
            })),
        ),
    ];
    assert_eq!(cases.len(), 8);
    for (intent, response) in cases {
        let expected = intent.operation();
        assert_eq!(project(intent, response).operation(), expected);
    }
}

#[test]
fn body_projection_uses_authoritative_kind_count_and_terminal_safe_text() {
    let result = project(
        FrontendIntent::UpdateMessage {
            room_id: room_id(),
            message_id: message_id(),
        },
        ClientResponse::UpdateMessage(Ok(dto::MessageRead {
            id: "message-a".into(),
            state: "mirrored".into(),
            message: Some(message("message-a", None)),
        })),
    );
    let FrontendResult::UpdateMessage(ReadOutcome::Success(message)) = result else {
        panic!("expected message");
    };
    assert_eq!(
        message.presentation().body.display(),
        DisplayBody::Text("**literal**")
    );
    assert_eq!(message.attachment_count, Some(2));
    assert_eq!(message.revision, Some(4));
    assert_eq!(message.sender_email, None);
}

#[test]
fn historic_body_stays_unknown_and_tombstone_never_retains_body_or_count() {
    let mut old = message("message-a", None);
    old.body = Some(dto::MessageBody {
        kind: "legacyUnknown".into(),
        content: "old body".into(),
    });
    let result = project(
        FrontendIntent::UpdateMessage {
            room_id: room_id(),
            message_id: message_id(),
        },
        ClientResponse::UpdateMessage(Ok(dto::MessageRead {
            id: "message-a".into(),
            state: "mirrored".into(),
            message: Some(old),
        })),
    );
    assert!(
        matches!(result, FrontendResult::UpdateMessage(ReadOutcome::Success(m)) if matches!(m.body,Some(OwnedMessageBody::LegacyUnknown(_))))
    );
    let mut deleted = message("message-a", None);
    deleted.status = "deleted".into();
    let result = project(
        FrontendIntent::UpdateMessage {
            room_id: room_id(),
            message_id: message_id(),
        },
        ClientResponse::UpdateMessage(Ok(dto::MessageRead {
            id: "message-a".into(),
            state: "deleted".into(),
            message: Some(deleted),
        })),
    );
    assert!(
        matches!(result,FrontendResult::UpdateMessage(ReadOutcome::Success(m)) if m.status==MessageStatus::Deleted && m.body.is_none() && m.attachment_count.is_none())
    );
}

#[test]
fn missing_identity_is_partial_startup_not_a_fabricated_person() {
    let result = project(
        FrontendIntent::Initialize,
        ClientResponse::Initialize(Err(api_error(409, "identity_not_ready"))),
    );
    assert!(
        matches!(result, FrontendResult::Initialize(ReadOutcome::Partial { value, reason:PartialReason::IdentityUnavailable }) if value.account.is_none())
    );
}

#[test]
fn labels_lock_unknown_and_opaque_ids_are_not_invented_or_reinterpreted() {
    let result = project(
        FrontendIntent::UpdateRoom(room_id()),
        ClientResponse::UpdateRoom(Ok(room())),
    );
    assert!(
        matches!(result, FrontendResult::UpdateRoom(ReadOutcome::Success(r)) if r.title.as_deref()==Some("Fixture room") && r.is_locked.is_none())
    );
    let result = project(
        FrontendIntent::Initialize,
        ClientResponse::Initialize(Ok(person())),
    );
    assert!(
        matches!(result, FrontendResult::Initialize(ReadOutcome::Success(init)) if init.account.as_ref().is_some_and(|p| p.display_name.as_deref()==Some("Fixture name") && p.emails.is_none()))
    );
    let mut value = room();
    value.id = "opaque/room?x=1".into();
    let result = project(
        FrontendIntent::UpdateRoom(RoomId::new("opaque/room?x=1")),
        ClientResponse::UpdateRoom(Ok(value)),
    );
    assert!(
        matches!(result,FrontendResult::UpdateRoom(ReadOutcome::Success(r)) if r.id.as_str()=="opaque/room?x=1")
    );
}

#[test]
fn point_misses_remain_distinct_and_inconsistent_id_state_payloads_fail_closed() {
    for state in ["offline", "not_found", "unavailable"] {
        let result = project(
            FrontendIntent::UpdatePerson(PersonId::new("person-a")),
            ClientResponse::UpdatePerson(Ok(dto::PersonRead {
                id: "person-a".into(),
                state: state.into(),
                person: None,
            })),
        );
        assert!(
            matches!(result,FrontendResult::UpdatePerson(ReadOutcome::Rejected(error)) if error.code==state)
        );
    }
    let result = project(
        FrontendIntent::UpdatePerson(PersonId::new("person-a")),
        ClientResponse::UpdatePerson(Ok(dto::PersonRead {
            id: "wrong-person".into(),
            state: "hydrated".into(),
            person: Some(person()),
        })),
    );
    assert!(
        matches!(result,FrontendResult::UpdatePerson(ReadOutcome::Rejected(error)) if error.code=="invalid_response")
    );
    let result = project(
        FrontendIntent::UpdateMessage {
            message_id: message_id(),
            room_id: room_id(),
        },
        ClientResponse::UpdateMessage(Ok(dto::MessageRead {
            id: "message-a".into(),
            state: "offline".into(),
            message: Some(message("message-a", None)),
        })),
    );
    assert!(
        matches!(result,FrontendResult::UpdateMessage(ReadOutcome::Rejected(error)) if error.code=="invalid_response")
    );
}

#[test]
fn invalid_message_room_status_revision_timestamp_and_body_kind_are_rejected() {
    let mut samples = vec![];
    let mut m = message("message-a", None);
    m.room_id = "wrong-room".into();
    samples.push(m);
    samples.push(message("wrong-message", None));
    let mut m = message("message-a", None);
    m.status = "surprise".into();
    samples.push(m);
    let mut m = message("message-a", None);
    m.revision = -1;
    samples.push(m);
    let mut m = message("message-a", None);
    m.created_at = Some("invalid date".into());
    samples.push(m);
    let mut m = message("message-a", None);
    m.body.as_mut().unwrap().kind = "unknownKind".into();
    samples.push(m);
    for message in samples {
        let result = project(
            FrontendIntent::UpdateMessage {
                message_id: message_id(),
                room_id: room_id(),
            },
            ClientResponse::UpdateMessage(Ok(dto::MessageRead {
                id: "message-a".into(),
                state: "mirrored".into(),
                message: Some(message),
            })),
        );
        assert!(
            matches!(result,FrontendResult::UpdateMessage(ReadOutcome::Rejected(error)) if error.code=="invalid_response")
        );
    }
}

#[test]
fn partial_pages_keep_data_and_exact_cursor_with_truthful_reason() {
    let result = project(
        FrontendIntent::ListMessagesInRoom {
            room_id: room_id(),
            page: query(),
        },
        ClientResponse::ListMessagesInRoom(Ok(dto::MessagePage {
            messages: vec![message("message-a", None)],
            next_before: Some("message-a".into()),
            complete: false,
            partial_reason: Some("page_limit".into()),
        })),
    );
    assert!(
        matches!(result,FrontendResult::ListMessagesInRoom(ReadOutcome::Partial { value,reason:PartialReason::MorePages }) if value.messages.len()==1 && value.next_cursor.as_ref().unwrap().as_str()=="message-a")
    );
    for (complete, reason, cursor) in [
        (false, None, None),
        (true, Some("offline"), None),
        (false, Some("offline"), Some("foreign-cursor")),
    ] {
        let result = project(
            FrontendIntent::ListMessagesInRoom {
                room_id: room_id(),
                page: query(),
            },
            ClientResponse::ListMessagesInRoom(Ok(dto::MessagePage {
                messages: vec![message("message-a", None)],
                next_before: cursor.map(str::to_owned),
                complete,
                partial_reason: reason.map(str::to_owned),
            })),
        );
        assert!(matches!(
            result,
            FrontendResult::ListMessagesInRoom(ReadOutcome::Rejected(_))
        ));
    }
}

#[test]
fn orphan_and_deleted_roots_preserve_replies_and_scope_mismatches_fail() {
    for state in ["missing", "unavailable", "deleted"] {
        let parent = if state == "deleted" {
            let mut m = message("root-a", None);
            m.status = "deleted".into();
            Some(m)
        } else {
            None
        };
        let result = project(
            FrontendIntent::UpdateChildrenMessages {
                parent_id: MessageId::new("root-a"),
                room_id: room_id(),
                page: query(),
            },
            ClientResponse::UpdateChildrenMessages(Ok(dto::ThreadChildren {
                parent: dto::ThreadParent {
                    id: "root-a".into(),
                    state: state.into(),
                    message: parent,
                },
                children: vec![message("reply-a", Some("root-a"))],
                next_before: None,
                complete: false,
                partial_reason: Some("parent_unavailable".into()),
            })),
        );
        let FrontendResult::UpdateChildrenMessages(ReadOutcome::Partial { value, .. }) = result
        else {
            panic!("expected thread page");
        };
        assert_eq!(value.replies().messages.len(), 1);
        if matches!(value.parent(), ThreadParent::Deleted(_)) {
            assert_eq!(
                value.parent().presentation().unwrap().body.display(),
                DisplayBody::Missing
            );
        }
    }
    let result = project(
        FrontendIntent::UpdateChildrenMessages {
            parent_id: MessageId::new("root-a"),
            room_id: room_id(),
            page: query(),
        },
        ClientResponse::UpdateChildrenMessages(Ok(dto::ThreadChildren {
            parent: dto::ThreadParent {
                id: "root-a".into(),
                state: "missing".into(),
                message: None,
            },
            children: vec![message("reply-a", Some("wrong-root"))],
            next_before: None,
            complete: false,
            partial_reason: Some("parent_missing".into()),
        })),
    );
    assert!(matches!(
        result,
        FrontendResult::UpdateChildrenMessages(ReadOutcome::Rejected(_))
    ));
}

#[test]
fn a_mismatched_response_kind_is_refused() {
    let request = RequestSequence::default()
        .begin(FrontendIntent::ListAllRooms, None)
        .unwrap();
    assert!(matches!(
        project_response(request, ClientResponse::Initialize(Ok(person()))),
        Err(ProjectionError::ResponseKind)
    ));
}

#[test]
fn shared_write_projection_sanitizes_and_rejects_bad_canonical_messages() {
    let mut value = message("message-a", None);
    value.body.as_mut().unwrap().content = "safe\x1b]8;;https://bad\x1b\\body\u{9b}2J".into();
    let projected = project_write_message(value, &room_id()).unwrap();
    assert_eq!(
        projected.presentation().body.copy_text().unwrap(),
        "safebody"
    );
    for mutate in [0, 1, 2, 3] {
        let mut bad = message("message-a", None);
        match mutate {
            0 => bad.id.clear(),
            1 => bad.room_id = "other".into(),
            2 => bad.status = "unknown".into(),
            _ => bad.revision = -1,
        }
        assert!(project_write_message(bad, &room_id()).is_err());
    }
    let mut unknown = message("message-a", None);
    unknown.body = Some(dto::MessageBody {
        kind: "legacyUnknown".into(),
        content: "body".into(),
    });
    assert!(matches!(
        project_write_message(unknown, &room_id()).unwrap().body,
        Some(OwnedMessageBody::LegacyUnknown(_))
    ));
}

#[test]
fn shared_write_failure_classifies_all_client_4xx_and_sanitizes_error() {
    for status in [400, 401, 403, 404] {
        match write_failure::<()>(api_error(status, "other")) {
            WriteOutcome::Rejected(error) => {
                assert_eq!(error.code, "other");
                assert!(!error.message.contains("secret-or-body"));
            }
            _ => panic!("expected rejected"),
        }
    }
    assert!(matches!(
        write_failure::<()>(api_error(202, "delivery_unknown")),
        WriteOutcome::DeliveryUnknown(_)
    ));
    assert!(matches!(
        write_failure::<()>(api_error(500, "server")),
        WriteOutcome::DeliveryUnknown(_)
    ));
}
