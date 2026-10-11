//! Pure legacy-transport projection into frontend-owned snapshots.
//!
//! This keeps `webex` resource types outside the app cache while the old Teams
//! transport remains in place. It performs no network I/O, cache mutation, or
//! bridge session work.

use super::text::{body_text, label_text};
use crate::frontend::data::{
    MessageId, MessageSnapshot, MessageStatus, OwnedMessageBody, PersonId, PersonSnapshot, RoomId,
    RoomKind, RoomSnapshot, TeamId, TeamSnapshot,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LegacyProjectionError {
    MissingIdentity,
}

fn id(value: Option<&str>) -> Result<&str, LegacyProjectionError> {
    value
        .filter(|value| !value.is_empty() && !value.chars().any(char::is_control))
        .ok_or(LegacyProjectionError::MissingIdentity)
}

/// Project a legacy message without retaining its Webex transport representation.
pub(crate) fn message(value: &webex::Message) -> Result<MessageSnapshot, LegacyProjectionError> {
    let message_id = id(value.id.as_deref())?;
    let room_id = id(value.room_id.as_deref())?;
    let body = if value.text.is_some() || value.markdown.is_some() || value.html.is_some() {
        Some(OwnedMessageBody::Forms {
            text: value.text.as_deref().map(body_text),
            markdown: value.markdown.as_deref().map(body_text),
            html: value.html.as_deref().map(body_text),
        })
    } else {
        None
    };
    Ok(MessageSnapshot {
        id: MessageId::new(message_id),
        room_id: RoomId::new(room_id),
        revision: None,
        parent_id: value.parent_id.as_deref().map(MessageId::new),
        sender_id: value.person_id.as_deref().map(PersonId::new),
        sender_email: value.person_email.clone().map(label_text),
        created_at: value.created.clone(),
        updated_at: value.updated.clone(),
        status: MessageStatus::Active,
        body,
        attachment_count: value.files.as_ref().map(Vec::len),
    })
}

/// Rebuild only the legacy editor fields required while its transport remains.
/// The cache never retains this transport value.
pub(crate) fn message_for_editor(value: &MessageSnapshot) -> webex::Message {
    let (text, markdown, html) = match &value.body {
        Some(OwnedMessageBody::Forms {
            text,
            markdown,
            html,
        }) => (text.clone(), markdown.clone(), html.clone()),
        Some(OwnedMessageBody::LegacyUnknown(text)) => (Some(text.clone()), None, None),
        None => (None, None, None),
    };
    webex::Message {
        id: Some(value.id.as_str().to_owned()),
        room_id: Some(value.room_id.as_str().to_owned()),
        parent_id: value.parent_id.as_ref().map(|id| id.as_str().to_owned()),
        person_id: value.sender_id.as_ref().map(|id| id.as_str().to_owned()),
        person_email: value.sender_email.clone(),
        created: value.created_at.clone(),
        updated: value.updated_at.clone(),
        text,
        markdown,
        html,
        ..Default::default()
    }
}

pub(crate) fn person(value: &webex::Person) -> Result<PersonSnapshot, LegacyProjectionError> {
    Ok(PersonSnapshot {
        id: PersonId::new(id(Some(&value.id))?),
        display_name: Some(label_text(value.display_name.clone())),
        emails: Some(value.emails.iter().cloned().map(label_text).collect()),
    })
}

pub(crate) fn room(value: &webex::Room) -> Result<RoomSnapshot, LegacyProjectionError> {
    Ok(RoomSnapshot {
        id: RoomId::new(id(Some(&value.id))?),
        title: value.title.clone().map(label_text),
        kind: match value.room_type.as_str() {
            "direct" => RoomKind::Direct,
            "group" => RoomKind::Group,
            _ => RoomKind::Unknown,
        },
        team_id: value.team_id.as_deref().map(TeamId::new),
        last_activity: Some(value.last_activity.clone()),
        is_member: true,
        is_locked: Some(value.is_locked),
    })
}

pub(crate) fn team(value: &webex::Team) -> Result<TeamSnapshot, LegacyProjectionError> {
    Ok(TeamSnapshot {
        id: TeamId::new(id(Some(&value.id))?),
        name: value.name.clone().map(label_text),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::models::DisplayBody;

    #[test]
    fn message_projection_preserves_body_precedence_and_attachment_presence() {
        let legacy_message = webex::Message {
            id: Some("message-a".into()),
            room_id: Some("room-a".into()),
            text: Some("plain".into()),
            markdown: Some("**markdown**".into()),
            html: Some("<p>html</p>".into()),
            files: Some(vec!["attachment-a".into()]),
            ..Default::default()
        };
        let snapshot = message(&legacy_message).unwrap();
        let view = snapshot.presentation();
        assert_eq!(view.body.display(), DisplayBody::Html("<p>html</p>"));
        assert_eq!(view.body.copy_text().unwrap(), "plain");
        assert_eq!(view.attachment_count, Some(1));
    }

    #[test]
    fn resource_projections_retain_neutral_ids_and_unknown_room_kind() {
        let person = person(&webex::Person {
            id: "person-a".into(),
            display_name: "Person A".into(),
            emails: vec!["person@example.invalid".into()],
            ..Default::default()
        })
        .unwrap();
        let room = room(&webex::Room {
            id: "room-a".into(),
            title: None,
            room_type: "future-kind".into(),
            is_locked: true,
            team_id: None,
            last_activity: "2026-01-02T03:04:05Z".into(),
            creator_id: String::new(),
            created: String::new(),
        })
        .unwrap();
        let team = team(&webex::Team {
            id: "team-a".into(),
            name: Some("Team A".into()),
            created: "2026-01-02T03:04:05Z".into(),
            description: None,
        })
        .unwrap();
        assert_eq!(person.id.as_str(), "person-a");
        assert_eq!(room.kind, RoomKind::Unknown);
        assert_eq!(room.is_locked, Some(true));
        assert_eq!(team.id.as_str(), "team-a");
    }
}
