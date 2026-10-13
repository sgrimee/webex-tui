#[cfg(target_os = "macos")]
use super::keychain::MacosKeychainSecretStore;
use super::keychain::{KeychainItem, KeychainSecretStore};
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
    type Writer = ();
    type SnapshotReader = ();

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

    async fn renew(&mut self, _: &()) -> Result<u64, crate::bridge::session::RenewFailure> {
        // Not exercised here: behaves like a daemon at the 12 h cap.

        Err(crate::bridge::session::RenewFailure::CapReached)
    }

    fn lease_ttl(_: &()) -> u64 {
        600
    }

    fn writer(_: &()) {}
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

/// REVIEW-13 LOW-4: a malformed stored secret is permanent, distinct from a
/// transient store failure, and never leads to generation or overwrite.
#[tokio::test]
async fn corrupt_existing_secret_is_permanent_and_never_replaced() {
    let (mut bootstrap, mut session, calls) = bootstrap(
        Ok(Some("not-a-canonical-pairing-secret".into())),
        Ok(()),
        Ok(SUPPORTED_API_VERSION),
        Ok(()),
        Ok([1; 32]),
    );
    assert_eq!(
        bootstrap.connect(&mut session).await,
        Err(BootstrapFailure::SecretCorrupt)
    );
    assert_eq!(*calls.lock().unwrap(), [Call::Status, Call::Load]);
    assert_eq!(session.phase(), SessionPhase::Disconnected);
    // Non-canonical encodings and wrong lengths are corrupt, not transient.
    for value in [
        "A".repeat(42),
        "A".repeat(44),
        format!("{}=", "A".repeat(43)),
        "!".repeat(43),
    ] {
        assert!(matches!(
            UiSecret::from_persisted(value),
            Err(SecretStoreFailure::Corrupt)
        ));
    }
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

type Events = Arc<Mutex<Vec<&'static str>>>;

/// These fakes record only control flow. They intentionally have no field that
/// can retain a borrowed secret or pairing hash, so failure/state assertions
/// below exercise the same no-retention contract required of real adapters.
struct OneShotStore {
    secret: Option<UiSecret>,
    events: Events,
}

impl UiSecretStore for OneShotStore {
    async fn load(&mut self) -> Result<Option<UiSecret>, SecretStoreFailure> {
        self.events.lock().unwrap().push("load");
        Ok(self.secret.take())
    }

    async fn persist(&mut self, _: &UiSecret) -> Result<(), SecretStoreFailure> {
        self.events.lock().unwrap().push("persist");
        Ok(())
    }
}

struct NoRetainBackend {
    events: Events,
    pair: Result<(), PairingBackendFailure>,
}

impl PairingBackend for NoRetainBackend {
    async fn status(&mut self) -> Result<u32, PairingBackendFailure> {
        self.events.lock().unwrap().push("status");
        Ok(SUPPORTED_API_VERSION)
    }

    async fn pair(&mut self, _: &str) -> Result<(), PairingBackendFailure> {
        self.events.lock().unwrap().push("pair");
        self.pair
    }
}

struct FixedRandom {
    events: Events,
    byte: u8,
}

impl RandomSource for FixedRandom {
    fn fill_256(&mut self, bytes: &mut [u8; 32]) -> Result<(), RandomFailure> {
        self.events.lock().unwrap().push("random");
        bytes.fill(self.byte);
        Ok(())
    }
}

struct FailingSession {
    events: Events,
}

impl SessionBackend for FailingSession {
    type Client = ();
    type Stream = ();
    type Writer = ();
    type SnapshotReader = ();

    async fn issue(&mut self, _: &str) -> Result<(), SessionFailure> {
        self.events.lock().unwrap().push("issue");
        Err(SessionFailure::Transport)
    }

    async fn subscribe(&mut self, _: &()) -> Result<(), SessionFailure> {
        unreachable!("the issue failure prevents subscription")
    }

    async fn continuity(
        &mut self,
        _: &(),
    ) -> Result<crate::frontend::continuity::ContinuityBoundary, SessionFailure> {
        unreachable!("the issue failure prevents continuity")
    }

    async fn renew(&mut self, _: &()) -> Result<u64, crate::bridge::session::RenewFailure> {
        // Not exercised here: behaves like a daemon at the 12 h cap.

        Err(crate::bridge::session::RenewFailure::CapReached)
    }

    fn lease_ttl(_: &()) -> u64 {
        600
    }

    fn writer(_: &()) {}
    fn snapshot_reader(_: &()) {}
}

fn assert_secret_free<T: std::fmt::Debug>(value: &T, secret: &str) {
    assert!(
        !format!("{value:?}").contains(secret),
        "secret escaped through an observable failure/state value"
    );
}

#[tokio::test]
async fn fresh_secret_is_not_observable_through_pair_failure_or_state() {
    let secret = encoded([0xD1; 32]);
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut bootstrap = PairingBootstrap::new(
        OneShotStore {
            secret: None,
            events: Arc::clone(&events),
        },
        NoRetainBackend {
            events: Arc::clone(&events),
            pair: Err(PairingBackendFailure::Denied),
        },
        FixedRandom {
            events: Arc::clone(&events),
            byte: 0xD1,
        },
    );
    let mut session = SessionLifecycle::new(
        Session {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        Clock,
    );

    let result = bootstrap.connect(&mut session).await;

    assert_eq!(result, Err(BootstrapFailure::PairDenied));
    assert_eq!(
        bootstrap.state(),
        PairingState::Failed(BootstrapFailure::PairDenied)
    );
    assert_secret_free(&result, &secret);
    assert_secret_free(&bootstrap.state(), &secret);
    assert_eq!(
        *events.lock().unwrap(),
        ["status", "load", "random", "persist", "pair"]
    );
}

#[tokio::test]
async fn existing_secret_is_not_observable_through_session_failure_or_state() {
    let secret = encoded([0xE2; 32]);
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut bootstrap = PairingBootstrap::new(
        OneShotStore {
            secret: Some(UiSecret::from_persisted(secret.clone()).unwrap()),
            events: Arc::clone(&events),
        },
        NoRetainBackend {
            events: Arc::clone(&events),
            pair: Ok(()),
        },
        FixedRandom {
            events: Arc::clone(&events),
            byte: 0xFF,
        },
    );
    let mut session = SessionLifecycle::new(
        FailingSession {
            events: Arc::clone(&events),
        },
        Clock,
    );

    let result = bootstrap.connect(&mut session).await;

    assert_eq!(
        result,
        Err(BootstrapFailure::Session(SessionFailure::Transport))
    );
    assert_eq!(
        bootstrap.state(),
        PairingState::Failed(BootstrapFailure::Session(SessionFailure::Transport))
    );
    assert_secret_free(&result, &secret);
    assert_secret_free(&bootstrap.state(), &secret);
    assert_eq!(*events.lock().unwrap(), ["status", "load", "pair", "issue"]);
}

#[derive(Debug, Eq, PartialEq)]
enum KeychainCall {
    Read,
    Write,
}

struct FakeKeychainItem {
    calls: Arc<Mutex<Vec<KeychainCall>>>,
    read: Option<Result<Option<String>, SecretStoreFailure>>,
    write: Result<(), SecretStoreFailure>,
}

impl KeychainItem for FakeKeychainItem {
    fn read(&mut self) -> Result<Option<String>, SecretStoreFailure> {
        self.calls.lock().unwrap().push(KeychainCall::Read);
        self.read
            .take()
            .expect("contract fake is read only once per store")
    }

    fn write(&mut self, _: &UiSecret) -> Result<(), SecretStoreFailure> {
        self.calls.lock().unwrap().push(KeychainCall::Write);
        self.write
    }
}

fn fake_keychain(
    read: Result<Option<String>, SecretStoreFailure>,
    write: Result<(), SecretStoreFailure>,
) -> (
    KeychainSecretStore<FakeKeychainItem>,
    Arc<Mutex<Vec<KeychainCall>>>,
) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let item = FakeKeychainItem {
        calls: Arc::clone(&calls),
        read: Some(read),
        write,
    };
    (KeychainSecretStore::new(item), calls)
}

#[tokio::test]
async fn keychain_store_writes_only_after_a_confirmed_missing_item() {
    let (mut store, calls) = fake_keychain(Ok(None), Ok(()));
    let secret = UiSecret::from_random([0xA9; 32]);

    assert!(matches!(store.load().await, Ok(None)));
    assert!(store.persist(&secret).await.is_ok());
    assert_eq!(
        *calls.lock().unwrap(),
        [KeychainCall::Read, KeychainCall::Write]
    );
}

#[tokio::test]
async fn keychain_store_never_writes_after_denied_or_unavailable_read() {
    for failure in [SecretStoreFailure::Denied, SecretStoreFailure::Unavailable] {
        let (mut store, calls) = fake_keychain(Err(failure), Ok(()));
        let secret = UiSecret::from_random([0xBA; 32]);

        assert!(matches!(store.load().await, Err(error) if error == failure));
        assert!(matches!(store.persist(&secret).await, Err(error) if error == failure));
        assert_eq!(*calls.lock().unwrap(), [KeychainCall::Read]);
    }
}

#[tokio::test]
async fn keychain_store_does_not_retry_a_failed_write() {
    for failure in [SecretStoreFailure::Denied, SecretStoreFailure::Unavailable] {
        let (mut store, calls) = fake_keychain(Ok(None), Err(failure));
        let secret = UiSecret::from_random([0xCB; 32]);

        assert!(matches!(store.load().await, Ok(None)));
        assert!(matches!(store.persist(&secret).await, Err(error) if error == failure));
        assert!(matches!(store.persist(&secret).await, Err(error) if error == failure));
        assert_eq!(
            *calls.lock().unwrap(),
            [KeychainCall::Read, KeychainCall::Write]
        );
    }
}

#[tokio::test]
async fn keychain_store_never_overwrites_present_or_malformed_items() {
    for value in [encoded([0xCB; 32]), "not-a-canonical-pairing-secret".into()] {
        let (mut store, calls) = fake_keychain(Ok(Some(value)), Ok(()));
        let secret = UiSecret::from_random([0xDC; 32]);

        let load = store.load().await;
        let expected = match load {
            Ok(Some(_)) => SecretStoreFailure::Unavailable,
            Err(SecretStoreFailure::Corrupt) => SecretStoreFailure::Corrupt,
            _ => panic!("unexpected Keychain load result"),
        };
        assert!(matches!(
            store.persist(&secret).await,
            Err(failure) if failure == expected
        ));
        assert_eq!(*calls.lock().unwrap(), [KeychainCall::Read]);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn macos_keychain_adapter_is_compile_checked_without_opening_keychain() {
    let _: fn() -> Result<MacosKeychainSecretStore, SecretStoreFailure> =
        MacosKeychainSecretStore::new;
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
