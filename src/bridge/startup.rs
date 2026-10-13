//! Dormant pre-terminal startup for the bridge-backed TUI (cutover step S2).
//!
//! Not called from `main`. Runs before raw terminal mode and either returns an
//! assembled, not-yet-running bridge stack or a typed [`StartupError`] whose
//! [`StartupError::message`] is printed to the user. In order:
//!
//! 1. Probe the protected socket without connecting: a missing socket means
//!    the daemon is absent (D18); an inaccessible one is a permission error; a
//!    non-socket or a run directory open to group/others is untrusted.
//! 2. Read the unauthenticated `/v1/status` (bounded) and check `apiVersion`
//!    before any secret is touched. A Webex login requirement is a notice, not
//!    a failure: the mirror stays usable.
//! 3. Run the session-first pairing bootstrap (bounded; the daemon's pairing
//!    dialog times out after 60 s), then assemble runtime and App glue.
//!
//! The TUI never executes `webex-bridge install-service` or any other
//! command: when the daemon is absent it only prints the resolved absolute
//! command for the user to run. Messages carry paths and commands, never a
//! secret, hash, token or message content.

use super::{
    pairing::{
        BootstrapFailure, PairingBackend, PairingBootstrap, RandomFailure, RandomSource,
        UiSecretStore, SUPPORTED_API_VERSION,
    },
    runtime::{EventSource, RuntimeConfig},
    session::{SessionBackend, SessionClock, SessionFailure, SessionLifecycle},
};
use crate::app::bridge_glue::{BridgeGlue, BridgeRuntime};
use std::{
    future::Future,
    io::Read,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

/// Keychain item identifiers, repeated here only to print removal commands.
const KEYCHAIN_SERVICE: &str = "com.cisco.webex-tui.ui-pairing";
const KEYCHAIN_ACCOUNT: &str = "ui-secret-v1";

#[derive(Clone, Copy, Debug)]
pub(crate) struct StartupConfig {
    /// Bound for the unauthenticated status read.
    pub(crate) status_deadline: Duration,
    /// Bound for the whole bootstrap, including a pairing dialog (60 s).
    pub(crate) bootstrap_deadline: Duration,
}

impl Default for StartupConfig {
    fn default() -> Self {
        Self {
            status_deadline: Duration::from_secs(5),
            bootstrap_deadline: Duration::from_secs(90),
        }
    }
}

/// What the socket path looks like, observed without connecting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SocketState {
    Present,
    Missing,
    PermissionDenied,
    /// Not a socket, or its run directory is a symlink or open to others.
    Untrusted,
}

pub(crate) trait SocketProbe {
    fn probe(&self) -> SocketState;
}

/// Real probe of the bridge socket and its private run directory.
pub(crate) struct FsSocketProbe {
    socket: PathBuf,
}

impl FsSocketProbe {
    pub(crate) fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
        }
    }
}

impl SocketProbe for FsSocketProbe {
    fn probe(&self) -> SocketState {
        probe_socket(&self.socket)
    }
}

fn probe_socket(socket: &Path) -> SocketState {
    use std::io::ErrorKind;
    let state = |error: std::io::Error| match error.kind() {
        ErrorKind::NotFound => SocketState::Missing,
        ErrorKind::PermissionDenied => SocketState::PermissionDenied,
        _ => SocketState::Untrusted,
    };
    // The daemon keeps its socket in a 0700 run directory: only its owner can
    // traverse it, so a private, non-symlink directory proves ownership.
    if let Some(run) = socket.parent() {
        match std::fs::symlink_metadata(run) {
            Ok(meta) if !meta.is_dir() => return SocketState::Untrusted,
            Ok(meta) if meta.permissions().mode() & 0o077 != 0 => return SocketState::Untrusted,
            Ok(_) => {}
            Err(error) => return state(error),
        }
    }
    match std::fs::symlink_metadata(socket) {
        Ok(meta) if meta.file_type().is_socket() => SocketState::Present,
        Ok(_) => SocketState::Untrusted,
        Err(error) => state(error),
    }
}

/// The parts of `/v1/status` startup needs. Contains no content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StatusSummary {
    pub(crate) api_version: u32,
    /// Daemon-to-Webex state: `connected`, `disconnected` or `auth_required`.
    pub(crate) connection: String,
}

pub(crate) trait StatusSource {
    fn status(&mut self) -> impl Future<Output = Result<StatusSummary, ()>> + Send;
}

pub(crate) struct BridgeStatusSource {
    bridge: webex_bridge::client::BridgeClient,
}

impl BridgeStatusSource {
    pub(crate) fn new(bridge: webex_bridge::client::BridgeClient) -> Self {
        Self { bridge }
    }
}

