//! Secure storage of account secrets (task 5).
//!
//! Tokens must never be written to disk in clear text. This module defines the
//! [`SecretVault`] abstraction so the rest of the auth subsystem is agnostic to
//! *where* the encryption key lives:
//!
//! * [`InsecureVault`] — identity transform. **Dev / unit-test only**; it does
//!   NOT protect secrets and must never ship in a release build.
//! * [`AesGcmVault`] — AES-256-GCM encryption under a key supplied by a
//!   [`KeyProvider`]. This is the backend used when no platform keystore is
//!   wired in (host tests, dev builds).
//!
//! ## Android integration (Android Keystore)
//!
//! On device the production backend seals the token blob with a key held in the
//! **Android Keystore**. The FFI bridge (`crate::ffi`) constructs a
//! [`SecretVault`] backed by Keystore (the Kotlin side exposes
//! `encrypt`/`decrypt` via `javax.crypto` and a `KeyStore` "AndroidKeyStore"
//! entry) and injects it into [`crate::auth::store::FileTokenStorage`]. The
//! Rust core therefore never sees the raw key — it only ever calls
//! `seal`/`unseal`, which are forwarded to Keystore across the JNI boundary.
//! This matches the `FCLCore/auth` secret-handling boundary.

use crate::error::{RcError, RcResult};

/// Abstraction over a secrets vault: opaque seal/unseal of byte blobs.
pub trait SecretVault: Send + Sync {
    /// Encrypt `plaintext` into an opaque, self-describing blob.
    fn seal(&self, plaintext: &[u8]) -> RcResult<Vec<u8>>;
    /// Reverse [`SecretVault::seal`].
    fn unseal(&self, sealed: &[u8]) -> RcResult<Vec<u8>>;
}

/// Identity vault — stores secrets in clear text. For tests / dev ONLY.
pub struct InsecureVault;

impl SecretVault for InsecureVault {
    fn seal(&self, plaintext: &[u8]) -> RcResult<Vec<u8>> {
        Ok(plaintext.to_vec())
    }
    fn unseal(&self, sealed: &[u8]) -> RcResult<Vec<u8>> {
        Ok(sealed.to_vec())
    }
}

/// Source of the raw symmetric key for [`AesGcmVault`].
pub trait KeyProvider: Send + Sync {
    /// Return the key bytes (should be 32 bytes for AES-256).
    fn key(&self) -> RcResult<Vec<u8>>;
}

/// Fixed in-memory key (tests / explicitly configured keys).
pub struct StaticKeyProvider(pub Vec<u8>);

impl KeyProvider for StaticKeyProvider {
    fn key(&self) -> RcResult<Vec<u8>> {
        Ok(self.0.clone())
    }
}

/// Key read from an environment variable (hex or raw bytes). Used when the
/// Android build injects a Keystore-derived key via an env var for host tests.
pub struct EnvKeyProvider(pub String);

impl KeyProvider for EnvKeyProvider {
    fn key(&self) -> RcResult<Vec<u8>> {
        let v = std::env::var(&self.0)
            .map_err(|e| RcError::Auth(format!("env key {}: {e}", self.0)))?;
        // Accept hex (any even length of hex digits, e.g. 64 chars = 32
        // bytes, or 2 chars = 1 byte) or fall back to raw bytes.
        if v.len() % 2 == 0 && !v.is_empty() && v.chars().all(|c| c.is_ascii_hexdigit()) {
            let bytes = (0..v.len() / 2)
                .map(|i| u8::from_str_radix(&v[2 * i..2 * i + 2], 16).unwrap())
                .collect::<Vec<_>>();
            Ok(bytes)
        } else {
            Ok(v.into_bytes())
        }
    }
}

/// AES-256-GCM encrypted vault. The 12-byte nonce is prepended to the
/// ciphertext so each sealed blob is self-contained.
pub struct AesGcmVault {
    key: Vec<u8>,
}

impl AesGcmVault {
    /// Build directly from raw key material (must be exactly 32 bytes).
    pub fn new(key: Vec<u8>) -> RcResult<Self> {
        if key.len() != 32 {
            return Err(RcError::Auth(format!(
                "AES-256 requires a 32-byte key, got {}",
                key.len()
            )));
        }
        Ok(Self { key })
    }

