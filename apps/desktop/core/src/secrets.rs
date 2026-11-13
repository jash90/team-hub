//! The API token and the key of the local cache. Both live in the operating system's keychain
//! and nowhere else: not in a file, not in a log, not in anything sent to the window.

use std::collections::HashMap;
use std::sync::Mutex;

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};

use crate::error::{CoreError, CoreResult};

pub const TOKEN_ENTRY: &str = "api-token";
pub const CACHE_KEY_ENTRY: &str = "cache-key";
/// For machines without a keychain (CI, a container): the token for this process only.
pub const TOKEN_VARIABLE: &str = "TEAM_HUB_TOKEN";

const SERVICE: &str = "team-hub-desktop";
const NONCE_LENGTH: usize = 12;
const KEY_LENGTH: usize = 32;

/// Text that must not be printed. `Debug` shows only that it is there.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Secret(***)")
    }
}

pub trait Keychain: Send + Sync {
    fn get(&self, entry: &str) -> CoreResult<Option<Secret>>;
    fn set(&self, entry: &str, value: &Secret) -> CoreResult<()>;
    fn delete(&self, entry: &str) -> CoreResult<()>;
}

/// The real one: Keychain on macOS, Credential Manager on Windows, Secret Service on Linux.
pub struct OsKeychain;

impl OsKeychain {
    /// The keychain libraries block, and on Linux start a runtime of their own, so every call
    /// gets a thread of its own instead of whichever one asked.
    fn on_thread<T: Send>(work: impl FnOnce() -> Result<T, keyring::Error> + Send) -> Result<T, keyring::Error> {
        std::thread::scope(|scope| {
            scope
                .spawn(work)
                .join()
                .unwrap_or_else(|_| Err(keyring::Error::PlatformFailure("the keychain call panicked".into())))
        })
    }
}

fn keychain_error(error: keyring::Error) -> CoreError {
    CoreError::Keychain(error.to_string())
}

impl Keychain for OsKeychain {
    fn get(&self, entry: &str) -> CoreResult<Option<Secret>> {
        let found = Self::on_thread(|| keyring::Entry::new(SERVICE, entry)?.get_password());

        match found {
            Ok(value) => Ok(Some(Secret::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(keychain_error(error)),
        }
    }

    fn set(&self, entry: &str, value: &Secret) -> CoreResult<()> {
        Self::on_thread(|| keyring::Entry::new(SERVICE, entry)?.set_password(value.expose())).map_err(keychain_error)
    }

    fn delete(&self, entry: &str) -> CoreResult<()> {
        match Self::on_thread(|| keyring::Entry::new(SERVICE, entry)?.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(keychain_error(error)),
        }
    }
}

/// Holds secrets for the life of the process: tests, and machines without a keychain.
#[derive(Default)]
pub struct MemoryKeychain {
    entries: Mutex<HashMap<String, Secret>>,
}

impl MemoryKeychain {
    pub fn with_token(token: &str) -> Self {
        let keychain = Self::default();

        keychain.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(TOKEN_ENTRY.to_string(), Secret::new(token));
        keychain
    }
}

impl Keychain for MemoryKeychain {
    fn get(&self, entry: &str) -> CoreResult<Option<Secret>> {
        Ok(self.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).get(entry).cloned())
    }

    fn set(&self, entry: &str, value: &Secret) -> CoreResult<()> {
        self.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(entry.to_string(), value.clone());

        Ok(())
    }

    fn delete(&self, entry: &str) -> CoreResult<()> {
        self.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(entry);

        Ok(())
    }
}

/// The token for this process: `TEAM_HUB_TOKEN` when set, else the keychain's.
pub fn load_token(keychain: &dyn Keychain) -> CoreResult<Option<Secret>> {
    if let Ok(value) = std::env::var(TOKEN_VARIABLE) {
        if !value.trim().is_empty() {
            return Ok(Some(Secret::new(value.trim())));
        }
    }

    keychain.get(TOKEN_ENTRY)
}

/// Seals the copy of the server's data kept on disk.
pub struct CacheKey {
    cipher: ChaCha20Poly1305,
    /// False when the key could not be kept in a keychain: what it seals is unreadable after
    /// this process ends, which is the point.
    pub persistent: bool,
}

fn random_bytes<const N: usize>() -> CoreResult<[u8; N]> {
    let mut bytes = [0u8; N];

    getrandom::fill(&mut bytes).map_err(|error| CoreError::Keychain(format!("no random numbers: {error}")))?;

    Ok(bytes)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }

