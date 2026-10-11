//! Injectable TUI-owned Keychain persistence for the UI pairing secret.
//!
//! The concrete macOS item is deliberately separate from every bridge OAuth,
//! mirror, or daemon credential. Constructing this adapter does not start a
//! pairing flow; callers must explicitly inject it into `PairingBootstrap`.

use super::{SecretStoreFailure, UiSecret, UiSecretStore};
#[cfg(target_os = "macos")]
use std::future::Future;
#[cfg(target_os = "macos")]
use zeroize::Zeroize;

/// Stable, TUI-owned Keychain identifiers. They are labels, never secret data.
#[cfg(target_os = "macos")]
const TUI_PAIRING_SERVICE: &str = "com.cisco.webex-tui.ui-pairing";
#[cfg(target_os = "macos")]
const TUI_PAIRING_ACCOUNT: &str = "ui-secret-v1";

/// Minimal synchronous Keychain item boundary. Keeping this injectable makes
/// contract tests independent of the user's Keychain and prevents the pairing
/// state machine from gaining a platform dependency.
pub(crate) trait KeychainItem: Send {
    /// `Ok(None)` is reserved exclusively for a confirmed missing item.
    fn read(&mut self) -> Result<Option<String>, SecretStoreFailure>;
    /// Write the borrowed secret only after this store has observed `None`.
    fn write(&mut self, secret: &UiSecret) -> Result<(), SecretStoreFailure>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ItemState {
    Unchecked,
    Missing,
    Present,
    AccessFailed(SecretStoreFailure),
}

/// State-gated Keychain-backed [`UiSecretStore`].
///
/// A write is possible only after this instance has conclusively observed a
/// missing item. Any denied, unavailable, malformed, or otherwise failed read
/// blocks writes and returns the same secret-store failure. This makes an
/// access problem fail closed rather than risk replacing an existing secret.
pub(crate) struct KeychainSecretStore<I: KeychainItem> {
    item: I,
    state: ItemState,
}

impl<I: KeychainItem> KeychainSecretStore<I> {
    pub(crate) fn new(item: I) -> Self {
        Self {
            item,
            state: ItemState::Unchecked,
        }
    }

    fn record_failure<T>(&mut self, failure: SecretStoreFailure) -> Result<T, SecretStoreFailure> {
        self.state = ItemState::AccessFailed(failure);
        Err(failure)
    }
}

impl<I: KeychainItem> UiSecretStore for KeychainSecretStore<I> {
    async fn load(&mut self) -> Result<Option<UiSecret>, SecretStoreFailure> {
        match self.item.read() {
            Ok(None) => {
                self.state = ItemState::Missing;
                Ok(None)
            }
            Ok(Some(value)) => match UiSecret::from_persisted(value) {
                Ok(secret) => {
                    self.state = ItemState::Present;
                    Ok(Some(secret))
                }
                Err(failure) => self.record_failure(failure),
            },
            Err(failure) => self.record_failure(failure),
        }
    }

    async fn persist(&mut self, secret: &UiSecret) -> Result<(), SecretStoreFailure> {
        match self.state {
            ItemState::Missing => match self.item.write(secret) {
                Ok(()) => {
                    self.state = ItemState::Present;
                    Ok(())
                }
                Err(failure) => self.record_failure(failure),
            },
            ItemState::AccessFailed(failure) => Err(failure),
            ItemState::Unchecked | ItemState::Present => Err(SecretStoreFailure::Unavailable),
        }
    }
}

#[cfg(target_os = "macos")]
struct NativeMacosKeychainItem {
    entry: keyring::Entry,
}

#[cfg(target_os = "macos")]
impl NativeMacosKeychainItem {
    fn new() -> Result<Self, SecretStoreFailure> {
        keyring::Entry::new(TUI_PAIRING_SERVICE, TUI_PAIRING_ACCOUNT)
            .map(|entry| Self { entry })
            .map_err(keyring_failure)
    }
}

#[cfg(target_os = "macos")]
fn keyring_failure(error: keyring::Error) -> SecretStoreFailure {
    match error {
        keyring::Error::NoStorageAccess(_) => SecretStoreFailure::Denied,
        keyring::Error::BadEncoding(mut bytes) => {
            bytes.zeroize();
            SecretStoreFailure::Unavailable
        }
        // `NoEntry` is handled only by `read`, where it means a confirmed
        // missing item. Every other failure is deliberately opaque.
        _ => SecretStoreFailure::Unavailable,
    }
}

#[cfg(target_os = "macos")]
impl KeychainItem for NativeMacosKeychainItem {
    fn read(&mut self) -> Result<Option<String>, SecretStoreFailure> {
        match self.entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keyring_failure(error)),
        }
    }

    fn write(&mut self, secret: &UiSecret) -> Result<(), SecretStoreFailure> {
        self.entry
            .set_password(secret.as_str())
            .map_err(keyring_failure)
    }
}

/// Concrete macOS login-Keychain adapter for the dedicated TUI pairing item.
/// It is deliberately not wired into main, App construction, startup, or a
/// live pairing path.
#[cfg(target_os = "macos")]
pub(crate) struct MacosKeychainSecretStore {
    inner: KeychainSecretStore<NativeMacosKeychainItem>,
}

#[cfg(target_os = "macos")]
impl MacosKeychainSecretStore {
    pub(crate) fn new() -> Result<Self, SecretStoreFailure> {
        Ok(Self {
            inner: KeychainSecretStore::new(NativeMacosKeychainItem::new()?),
        })
    }
}

#[cfg(target_os = "macos")]
impl UiSecretStore for MacosKeychainSecretStore {
    fn load(
        &mut self,
    ) -> impl Future<Output = Result<Option<UiSecret>, SecretStoreFailure>> + Send {
        self.inner.load()
    }

    fn persist(
        &mut self,
        secret: &UiSecret,
    ) -> impl Future<Output = Result<(), SecretStoreFailure>> + Send {
        self.inner.persist(secret)
    }
}
