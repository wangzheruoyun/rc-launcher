//! Persistent token storage (task 5).
//!
//! Accounts (including their secrets) are persisted through the [`TokenStorage`]
//! trait. Two implementations are provided:
//!
//! * [`MemoryTokenStorage`] — in-process only (used as the runtime default and
//!   in tests).
//! * [`FileTokenStorage`] — atomic, encrypted-on-disk storage. The blob is
//!   sealed by a [`crate::auth::vault::SecretVault`]; on Android that vault is
//!   Keystore-backed (see `vault`), so the encrypted file on disk is useless
//!   without the Keystore key.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::auth::model::Account;
use crate::auth::vault::{AesGcmVault, InsecureVault, SecretVault};
use crate::error::{RcError, RcResult};

/// A place that can persist the set of accounts.
pub trait TokenStorage: Send + Sync {
    /// Load all stored accounts (empty list if none / uninitialised).
    fn load(&self) -> RcResult<Vec<Account>>;
    /// Persist the full account set (overwrites previous state).
    fn save(&self, accounts: &[Account]) -> RcResult<()>;
}

/// Volatile, in-memory storage.
pub struct MemoryTokenStorage {
    inner: Mutex<Vec<Account>>,
}

impl MemoryTokenStorage {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
        }
    }
}

impl Default for MemoryTokenStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenStorage for MemoryTokenStorage {
    fn load(&self) -> RcResult<Vec<Account>> {
        Ok(self.inner.lock().unwrap().clone())
    }
    fn save(&self, accounts: &[Account]) -> RcResult<()> {
        *self.inner.lock().unwrap() = accounts.to_vec();
        Ok(())
    }
}

/// Encrypted, on-disk storage. The JSON-serialised account list is sealed by
/// `vault` before being written; `load` unlocks it again.
pub struct FileTokenStorage {
    path: PathBuf,
    vault: Box<dyn SecretVault>,
}

impl FileTokenStorage {
    /// Build with an explicit [`SecretVault`].
    pub fn with_vault(path: impl AsRef<Path>, vault: Box<dyn SecretVault>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            vault,
        }
    }

    /// Build with the identity (insecure) vault. **Dev only** — secrets are
    /// stored in clear text JSON.
    pub fn with_insecure(path: impl AsRef<Path>) -> Self {
        Self::with_vault(path, Box::new(InsecureVault))
    }

    /// Build with AES-256-GCM under `key` (32 bytes).
    pub fn with_aes(path: impl AsRef<Path>, key: &[u8]) -> RcResult<Self> {
        let vault = AesGcmVault::new(key.to_vec())?;
        Ok(Self::with_vault(path, Box::new(vault)))
    }

    /// Build with AES-256-GCM under a [`StaticKeyProvider`].
    pub fn from_key(path: impl AsRef<Path>, key: Vec<u8>) -> RcResult<Self> {
        Self::with_aes(path, &key)
    }

    fn atomic_write(&self, bytes: &[u8]) -> RcResult<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(RcError::Io)?;
        }
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, bytes).map_err(RcError::Io)?;
        std::fs::rename(&tmp, &self.path).map_err(RcError::Io)?;
        Ok(())
    }
}

impl TokenStorage for FileTokenStorage {
    fn load(&self) -> RcResult<Vec<Account>> {
        let raw = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(RcError::Io(e)),
        };
        if raw.is_empty() {
            return Ok(Vec::new());
        }
        let plain = self.vault.unseal(&raw)?;
        let accounts: Vec<Account> = serde_json::from_slice(&plain)
            .map_err(|e| RcError::Auth(format!("corrupt account store: {e}")))?;
        Ok(accounts)
    }

    fn save(&self, accounts: &[Account]) -> RcResult<()> {
        let json = serde_json::to_vec_pretty(accounts)
            .map_err(|e| RcError::Auth(format!("serialize accounts: {e}")))?;
        let sealed = self.vault.seal(&json)?;
        self.atomic_write(&sealed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::offline::offline_account_model;

    #[test]
    fn memory_store_roundtrip() {
        let s = MemoryTokenStorage::new();
        assert!(s.load().unwrap().is_empty());
        let acc = offline_account_model("Steve");
        s.save(std::slice::from_ref(&acc)).unwrap();
        let loaded = s.load().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], acc);
    }

    #[test]
    fn file_store_insecure_roundtrip() {
        let dir = std::env::temp_dir().join("rc_auth_test_insecure");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.json");
        let s = FileTokenStorage::with_insecure(&path);
        let acc = offline_account_model("Alex");
        s.save(std::slice::from_ref(&acc)).unwrap();
        let loaded = s.load().unwrap();
        assert_eq!(loaded, vec![acc]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_store_aes_roundtrip() {
        let dir = std::env::temp_dir().join("rc_auth_test_aes");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("accounts.enc");
        let s = FileTokenStorage::with_aes(&path, &[3u8; 32]).unwrap();
        let acc = offline_account_model("Bob");
        s.save(std::slice::from_ref(&acc)).unwrap();
        // File on disk must NOT contain the username in clear text.
        let raw = std::fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("Bob"));
        let loaded = s.load().unwrap();
        assert_eq!(loaded, vec![acc]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_store_missing_is_empty() {
        let s = FileTokenStorage::with_insecure("/nonexistent/rc/accounts.json");
        assert!(s.load().unwrap().is_empty());
    }
}
