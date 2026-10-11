use super::*;
use crate::bridge::session::{SessionBackend, SessionClock, SessionPhase};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Clone, Debug, Eq, PartialEq)]
enum Call {
    Status,
    Load,
    Random,
    Persist(String),
    Pair(String),
    Issue(String),
    Subscribe,
}

type Calls = Arc<Mutex<Vec<Call>>>;

struct Store {
    calls: Calls,
    loaded: Result<Option<String>, SecretStoreFailure>,
    persist: Result<(), SecretStoreFailure>,
}

impl UiSecretStore for Store {
    async fn load(&mut self) -> Result<Option<UiSecret>, SecretStoreFailure> {
        self.calls.lock().unwrap().push(Call::Load);
        match &self.loaded {
            Ok(Some(secret)) => UiSecret::from_persisted(secret.clone()).map(Some),
            Ok(None) => Ok(None),
            Err(error) => Err(*error),
        }
    }

    async fn persist(&mut self, secret: &UiSecret) -> Result<(), SecretStoreFailure> {
        self.calls
            .lock()
            .unwrap()
            .push(Call::Persist(secret.as_str().into()));
        self.persist
    }
}

struct Backend {
    calls: Calls,
    version: Result<u32, PairingBackendFailure>,
    pair: Result<(), PairingBackendFailure>,
}

impl PairingBackend for Backend {
    async fn status(&mut self) -> Result<u32, PairingBackendFailure> {
        self.calls.lock().unwrap().push(Call::Status);
        self.version
    }

    async fn pair(&mut self, secret_hash: &str) -> Result<(), PairingBackendFailure> {
        self.calls
            .lock()
            .unwrap()
            .push(Call::Pair(secret_hash.into()));
        self.pair
    }
}

struct Random {
    calls: Calls,
    value: Result<[u8; 32], RandomFailure>,
}

impl RandomSource for Random {
    fn fill_256(&mut self, bytes: &mut [u8; 32]) -> Result<(), RandomFailure> {
        self.calls.lock().unwrap().push(Call::Random);
        *bytes = self.value?;
        Ok(())
    }
}

struct Clock;
impl SessionClock for Clock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

struct Session {
    calls: Calls,
}

impl SessionBackend for Session {
    type Client = ();
    type Stream = ();
    type SnapshotReader<'a> = ();

    async fn issue(&mut self, secret: &str) -> Result<(), SessionFailure> {
        self.calls.lock().unwrap().push(Call::Issue(secret.into()));
        Ok(())
    }

    async fn subscribe(&mut self, _: &()) -> Result<(), SessionFailure> {
        self.calls.lock().unwrap().push(Call::Subscribe);
        Ok(())
    }

    async fn continuity(
        &mut self,
        _: &(),
    ) -> Result<crate::frontend::continuity::ContinuityBoundary, SessionFailure> {
        unreachable!("bootstrap never starts a snapshot")
    }

    fn lease_ttl(_: &()) -> u64 {
        600
    }

    fn snapshot_reader(_: &()) {}
}

fn encoded(bytes: [u8; 32]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn expected_hash(secret: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(secret.as_bytes()))
}

fn bootstrap(
    loaded: Result<Option<String>, SecretStoreFailure>,
    persist: Result<(), SecretStoreFailure>,
    version: Result<u32, PairingBackendFailure>,
    pair: Result<(), PairingBackendFailure>,
    random: Result<[u8; 32], RandomFailure>,
) -> (
    PairingBootstrap<Store, Backend, Random>,
    SessionLifecycle<Session, Clock>,
    Calls,
) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let store = Store {
        calls: Arc::clone(&calls),
        loaded,
        persist,
    };
    let backend = Backend {
        calls: Arc::clone(&calls),
        version,
        pair,
    };
    let random = Random {
        calls: Arc::clone(&calls),
        value: random,
    };
    let session = SessionLifecycle::new(
        Session {
            calls: Arc::clone(&calls),
        },
        Clock,
    );
    (
        PairingBootstrap::new(store, backend, random),
        session,
        calls,
    )
}