impl StatusSource for BridgeStatusSource {
    async fn status(&mut self) -> Result<StatusSummary, ()> {
        let status = self.bridge.status().await.map_err(|_| ())?;
        Ok(StatusSummary {
            api_version: status.api_version,
            connection: status.connection,
        })
    }
}

/// Finds the `webex-bridge` CLI so its commands can be printed (never run).
pub(crate) trait CliLocator {
    fn locate(&self) -> Option<PathBuf>;
}

/// `webex-bridge` on `PATH`, else the service's private installed copy.
pub(crate) struct EnvCliLocator {
    path: Option<std::ffi::OsString>,
    installed: Option<PathBuf>,
}

impl EnvCliLocator {
    pub(crate) fn from_env() -> Self {
        Self {
            path: std::env::var_os("PATH"),
            installed: webex_bridge::paths::Paths::discover()
                .ok()
                .map(|paths| paths.bin.join("webex-bridge")),
        }
    }
}

impl CliLocator for EnvCliLocator {
    fn locate(&self) -> Option<PathBuf> {
        let on_path = self.path.as_ref().and_then(|path| {
            std::env::split_paths(path)
                .filter(|dir| dir.is_absolute())
                .map(|dir| dir.join("webex-bridge"))
                .find(|candidate| is_executable(candidate))
        });
        on_path.or_else(|| self.installed.clone().filter(|p| is_executable(p)))
    }
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// 256 bits from the OS CSPRNG device; no extra dependency.
pub(crate) struct OsRandom;

impl RandomSource for OsRandom {
    fn fill_256(&mut self, bytes: &mut [u8; 32]) -> Result<(), RandomFailure> {
        std::fs::File::open("/dev/urandom")
            .and_then(|mut device| device.read_exact(bytes))
            .map_err(|_| RandomFailure::Unavailable)
    }
}

/// Non-fatal conditions shown once the UI is up.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StartupNotice {
    /// The daemon's Webex login was rejected; mirrored content still works.
    WebexLoginRequired { cli: Option<PathBuf> },
    /// The daemon is up but not connected to Webex (offline mirror).
    WebexDisconnected,
}

impl StartupNotice {
    pub(crate) fn message(&self) -> String {
        match self {
            Self::WebexLoginRequired { cli } => format!(
                "The bridge needs a new Webex login; showing mirrored content only. Run: {} login",
                cli_display(cli.as_deref())
            ),
            Self::WebexDisconnected => {
                "The bridge is not connected to Webex; showing mirrored content, writes unavailable."
                    .into()
            }
        }
    }
}

/// Why startup stopped. Every variant is printed and the TUI exits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StartupError {
    DaemonAbsent {
        socket: PathBuf,
        cli: Option<PathBuf>,
    },
    DaemonNotResponding {
        socket: PathBuf,
        cli: Option<PathBuf>,
    },
    SocketPermission {
        socket: PathBuf,
    },
    SocketUntrusted {
        socket: PathBuf,
    },
    /// `daemon` is `None` if the version changed between the two checks.
    VersionMismatch {
        daemon: Option<u32>,
    },
    /// No per-user application data directory to find the socket in.
    NoDataDirectory,
    SecretDenied,
    SecretUnavailable,
    SecretCorrupt,
    SecretNotSaved,
    RandomUnavailable,
    PairingNotApproved,
    PairingUnavailable,
    Session(SessionFailure),
    TimedOut,
}

impl StartupError {
    fn from_bootstrap(failure: BootstrapFailure) -> Self {
        match failure {
            BootstrapFailure::StatusUnavailable => Self::PairingUnavailable,
            BootstrapFailure::VersionMismatch => Self::VersionMismatch { daemon: None },
            BootstrapFailure::SecretDenied => Self::SecretDenied,
            BootstrapFailure::SecretUnavailable => Self::SecretUnavailable,
            BootstrapFailure::SecretCorrupt => Self::SecretCorrupt,
            BootstrapFailure::PersistDenied | BootstrapFailure::PersistUnavailable => {
                Self::SecretNotSaved
            }
            BootstrapFailure::RandomUnavailable => Self::RandomUnavailable,
            BootstrapFailure::PairDenied => Self::PairingNotApproved,
            BootstrapFailure::PairUnavailable => Self::PairingUnavailable,
            BootstrapFailure::Session(failure) => Self::Session(failure),
        }
    }

