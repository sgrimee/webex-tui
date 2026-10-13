//! Fake-only startup tests: no daemon, Keychain, pairing dialog or process
//! execution. The real socket probe is exercised on a private temp directory.
use super::*;
use crate::bridge::{
    pairing::{PairingBackendFailure, SecretStoreFailure, UiSecret},
    session::RenewFailure,
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Instant,
};
use webex_bridge::client::events::BridgeEvent;

type Log = Arc<Mutex<Vec<&'static str>>>;

const SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

struct Probe(SocketState, Log);
impl SocketProbe for Probe {
    fn probe(&self) -> SocketState {
        self.1.lock().unwrap().push("probe");
        self.0
    }
}

struct Status(Option<Result<StatusSummary, ()>>, Log);
impl StatusSource for Status {
    async fn status(&mut self) -> Result<StatusSummary, ()> {
        self.1.lock().unwrap().push("status");
        match self.0.clone() {
            Some(reply) => reply,
            None => std::future::pending().await,
        }
    }
}

struct Cli(Option<PathBuf>);
impl CliLocator for Cli {
    fn locate(&self) -> Option<PathBuf> {
        self.0.clone()
    }
}

struct Store {
    log: Log,
    loaded: Result<Option<&'static str>, SecretStoreFailure>,
}
impl UiSecretStore for Store {
    async fn load(&mut self) -> Result<Option<UiSecret>, SecretStoreFailure> {
        self.log.lock().unwrap().push("load");
        match self.loaded {
            Ok(Some(value)) => UiSecret::from_persisted(value.into()).map(Some),
            Ok(None) => Ok(None),
            Err(failure) => Err(failure),
        }
    }
    async fn persist(&mut self, _: &UiSecret) -> Result<(), SecretStoreFailure> {
        self.log.lock().unwrap().push("persist");
        Ok(())
    }
}

struct Pairing {
    log: Log,
    pair: Option<Result<(), PairingBackendFailure>>,
}
impl PairingBackend for Pairing {
    async fn status(&mut self) -> Result<u32, PairingBackendFailure> {
        Ok(SUPPORTED_API_VERSION)
    }
    async fn pair(&mut self, _: &str) -> Result<(), PairingBackendFailure> {
        self.log.lock().unwrap().push("pair");
        match self.pair {
            Some(reply) => reply,
            None => std::future::pending().await,
        }
    }
}

struct Random;
impl RandomSource for Random {
    fn fill_256(&mut self, bytes: &mut [u8; 32]) -> Result<(), RandomFailure> {
        *bytes = [7; 32];
        Ok(())
    }
}

struct Stream;
impl EventSource for Stream {
    async fn next_event(&mut self) -> Option<Result<BridgeEvent, SessionFailure>> {
        std::future::pending().await
    }
}

struct Session {
    log: Log,
    issues: VecDeque<SessionFailure>,
    hang: bool,
}
impl SessionBackend for Session {
    type Client = ();
    type Stream = Stream;
    type Writer = ();
    type SnapshotReader = ();
    async fn issue(&mut self, _: &str) -> Result<(), SessionFailure> {
        self.log.lock().unwrap().push("issue");
        if self.hang {
            std::future::pending::<()>().await;
        }
        match self.issues.pop_front() {
            Some(failure) => Err(failure),
            None => Ok(()),
        }
    }
    async fn subscribe(&mut self, _: &()) -> Result<Stream, SessionFailure> {
        self.log.lock().unwrap().push("subscribe");
        Ok(Stream)
    }
    async fn continuity(
        &mut self,
        _: &(),
    ) -> Result<crate::frontend::continuity::ContinuityBoundary, SessionFailure> {
        unreachable!("startup never snapshots")
    }
    async fn renew(&mut self, _: &()) -> Result<u64, RenewFailure> {
        unreachable!("startup never renews")
    }
    fn lease_ttl(_: &()) -> u64 {
        600
    }
    fn writer(_: &()) {}
    fn snapshot_reader(_: &()) {}
}

struct Clock;
impl SessionClock for Clock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

struct Fixture {
    socket: SocketState,
    status: Option<Result<StatusSummary, ()>>,
    cli: Option<PathBuf>,
    loaded: Result<Option<&'static str>, SecretStoreFailure>,
    pair: Option<Result<(), PairingBackendFailure>>,
    issues: Vec<SessionFailure>,
    /// Session creation never answers.
    issue_hangs: bool,
}