#[tokio::test]
async fn missing_secret_status_gates_then_generates_persists_hashes_pairs_and_borrows() {
    let generated = [0xA5; 32];
    let secret = encoded(generated);
    let hash = expected_hash(&secret);
    let (mut bootstrap, mut session, calls) = bootstrap(
        Ok(None),
        Ok(()),
        Ok(SUPPORTED_API_VERSION),
        Ok(()),
        Ok(generated),
    );

    assert!(bootstrap.connect(&mut session).await.is_ok());
    assert_eq!(bootstrap.state(), PairingState::Ready);
    assert_eq!(session.phase(), SessionPhase::AwaitingReady);
    assert_eq!(
        *calls.lock().unwrap(),
        [
            Call::Status,
            Call::Load,
            Call::Random,
            Call::Persist(secret.clone()),
            Call::Pair(hash),
            Call::Issue(secret.clone()),
            Call::Subscribe,
        ]
    );
    // The observable state/configuration surface remains secret-free.
    assert!(!format!("{:?}", bootstrap.state()).contains(&secret));
}

#[tokio::test]
async fn denied_existing_secret_stops_before_pairing_or_session_use() {
    let (mut bootstrap, mut session, calls) = bootstrap(
        Err(SecretStoreFailure::Denied),
        Ok(()),
        Ok(SUPPORTED_API_VERSION),
        Ok(()),
        Ok([1; 32]),
    );

    assert_eq!(
        bootstrap.connect(&mut session).await,
        Err(BootstrapFailure::SecretDenied)
    );
    assert_eq!(
        bootstrap.state(),
        PairingState::Failed(BootstrapFailure::SecretDenied)
    );
    assert_eq!(*calls.lock().unwrap(), [Call::Status, Call::Load]);
    assert_eq!(session.phase(), SessionPhase::Disconnected);
}

#[tokio::test]
async fn missing_secret_persist_failure_never_pairs_or_connects() {
    let (mut bootstrap, mut session, calls) = bootstrap(
        Ok(None),
        Err(SecretStoreFailure::Unavailable),
        Ok(SUPPORTED_API_VERSION),
        Ok(()),
        Ok([2; 32]),
    );

    assert_eq!(
        bootstrap.connect(&mut session).await,
        Err(BootstrapFailure::PersistUnavailable)
    );
    let calls = calls.lock().unwrap();
    assert!(matches!(
        calls.as_slice(),
        [Call::Status, Call::Load, Call::Random, Call::Persist(_)]
    ));
    assert_eq!(session.phase(), SessionPhase::Disconnected);
}

#[tokio::test]
async fn explicit_pair_denial_reuses_secret_without_overwriting_then_fails_closed() {
    let secret = encoded([3; 32]);
    let hash = expected_hash(&secret);
    let (mut bootstrap, mut session, calls) = bootstrap(
        Ok(Some(secret)),
        Ok(()),
        Ok(SUPPORTED_API_VERSION),
        Err(PairingBackendFailure::Denied),
        Ok([4; 32]),
    );

    assert_eq!(
        bootstrap.connect(&mut session).await,
        Err(BootstrapFailure::PairDenied)
    );
    assert_eq!(
        bootstrap.state(),
        PairingState::Failed(BootstrapFailure::PairDenied)
    );
    assert_eq!(
        *calls.lock().unwrap(),
        [Call::Status, Call::Load, Call::Pair(hash)]
    );
    assert_eq!(session.phase(), SessionPhase::Disconnected);
}

#[tokio::test]
async fn version_mismatch_stops_before_secret_read_or_pairing() {
    let (mut bootstrap, mut session, calls) = bootstrap(
        Ok(None),
        Ok(()),
        Ok(SUPPORTED_API_VERSION + 1),
        Ok(()),
        Ok([5; 32]),
    );

    assert_eq!(
        bootstrap.connect(&mut session).await,
        Err(BootstrapFailure::VersionMismatch)
    );
    assert_eq!(
        bootstrap.state(),
        PairingState::Failed(BootstrapFailure::VersionMismatch)
    );
    assert_eq!(*calls.lock().unwrap(), [Call::Status]);
    assert_eq!(session.phase(), SessionPhase::Disconnected);
}