    /// The user-facing explanation, with exact commands to run. Never
    /// contains a secret, hash, token or message content.
    pub(crate) fn message(&self) -> String {
        match self {
            Self::DaemonAbsent { socket, cli } => format!(
                "The webex-bridge daemon is not running (no socket at {}).\n\
                 webex-tui needs it (it owns the Webex connection). Install and start it yourself:\n  \
                 {}\n\
                 Then start webex-tui again. webex-tui never runs this command for you.",
                socket.display(),
                install_command(cli.as_deref())
            ),
            Self::DaemonNotResponding { socket, cli } => format!(
                "The webex-bridge socket exists at {} but the daemon does not answer.\n\
                 Check it with: {} health\n\
                 If it is not installed or not running, install and start it yourself:\n  {}",
                socket.display(),
                cli_display(cli.as_deref()),
                install_command(cli.as_deref())
            ),
            Self::SocketPermission { socket } => format!(
                "Cannot access the webex-bridge socket at {} (permission denied). \
                 It must belong to your user; do not run webex-tui as another user.",
                socket.display()
            ),
            Self::SocketUntrusted { socket } => format!(
                "Refusing to use {}: it is not a socket in a private (0700) directory. \
                 Stop and reinstall the webex-bridge service.",
                socket.display()
            ),
            Self::VersionMismatch { daemon } => format!(
                "The webex-bridge daemon speaks API version {}, but this webex-tui needs \
                 version {SUPPORTED_API_VERSION}. Update both to matching builds.",
                daemon.map_or_else(|| "(changed during startup)".into(), |v| v.to_string())
            ),
            Self::NoDataDirectory => {
                "No per-user application data directory was found, so the webex-bridge socket cannot be located."
                    .into()
            }
            Self::SecretDenied => "Access to the webex-tui pairing secret in your Keychain was denied. \
                 Start webex-tui again and choose Allow (Always Allow avoids repeated prompts)."
                .into(),
            Self::SecretUnavailable => "The Keychain is unavailable (locked?). Unlock it and start webex-tui again."
                .into(),
            Self::SecretCorrupt => format!(
                "The webex-tui pairing secret in your Keychain is damaged and will not be replaced \
                 automatically. Remove it, then start webex-tui again to pair once more:\n  \
                 security delete-generic-password -s {KEYCHAIN_SERVICE} -a {KEYCHAIN_ACCOUNT}"
            ),
            Self::SecretNotSaved => "A new pairing secret could not be saved to your Keychain; nothing was paired. \
                 Unlock the Keychain (or allow access) and start webex-tui again."
                .into(),
            Self::RandomUnavailable => "The system random source is unavailable; no pairing secret was created."
                .into(),
            Self::PairingNotApproved => "Pairing was not approved (denied, timed out after 60 s, or no graphical session). \
                 Start webex-tui again from your logged-in desktop session to retry; the same secret is reused."
                .into(),
            Self::PairingUnavailable => "The webex-bridge daemon could not be reached for pairing. Check that it is running and start webex-tui again."
                .into(),
            Self::Session(SessionFailure::NotPaired) => "The daemon still does not recognise this webex-tui after pairing. Start webex-tui again."
                .into(),
            Self::Session(failure) => format!(
                "Could not open a webex-bridge UI session ({failure:?}). Start webex-tui again."
            ),
            Self::TimedOut => "Starting the bridge session timed out. Start webex-tui again.".into(),
        }
    }
}

fn cli_display(cli: Option<&Path>) -> String {
    match cli {
        Some(path) => shell_quote(path),
        None => "webex-bridge".into(),
    }
}

fn install_command(cli: Option<&Path>) -> String {
    match cli {
        Some(path) => format!("{} install-service", shell_quote(path)),
        None => "(webex-bridge was not found on PATH or in its service directory; build it from \
                 the webex-bridge checkout and run `mise run service:start` there)"
            .into(),
    }
}

/// Single-quote a path for copy-paste into a POSIX shell.
fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', r"'\''"))
}

/// Injected dependencies for one startup attempt.
pub(crate) struct StartupDeps<Probe, Status, Cli, S, P, R, B, C, K>
where
    S: UiSecretStore,
    P: PairingBackend,
    R: RandomSource,
    B: SessionBackend,
    C: SessionClock,
{
    pub(crate) socket: PathBuf,
    pub(crate) probe: Probe,
    pub(crate) status: Status,
    pub(crate) cli: Cli,
    pub(crate) bootstrap: PairingBootstrap<S, P, R>,
    pub(crate) session: SessionLifecycle<B, C>,
    /// Second secret-store instance for the runtime's reconnect re-borrows.
    pub(crate) runtime_secrets: K,
    pub(crate) runtime: RuntimeConfig,
    pub(crate) page_size: u32,
    pub(crate) config: StartupConfig,
}