impl Default for Fixture {
    fn default() -> Self {
        Self {
            socket: SocketState::Present,
            status: Some(Ok(connected())),
            cli: Some(PathBuf::from("/opt/bridge/bin/webex-bridge")),
            loaded: Ok(Some(SECRET)),
            pair: Some(Ok(())),
            issues: Vec::new(),
            issue_hangs: false,
        }
    }
}

fn connected() -> StatusSummary {
    StatusSummary {
        api_version: SUPPORTED_API_VERSION,
        connection: "connected".into(),
    }
}

type Deps = StartupDeps<Probe, Status, Cli, Store, Pairing, Random, Session, Clock, Store>;

fn deps(f: Fixture, log: &Log) -> Deps {
    StartupDeps {
        socket: PathBuf::from("/synthetic/run/bridge.sock"),
        probe: Probe(f.socket, Arc::clone(log)),
        status: Status(f.status, Arc::clone(log)),
        cli: Cli(f.cli),
        bootstrap: PairingBootstrap::new(
            Store {
                log: Arc::clone(log),
                loaded: f.loaded,
            },
            Pairing {
                log: Arc::clone(log),
                pair: f.pair,
            },
            Random,
        ),
        session: SessionLifecycle::new(
            Session {
                log: Arc::clone(log),
                issues: f.issues.into(),
                hang: f.issue_hangs,
            },
            Clock,
        ),
        runtime_secrets: Store {
            log: Arc::new(Mutex::new(Vec::new())),
            loaded: Ok(Some(SECRET)),
        },
        runtime: RuntimeConfig::default(),
        page_size: 50,
        config: StartupConfig {
            status_deadline: Duration::from_millis(20),
            bootstrap_deadline: Duration::from_millis(50),
        },
    }
}

async fn run(f: Fixture) -> (Result<Vec<StartupNotice>, StartupError>, Vec<&'static str>) {
    let log: Log = Arc::default();
    let result = start(deps(f, &log)).await.map(|started| started.notices);
    let calls = log.lock().unwrap().clone();
    (result, calls)
}

#[tokio::test]
async fn a_paired_secret_starts_without_pairing_and_assembles_the_stack() {
    let (result, calls) = run(Fixture::default()).await;
    assert_eq!(result.unwrap(), []);
    assert_eq!(calls, ["probe", "status", "load", "issue", "subscribe"]);
}

#[tokio::test]
async fn an_absent_daemon_prints_the_install_command_and_touches_nothing_else() {
    let (result, calls) = run(Fixture {
        socket: SocketState::Missing,
        ..Fixture::default()
    })
    .await;
    let error = result.unwrap_err();
    assert!(matches!(error, StartupError::DaemonAbsent { .. }));
    let message = error.message();
    assert!(message.contains("'/opt/bridge/bin/webex-bridge' install-service"));
    assert!(message.contains("never runs this command"));
    assert!(message.contains("/synthetic/run/bridge.sock"));
    // No status read, no Keychain access, no pairing, no session.
    assert_eq!(calls, ["probe"]);

    let (result, _) = run(Fixture {
        socket: SocketState::Missing,
        cli: None,
        ..Fixture::default()
    })
    .await;
    let message = result.unwrap_err().message();
    assert!(message.contains("not found on PATH"));
    assert!(message.contains("mise run service:start"));
}

#[tokio::test]
async fn socket_permission_and_untrusted_sockets_stop_before_any_io() {
    for (state, expected) in [
        (SocketState::PermissionDenied, "permission denied"),
        (SocketState::Untrusted, "Refusing to use"),
    ] {
        let (result, calls) = run(Fixture {
            socket: state,
            ..Fixture::default()
        })
        .await;
        assert!(result.unwrap_err().message().contains(expected));
        assert_eq!(calls, ["probe"]);
    }
}