    (0..text.len()).step_by(2).map(|index| u8::from_str_radix(text.get(index..index + 2)?, 16).ok()).collect()
}

impl CacheKey {
    /// The key from the keychain, created on first use. Without a working keychain the key
    /// lives in memory only, and so does everything sealed with it.
    pub fn load_or_create(keychain: &dyn Keychain) -> CoreResult<Self> {
        let stored = keychain.get(CACHE_KEY_ENTRY).ok().flatten().and_then(|secret| from_hex(secret.expose()));

        if let Some(bytes) = stored.filter(|bytes| bytes.len() == KEY_LENGTH) {
            return Ok(Self { cipher: ChaCha20Poly1305::new(Key::from_slice(&bytes)), persistent: true });
        }

        let bytes = random_bytes::<KEY_LENGTH>()?;
        let persistent = keychain.set(CACHE_KEY_ENTRY, &Secret::new(to_hex(&bytes))).is_ok() && keychain.get(CACHE_KEY_ENTRY).ok().flatten().is_some();

        Ok(Self { cipher: ChaCha20Poly1305::new(Key::from_slice(&bytes)), persistent })
    }

    pub fn seal(&self, plain: &[u8]) -> CoreResult<Vec<u8>> {
        let nonce = random_bytes::<NONCE_LENGTH>()?;
        let sealed = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), plain)
            .map_err(|_| CoreError::not_possible("the cache could not be sealed"))?;

        Ok([nonce.as_slice(), sealed.as_slice()].concat())
    }

    /// `None` when the bytes were sealed with another key or changed since.
    pub fn open(&self, sealed: &[u8]) -> Option<Vec<u8>> {
        if sealed.len() < NONCE_LENGTH {
            return None;
        }

        let (nonce, body) = sealed.split_at(NONCE_LENGTH);

        self.cipher.decrypt(Nonce::from_slice(nonce), body).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_never_prints_itself() {
        let secret = Secret::new("th_abcdef");

        assert_eq!(format!("{secret:?}"), "Secret(***)");
        assert!(!format!("{:?}", Some(&secret)).contains("abcdef"));
    }

    #[test]
    fn the_cache_key_is_kept_in_the_keychain_and_reused() {
        let keychain = MemoryKeychain::default();
        let first = CacheKey::load_or_create(&keychain).unwrap();
        let sealed = first.seal(b"dashboard").unwrap();
        let second = CacheKey::load_or_create(&keychain).unwrap();

        assert!(first.persistent);
        assert!(!sealed.windows(9).any(|window| window == b"dashboard"));
        assert_eq!(second.open(&sealed).as_deref(), Some(b"dashboard".as_slice()));
    }

    #[test]
    fn another_key_or_a_changed_byte_opens_nothing() {
        let key = CacheKey::load_or_create(&MemoryKeychain::default()).unwrap();
        let other = CacheKey::load_or_create(&MemoryKeychain::default()).unwrap();
        let mut sealed = key.seal(b"issue").unwrap();

        assert_eq!(other.open(&sealed), None);

        let last = sealed.len() - 1;
        sealed[last] ^= 1;

        assert_eq!(key.open(&sealed), None);
        assert_eq!(key.open(b"short"), None);
    }

    struct BrokenKeychain;

    impl Keychain for BrokenKeychain {
        fn get(&self, _entry: &str) -> CoreResult<Option<Secret>> {
            Err(CoreError::Keychain("no keychain here".into()))
        }

        fn set(&self, _entry: &str, _value: &Secret) -> CoreResult<()> {
            Err(CoreError::Keychain("no keychain here".into()))
        }

        fn delete(&self, _entry: &str) -> CoreResult<()> {
            Ok(())
        }
    }

    #[test]
    fn without_a_keychain_the_key_lasts_only_as_long_as_the_process() {
        let key = CacheKey::load_or_create(&BrokenKeychain).unwrap();

        assert!(!key.persistent);
        assert!(key.open(&key.seal(b"x").unwrap()).is_some());
    }
}
