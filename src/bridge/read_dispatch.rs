//! Read-only paired-UI dispatch: one typed read intent → one client read →
//! one correlated, projected result. No retry, cache, scheduling or logging.
//!
//! The read facade is borrowed from the active session for one call only; it
//! exposes no bearer, write or event method and is never returned to a host.

use super::{
    projection::{project_response, ClientResponse},
    session::UiSnapshotReader,
};
use crate::frontend::{
    data::PageCursor,
    envelopes::{FrontendIntent, PageRequest, RequestEnvelope, ResultEnvelope},
    write_rules::valid_opaque_id,
};
use std::future::Future;
use webex_bridge::{client::ClientError, dto};

type Reply<T> = Result<T, ClientError>;

/// Paired-UI read methods only. Implementations must not expose mutation,
/// event-subscription or bearer access through any other method.
pub(crate) trait UiReadBackend: Send + Sync {
    fn me(&self) -> impl Future<Output = Reply<dto::Person>> + Send;
    fn rooms(&self) -> impl Future<Output = Reply<Vec<dto::Room>>> + Send;
    fn room(&self, room_id: &str) -> impl Future<Output = Reply<dto::Room>> + Send;
    fn team(&self, team_id: &str) -> impl Future<Output = Reply<dto::Team>> + Send;
    fn person_read(&self, person_id: &str) -> impl Future<Output = Reply<dto::PersonRead>> + Send;
    fn room_messages_page(
        &self,
        room_id: &str,
        before: Option<&str>,
        max: Option<u32>,
    ) -> impl Future<Output = Reply<dto::MessagePage>> + Send;
    fn message_read(
        &self,
        message_id: &str,
    ) -> impl Future<Output = Reply<dto::MessageRead>> + Send;
    fn thread_children(
        &self,
        parent_id: &str,
        before: Option<&str>,
        max: Option<u32>,
    ) -> impl Future<Output = Reply<dto::ThreadChildren>> + Send;
}

impl UiReadBackend for UiSnapshotReader<'_> {
    fn me(&self) -> impl Future<Output = Reply<dto::Person>> + Send {
        UiSnapshotReader::me(self)
    }
    fn rooms(&self) -> impl Future<Output = Reply<Vec<dto::Room>>> + Send {
        UiSnapshotReader::rooms(self)
    }
    fn room(&self, room_id: &str) -> impl Future<Output = Reply<dto::Room>> + Send {
        UiSnapshotReader::room(self, room_id)
    }
    fn team(&self, team_id: &str) -> impl Future<Output = Reply<dto::Team>> + Send {
        UiSnapshotReader::team(self, team_id)
    }
    fn person_read(&self, person_id: &str) -> impl Future<Output = Reply<dto::PersonRead>> + Send {
        UiSnapshotReader::person_read(self, person_id)
    }
    fn room_messages_page(
        &self,
        room_id: &str,
        before: Option<&str>,
        max: Option<u32>,
    ) -> impl Future<Output = Reply<dto::MessagePage>> + Send {
        UiSnapshotReader::room_messages_page(self, room_id, before, max)
    }
    fn message_read(
        &self,
        message_id: &str,
    ) -> impl Future<Output = Reply<dto::MessageRead>> + Send {
        UiSnapshotReader::message_read(self, message_id)
    }
    fn thread_children(
        &self,
        parent_id: &str,
        before: Option<&str>,
        max: Option<u32>,
    ) -> impl Future<Output = Reply<dto::ThreadChildren>> + Send {
        UiSnapshotReader::thread_children(self, parent_id, before, max)
    }
}

/// Test-only inert reader lets lifecycle fakes satisfy the bound without I/O.
#[cfg(test)]
impl UiReadBackend for () {
    async fn me(&self) -> Reply<dto::Person> {
        Err(inert())
    }
    async fn rooms(&self) -> Reply<Vec<dto::Room>> {
        Err(inert())
    }
    async fn room(&self, _: &str) -> Reply<dto::Room> {
        Err(inert())
    }
    async fn team(&self, _: &str) -> Reply<dto::Team> {
        Err(inert())
    }
    async fn person_read(&self, _: &str) -> Reply<dto::PersonRead> {
        Err(inert())
    }
    async fn room_messages_page(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<u32>,
    ) -> Reply<dto::MessagePage> {
        Err(inert())
    }
    async fn message_read(&self, _: &str) -> Reply<dto::MessageRead> {
        Err(inert())
    }
    async fn thread_children(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<u32>,
    ) -> Reply<dto::ThreadChildren> {
        Err(inert())
    }
}