#[tokio::test]
async fn an_unresponsive_daemon_is_bounded_and_reported_with_commands() {
    for status in [None, Some(Err(()))] {
        let (result, calls) = run(Fixture {
            status,
            ..Fixture::default()
        })
        .await;
        let error = result.unwrap_err();
        assert!(matches!(error, StartupError::DaemonNotResponding { .. }));
        let message = error.message();
        assert!(message.contains("'/opt/bridge/bin/webex-bridge' health"));
        assert!(message.contains("install-service"));
        assert_eq!(calls, ["probe", "status"]);
    }
}

#[tokio::test]
async fn a_version_mismatch_stops_before_the_keychain() {
    let (result, calls) = run(Fixture {
        status: Some(Ok(StatusSummary {
            api_version: SUPPORTED_API_VERSION + 1,
            connection: "connected".into(),
        })),
        ..Fixture::default()
    })
    .await;
    let error = result.unwrap_err();
    assert_eq!(
        error,
        StartupError::VersionMismatch {
            daemon: Some(SUPPORTED_API_VERSION + 1)
        }
    );
    assert!(error.message().contains("API version 2"));
    assert_eq!(calls, ["probe", "status"]);
}

#[tokio::test]
async fn webex_login_and_offline_states_are_notices_not_failures() {
    for (connection, login) in [("auth_required", true), ("disconnected", false)] {
        let (result, _) = run(Fixture {
            status: Some(Ok(StatusSummary {
                api_version: SUPPORTED_API_VERSION,
                connection: connection.into(),
            })),
            ..Fixture::default()
        })
        .await;
        let notices = result.unwrap();
        assert_eq!(notices.len(), 1);
        if login {
            assert!(notices[0]
                .message()
                .contains("'/opt/bridge/bin/webex-bridge' login"));
        } else {
            assert_eq!(notices[0], StartupNotice::WebexDisconnected);
        }
    }
}

#[tokio::test]
async fn bootstrap_failures_map_to_actionable_messages() {
    let corrupt = run(Fixture {
        loaded: Err(SecretStoreFailure::Corrupt),
        ..Fixture::default()
    })
    .await
    .0
    .unwrap_err();
    assert_eq!(corrupt, StartupError::SecretCorrupt);
    assert!(corrupt.message().contains(
        "security delete-generic-password -s com.cisco.webex-tui.ui-pairing -a ui-secret-v1"
    ));

    let denied = run(Fixture {
        loaded: Err(SecretStoreFailure::Denied),
        ..Fixture::default()
    })
    .await
    .0
    .unwrap_err();
    assert_eq!(denied, StartupError::SecretDenied);

    // Unpaired existing secret, pairing refused (denied/timeout/headless).
    let (result, calls) = run(Fixture {
        issues: vec![SessionFailure::NotPaired],
        pair: Some(Err(PairingBackendFailure::Denied)),
        ..Fixture::default()
    })
    .await;
    let refused = result.unwrap_err();
    assert_eq!(refused, StartupError::PairingNotApproved);
    assert!(refused.message().contains("same secret is reused"));
    assert_eq!(calls, ["probe", "status", "load", "issue", "pair"]);

    // Fresh secret: generated, persisted, paired, connected.
    let (result, calls) = run(Fixture {
        loaded: Ok(None),
        ..Fixture::default()
    })
    .await;
    assert!(result.is_ok());
    assert_eq!(
        calls,
        [
            "probe",
            "status",
            "load",
            "persist",
            "pair",
            "issue",
            "subscribe"
        ]
    );
}

#[tokio::test]
async fn an_unanswered_pairing_dialog_times_out_as_a_pairing_timeout() {
    let (result, calls) = run(Fixture {
        issues: vec![SessionFailure::NotPaired],
        pair: None, // the dialog never answers
        ..Fixture::default()
    })
    .await;
    let error = result.unwrap_err();
    assert_eq!(error, StartupError::PairingTimedOut);
    assert!(error.message().contains("close it with Deny"));
    assert_eq!(calls.last(), Some(&"pair"));
}

#[tokio::test]
async fn the_glue_starts_with_the_daemons_webex_state() {
    use crate::frontend::events::ConnectionState;
    for (connection, expected) in [
        ("connected", ConnectionState::Connected),
        ("auth_required", ConnectionState::AuthRequired),
        ("disconnected", ConnectionState::Disconnected),
    ] {
        let log: Log = Arc::default();
        let started = start(deps(
            Fixture {
                status: Some(Ok(StatusSummary {
                    api_version: SUPPORTED_API_VERSION,
                    connection: connection.into(),
                })),
                ..Fixture::default()
            },
            &log,
        ))
        .await
        .ok()
        .unwrap();
        assert_eq!(started.glue.upstream_webex(), expected, "{connection}");
    }
}

