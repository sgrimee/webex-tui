//! Real presentation snapshot built over the ticket-fenced read facade.
//!
//! A snapshot is the cached account identity plus the full room list. Message
//! pages are not part of it: the App glue requests the newest page of each
//! loaded room after installing the snapshot. Uses the same validated read
//! dispatch and projection as ordinary reads; no logging.

use super::{
    read_dispatch::dispatch_read,
    runtime::SnapshotSource,
    session::{SessionBackend, SessionFailure},
};
use crate::frontend::{
    data::{PersonSnapshot, RoomSnapshot},
    envelopes::{FrontendIntent, FrontendResult, ReadOutcome, RequestSequence},
};

/// No Debug: room titles and names are presentation data.
pub(crate) struct BridgeSnapshot {
    /// `None` when the daemon has no cached identity yet (first-run offline).
    pub(crate) account: Option<PersonSnapshot>,
    pub(crate) rooms: Vec<RoomSnapshot>,
}

#[derive(Default)]
pub(crate) struct BridgeSnapshotSource {
    sequence: RequestSequence,
}

impl<B: SessionBackend> SnapshotSource<B> for BridgeSnapshotSource {
    type Snapshot = BridgeSnapshot;

    /// Any rejected or malformed read fails the snapshot; the runtime then
    /// ends the subscription and reconnects with backoff. A partial identity
    /// (not yet known) or partial room list is still installed honestly.
    async fn build(
        &mut self,
        reader: &B::SnapshotReader,
    ) -> Result<BridgeSnapshot, SessionFailure> {
        let identity = self
            .sequence
            .begin(FrontendIntent::Initialize, None)
            .map_err(|_| SessionFailure::CounterExhausted)?;
        let account = match dispatch_read(reader, identity)
            .await
            .map_err(|_| SessionFailure::InvalidResponse)?
            .result
        {
            FrontendResult::Initialize(
                ReadOutcome::Success(init) | ReadOutcome::Partial { value: init, .. },
            ) => init.account,
            _ => return Err(SessionFailure::InvalidResponse),
        };
        let rooms = self
            .sequence
            .begin(FrontendIntent::ListAllRooms, None)
            .map_err(|_| SessionFailure::CounterExhausted)?;
        let rooms = match dispatch_read(reader, rooms)
            .await
            .map_err(|_| SessionFailure::InvalidResponse)?
            .result
        {
            FrontendResult::ListAllRooms(
                ReadOutcome::Success(rooms) | ReadOutcome::Partial { value: rooms, .. },
            ) => rooms,
            _ => return Err(SessionFailure::InvalidResponse),
        };
        Ok(BridgeSnapshot { account, rooms })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{read_dispatch::UiReadBackend, session::BridgeSessionBackend};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use webex_bridge::{client::ClientError, dto};

    type Reply<T> = Result<T, ClientError>;

    #[derive(Default)]
    struct Reader {
        identity_missing: bool,
        rooms_fail: bool,
        calls: Arc<AtomicUsize>,
    }
    fn api(status: u16, code: &str) -> ClientError {
        ClientError::Api {
            status: status.try_into().unwrap(),
            code: code.into(),
            message: "synthetic".into(),
        }
    }
    impl UiReadBackend for Reader {
        async fn me(&self) -> Reply<dto::Person> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.identity_missing {
                return Err(api(409, "identity_not_ready"));
            }
            Ok(dto::Person {
                id: "me".into(),
                display_name: Some("Me".into()),
                emails: None,
            })
        }
        async fn rooms(&self) -> Reply<Vec<dto::Room>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.rooms_fail {
                return Err(api(503, "bridge_unavailable"));
            }
            Ok(vec![dto::Room {
                id: "room/a?opaque".into(),
                title: Some("Room".into()),
                room_type: "group".into(),
                team_id: None,
                last_activity: None,
                is_member: true,
                is_locked: None,
            }])
        }
        async fn room(&self, _: &str) -> Reply<dto::Room> {
            unreachable!()
        }
        async fn team(&self, _: &str) -> Reply<dto::Team> {
            unreachable!()
        }
        async fn person_read(&self, _: &str) -> Reply<dto::PersonRead> {
            unreachable!()
        }
        async fn room_messages_page(
            &self,
            _: &str,
            _: Option<&str>,
            _: Option<u32>,
        ) -> Reply<dto::MessagePage> {
            unreachable!("snapshots never load message pages")
        }
        async fn message_read(&self, _: &str) -> Reply<dto::MessageRead> {
            unreachable!()
        }
        async fn thread_children(
            &self,
            _: &str,
            _: Option<&str>,
            _: Option<u32>,
        ) -> Reply<dto::ThreadChildren> {
            unreachable!()
        }
    }

    struct Backend;
    impl SessionBackend for Backend {
        type Client = ();
        type Stream = ();
        type Writer = ();
        type SnapshotReader = Reader;
        async fn issue(&mut self, _: &str) -> Result<(), SessionFailure> {
            Ok(())
        }
        async fn subscribe(&mut self, _: &()) -> Result<(), SessionFailure> {
            Ok(())
        }
        async fn continuity(
            &mut self,
            _: &(),
        ) -> Result<crate::frontend::continuity::ContinuityBoundary, SessionFailure> {
            unreachable!()
        }
        async fn renew(&mut self, _: &()) -> Result<u64, crate::bridge::session::RenewFailure> {
            // Not exercised here: behaves like a daemon at the 12 h cap.
            Err(crate::bridge::session::RenewFailure::CapReached)
        }
        fn lease_ttl(_: &()) -> u64 {
            1
        }
        fn writer(_: &()) {}
        fn snapshot_reader(_: &()) -> Reader {
            Reader::default()
        }
    }

    async fn build(reader: &Reader) -> Result<BridgeSnapshot, SessionFailure> {
        <BridgeSnapshotSource as SnapshotSource<Backend>>::build(
            &mut BridgeSnapshotSource::default(),
            reader,
        )
        .await
    }

    #[tokio::test]
    async fn snapshot_holds_identity_and_rooms_without_message_pages() {
        let reader = Reader::default();
        let snapshot = build(&reader).await.unwrap();
        assert_eq!(snapshot.account.unwrap().id.as_str(), "me");
        assert_eq!(snapshot.rooms.len(), 1);
        assert_eq!(snapshot.rooms[0].id.as_str(), "room/a?opaque");
        assert_eq!(snapshot.rooms[0].last_activity, None, "never fabricated");
        assert_eq!(reader.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn missing_first_run_identity_is_partial_and_room_failure_fails() {
        let missing = Reader {
            identity_missing: true,
            ..Reader::default()
        };
        let snapshot = build(&missing).await.unwrap();
        assert!(snapshot.account.is_none());
        assert_eq!(snapshot.rooms.len(), 1);
        let failing = Reader {
            rooms_fail: true,
            ..Reader::default()
        };
        assert!(matches!(
            build(&failing).await,
            Err(SessionFailure::InvalidResponse)
        ));
    }

    #[test]
    fn real_bridge_snapshot_source_type_checks() {
        fn source<P: SnapshotSource<BridgeSessionBackend, Snapshot = BridgeSnapshot>>(_: &P) {}
        source(&BridgeSnapshotSource::default());
    }
}