#[cfg(test)]
fn inert() -> ClientError {
    ClientError::Transport(std::io::Error::other("inert reader").into())
}

/// Metadata-only refusal before any read I/O.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReadDispatchError {
    /// Write intent, or a context/intent operation mismatch.
    WrongIntent,
    /// Invalid opaque ID/cursor or page size outside the UI API's 1..=100.
    InvalidRequest,
}

/// Validate locally, perform exactly one read and correlate its projection to
/// the original request. Backend errors become typed `ReadOutcome::Rejected`
/// inside the result; only the pre-I/O checks above return `Err`.
pub(crate) async fn dispatch_read<R: UiReadBackend>(
    reader: &R,
    request: RequestEnvelope,
) -> Result<ResultEnvelope, ReadDispatchError> {
    validate(&request)?;
    let response = match &request.intent {
        FrontendIntent::Initialize => ClientResponse::Initialize(reader.me().await),
        FrontendIntent::WhoAmI => ClientResponse::WhoAmI(reader.me().await),
        FrontendIntent::ListAllRooms => ClientResponse::ListAllRooms(reader.rooms().await),
        FrontendIntent::UpdateRoom(id) => {
            ClientResponse::UpdateRoom(reader.room(id.as_str()).await)
        }
        FrontendIntent::UpdateTeam(id) => {
            ClientResponse::UpdateTeam(reader.team(id.as_str()).await)
        }
        FrontendIntent::UpdatePerson(id) => {
            ClientResponse::UpdatePerson(reader.person_read(id.as_str()).await)
        }
        FrontendIntent::ListMessagesInRoom { room_id, page } => ClientResponse::ListMessagesInRoom(
            reader
                .room_messages_page(room_id.as_str(), cursor(page), Some(page.max))
                .await,
        ),
        FrontendIntent::UpdateMessage { message_id, .. } => {
            ClientResponse::UpdateMessage(reader.message_read(message_id.as_str()).await)
        }
        FrontendIntent::UpdateChildrenMessages {
            parent_id, page, ..
        } => ClientResponse::UpdateChildrenMessages(
            reader
                .thread_children(parent_id.as_str(), cursor(page), Some(page.max))
                .await,
        ),
        _ => return Err(ReadDispatchError::WrongIntent),
    };
    // Same operation is guaranteed above, so a kind mismatch is unreachable;
    // treat it conservatively as a local correlation error rather than panic.
    project_response(request, response).map_err(|_| ReadDispatchError::WrongIntent)
}

fn cursor(page: &PageRequest) -> Option<&str> {
    page.before.as_ref().map(PageCursor::as_str)
}

fn validate(request: &RequestEnvelope) -> Result<(), ReadDispatchError> {
    let operation = request.intent.operation();
    if operation.is_write() || request.context.operation != operation {
        return Err(ReadDispatchError::WrongIntent);
    }
    if request.context.room_view.as_ref().is_some_and(|view| {
        request.intent.room_id().map(|id| id.as_str()) != Some(view.room_id.as_str())
    }) {
        return Err(ReadDispatchError::InvalidRequest);
    }
    let ok = |id: &str| valid_opaque_id(id);
    let page_ok = |page: &PageRequest| {
        (1..=100).contains(&page.max) && page.before.as_ref().is_none_or(|c| ok(c.as_str()))
    };
    let valid = match &request.intent {
        FrontendIntent::Initialize | FrontendIntent::WhoAmI | FrontendIntent::ListAllRooms => true,
        FrontendIntent::UpdateRoom(id) => ok(id.as_str()),
        FrontendIntent::UpdateTeam(id) => ok(id.as_str()),
        FrontendIntent::UpdatePerson(id) => ok(id.as_str()),
        FrontendIntent::ListMessagesInRoom { room_id, page } => {
            ok(room_id.as_str()) && page_ok(page)
        }
        FrontendIntent::UpdateMessage {
            message_id,
            room_id,
        } => ok(message_id.as_str()) && ok(room_id.as_str()),
        FrontendIntent::UpdateChildrenMessages {
            parent_id,
            room_id,
            page,
        } => ok(parent_id.as_str()) && ok(room_id.as_str()) && page_ok(page),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(ReadDispatchError::InvalidRequest)
    }
}

#[cfg(test)]
mod tests;