#[tokio::test]
async fn a_hung_session_create_times_out_without_blaming_pairing() {
    let (result, calls) = run(Fixture {
        issue_hangs: true,
        ..Fixture::default()
    })
    .await;
    assert_eq!(result.unwrap_err(), StartupError::TimedOut);
    assert!(!calls.contains(&"pair"));
}

#[test]
fn messages_quote_paths_and_never_contain_secrets() {
    let error = StartupError::DaemonAbsent {
        socket: PathBuf::from("/x/run/bridge.sock"),
        cli: Some(PathBuf::from("/Users/o'neil/bin/webex-bridge")),
    };
    assert!(error
        .message()
        .contains(r"'/Users/o'\''neil/bin/webex-bridge' install-service"));
    for error in [
        StartupError::SecretDenied,
        StartupError::SecretCorrupt,
        StartupError::PairingNotApproved,
        StartupError::Session(SessionFailure::Transport),
    ] {
        assert!(!error.message().contains(SECRET));
    }
}

#[test]
fn the_real_probe_checks_the_private_run_directory_and_socket_type() {
    let temp = tempfile::tempdir().unwrap();
    let run = temp.path().join("run");
    std::fs::create_dir(&run).unwrap();
    std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o700)).unwrap();
    let socket = run.join("bridge.sock");
    assert_eq!(probe_socket(&socket), SocketState::Missing);
    std::fs::write(&socket, b"").unwrap();
    assert_eq!(
        probe_socket(&socket),
        SocketState::Untrusted,
        "not a socket"
    );
    std::fs::remove_file(&socket).unwrap();
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    assert_eq!(probe_socket(&socket), SocketState::Present);
    std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(probe_socket(&socket), SocketState::Untrusted, "open dir");
    assert_eq!(
        probe_socket(&temp.path().join("absent/bridge.sock")),
        SocketState::Missing
    );
}

#[test]
fn the_cli_locator_prefers_path_then_the_installed_copy() {
    let temp = tempfile::tempdir().unwrap();
    let on_path = temp.path().join("path");
    let installed = temp.path().join("installed/webex-bridge");
    std::fs::create_dir_all(&on_path).unwrap();
    std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
    let locator = |path: &Path| EnvCliLocator {
        path: Some(path.as_os_str().to_owned()),
        installed: Some(installed.clone()),
    };
    assert_eq!(locator(&on_path).locate(), None);
    std::fs::write(&installed, b"").unwrap();
    assert_eq!(locator(&on_path).locate(), None, "not executable");
    std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(locator(&on_path).locate(), Some(installed.clone()));
    let cli = on_path.join("webex-bridge");
    std::fs::write(&cli, b"").unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(locator(&on_path).locate(), Some(cli));
}

#[cfg(target_os = "macos")]
#[test]
fn the_production_dependency_set_is_compile_checked_only() {
    // Never called here: building it would construct Keychain entries.
    let _: fn(u32) -> Result<MacosStartupDeps, StartupError> = macos_deps;
}

#[test]
fn runtime_exits_map_to_actionable_secret_free_messages() {
    use crate::bridge::runtime::RuntimeExit;
    assert_eq!(exit_message(RuntimeExit::Shutdown), None);
    let repair = exit_message(RuntimeExit::PairingRequired(SessionFailure::NotPaired)).unwrap();
    assert!(repair.contains("pair once more"));
    assert!(repair.contains("ui unpair"));
    let keychain = exit_message(RuntimeExit::SecretUnavailable).unwrap();
    assert!(keychain.contains("choose Allow"));
    let corrupt = exit_message(RuntimeExit::SecretCorrupt).unwrap();
    assert!(corrupt.contains(
        "security delete-generic-password -s com.cisco.webex-tui.ui-pairing -a ui-secret-v1"
    ));
    for message in [repair, keychain, corrupt] {
        assert!(!message.contains(SECRET));
        assert!(!message.contains("   "), "stray spacing in {message:?}");
    }
}