    /// Build from a [`KeyProvider`] (e.g. a Keystore-backed provider on
    /// Android).
    pub fn with_provider(p: &dyn KeyProvider) -> RcResult<Self> {
        Self::new(p.key()?)
    }

    /// Convenience: read the key from an environment variable.
    pub fn from_env(name: &str) -> RcResult<Self> {
        Self::with_provider(&EnvKeyProvider(name.to_string()))
    }
}

impl SecretVault for AesGcmVault {
    fn seal(&self, plaintext: &[u8]) -> RcResult<Vec<u8>> {
        use aes_gcm::aead::Aead;
        use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| RcError::Auth(format!("cipher init: {e}")))?;
        let mut nonce = [0u8; 12];
        getrandom::getrandom(&mut nonce).map_err(|e| RcError::Auth(format!("rng: {e}")))?;
        let ct = cipher
            .encrypt(
                &Nonce::try_from(&nonce[..]).expect("12-byte nonce"),
                plaintext,
            )
            .map_err(|e| RcError::Auth(format!("encrypt: {e}")))?;
        let mut out = Vec::with_capacity(12 + ct.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    fn unseal(&self, sealed: &[u8]) -> RcResult<Vec<u8>> {
        use aes_gcm::aead::Aead;
        use aes_gcm::{Aes256Gcm, KeyInit, Nonce};

        if sealed.len() < 12 {
            return Err(RcError::Auth("sealed blob too short".into()));
        }
        let (nonce, ct) = sealed.split_at(12);
        let cipher = Aes256Gcm::new_from_slice(&self.key)
            .map_err(|e| RcError::Auth(format!("cipher init: {e}")))?;
        cipher
            .decrypt(&Nonce::try_from(nonce).expect("12-byte nonce"), ct)
            .map_err(|e| RcError::Auth(format!("decrypt (wrong key?): {e}")))
    }
}

/// A thread-safe default vault used by the in-memory manager when no platform
/// vault is supplied. Identity transform — DO NOT use for real secrets.
pub fn default_vault() -> Box<dyn SecretVault> {
    Box::new(InsecureVault)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insecure_roundtrip() {
        let v = InsecureVault;
        let sealed = v.seal(b"secret").unwrap();
        assert_eq!(sealed, b"secret");
        assert_eq!(v.unseal(&sealed).unwrap(), b"secret");
    }

    #[test]
    fn aes_gcm_roundtrip() {
        let key = vec![7u8; 32];
        let v = AesGcmVault::new(key).unwrap();
        let data = b"minecraft-tokens";
        let sealed = v.seal(data).unwrap();
        assert_ne!(sealed, data.to_vec());
        // nonce (12) + ciphertext + AES-GCM tag (16)
        assert_eq!(sealed.len(), 12 + data.len() + 16);
        assert_eq!(v.unseal(&sealed).unwrap(), data);
    }

    #[test]
    fn aes_gcm_wrong_key_fails() {
        let v1 = AesGcmVault::new(vec![1u8; 32]).unwrap();
        let v2 = AesGcmVault::new(vec![2u8; 32]).unwrap();
        let sealed = v1.seal(b"x").unwrap();
        assert!(v2.unseal(&sealed).is_err());
    }

    #[test]
    fn aes_gcm_rejects_bad_key_len() {
        assert!(AesGcmVault::new(vec![1u8; 16]).is_err());
    }

    #[test]
    fn static_key_provider() {
        assert_eq!(StaticKeyProvider(vec![9u8; 32]).key().unwrap().len(), 32);
    }

    #[test]
    fn env_key_provider_hex() {
        std::env::set_var("RC_TEST_KEY", "ab");
        // 2 hex chars -> 1 byte; not 32 -> AesGcmVault::new will reject length,
        // but the provider itself yields 1 byte.
        assert_eq!(
            EnvKeyProvider("RC_TEST_KEY".into()).key().unwrap(),
            vec![0xab]
        );
        std::env::remove_var("RC_TEST_KEY");
    }
}