/// An assembled, not-yet-running stack plus the notices to show.
pub(crate) struct Started<B: SessionBackend, C: SessionClock, K: UiSecretStore> {
    pub(crate) runtime: BridgeRuntime<B, C, K>,
    pub(crate) glue: BridgeGlue,
    pub(crate) notices: Vec<StartupNotice>,
}

/// Run startup steps 1–3. Nothing is spawned and nothing is executed.
pub(crate) async fn start<Probe, Status, Cli, S, P, R, B, C, K>(
    deps: StartupDeps<Probe, Status, Cli, S, P, R, B, C, K>,
) -> Result<Started<B, C, K>, StartupError>
where
    Probe: SocketProbe,
    Status: StatusSource,
    Cli: CliLocator,
    S: UiSecretStore,
    P: PairingBackend,
    R: RandomSource,
    B: SessionBackend,
    B::Stream: EventSource,
    C: SessionClock,
    K: UiSecretStore,
{
    let StartupDeps {
        socket,
        probe,
        mut status,
        cli,
        mut bootstrap,
        mut session,
        runtime_secrets,
        runtime,
        page_size,
        config,
    } = deps;
    match probe.probe() {
        SocketState::Present => {}
        SocketState::Missing => {
            return Err(StartupError::DaemonAbsent {
                socket,
                cli: cli.locate(),
            })
        }
        SocketState::PermissionDenied => return Err(StartupError::SocketPermission { socket }),
        SocketState::Untrusted => return Err(StartupError::SocketUntrusted { socket }),
    }
    let summary = match tokio::time::timeout(config.status_deadline, status.status()).await {
        Ok(Ok(summary)) => summary,
        Ok(Err(())) | Err(_) => {
            return Err(StartupError::DaemonNotResponding {
                socket,
                cli: cli.locate(),
            })
        }
    };
    if summary.api_version != SUPPORTED_API_VERSION {
        return Err(StartupError::VersionMismatch {
            daemon: Some(summary.api_version),
        });
    }
    let mut notices = Vec::new();
    match summary.connection.as_str() {
        "auth_required" => notices.push(StartupNotice::WebexLoginRequired { cli: cli.locate() }),
        "connected" => {}
        _ => notices.push(StartupNotice::WebexDisconnected),
    }
    match tokio::time::timeout(config.bootstrap_deadline, bootstrap.connect(&mut session)).await {
        Ok(Ok(_subscription)) => {}
        Ok(Err(failure)) => return Err(StartupError::from_bootstrap(failure)),
        Err(_) => return Err(StartupError::TimedOut),
    }
    let (runtime, glue) = BridgeGlue::assemble(session, runtime_secrets, runtime, page_size);
    Ok(Started {
        runtime,
        glue,
        notices,
    })
}

/// The production dependency set (macOS Keychain, protected UDS, system
/// clock). Building it opens no Keychain item and contacts no daemon; only
/// `start` does, and nothing calls it until the consented `main` switch.
#[cfg(target_os = "macos")]
pub(crate) type MacosStartupDeps = StartupDeps<
    FsSocketProbe,
    BridgeStatusSource,
    EnvCliLocator,
    super::pairing::keychain::MacosKeychainSecretStore,
    super::pairing::BridgePairingBackend,
    OsRandom,
    super::session::BridgeSessionBackend,
    super::session::SystemClock,
    super::pairing::keychain::MacosKeychainSecretStore,
>;

#[cfg(target_os = "macos")]
pub(crate) fn macos_deps(page_size: u32) -> Result<MacosStartupDeps, StartupError> {
    use super::pairing::keychain::MacosKeychainSecretStore;
    use webex_bridge::client::BridgeClient;
    let bridge = BridgeClient::discover().map_err(|_| StartupError::NoDataDirectory)?;
    let socket = bridge.socket().to_path_buf();
    let store = MacosKeychainSecretStore::new().map_err(|_| StartupError::SecretUnavailable)?;
    let runtime_secrets =
        MacosKeychainSecretStore::new().map_err(|_| StartupError::SecretUnavailable)?;
    Ok(StartupDeps {
        probe: FsSocketProbe::new(&socket),
        socket,
        status: BridgeStatusSource::new(bridge.clone()),
        cli: EnvCliLocator::from_env(),
        bootstrap: PairingBootstrap::new(
            store,
            super::pairing::BridgePairingBackend::new(bridge.clone()),
            OsRandom,
        ),
        session: SessionLifecycle::new(
            super::session::BridgeSessionBackend::new(bridge),
            super::session::SystemClock,
        ),
        runtime_secrets,
        runtime: RuntimeConfig::default(),
        page_size,
        config: StartupConfig::default(),
    })
}

#[cfg(test)]
mod tests;
