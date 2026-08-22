//! Offline (no-network / "cracked") accounts (task 5).
//!
//! Offline accounts need no server interaction: they carry just a username and
//! a deterministic UUID derived from it — the exact scheme the vanilla client
//! uses (`UUID.nameUUIDFromBytes("OfflinePlayer:<name>")`, a version-3 UUID
//! with no namespace).

use md5::{Digest, Md5};

use crate::auth::model::{Account, OfflineAccount};

/// Compute the offline UUID for a username, matching the vanilla client.
///
/// The vanilla client does `UUID.nameUUIDFromBytes(("OfflinePlayer:" + name)
/// .getBytes(UTF_8))`, which MD5-hashes the bytes and then forces the
/// version (0x30) and RFC-4122 variant (0x80) bits.
pub fn offline_uuid(username: &str) -> String {
    let input = format!("OfflinePlayer:{username}");
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    let digest = hasher.finalize();

    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest);

    // version = 3 (name-based)
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    // variant = RFC 4122
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

/// Create an offline account for `username`.
pub fn offline_account(username: &str) -> OfflineAccount {
    OfflineAccount {
        uuid: offline_uuid(username),
        username: username.to_string(),
    }
}

/// Build an [`Account::Offline`] for `username`.
pub fn offline_account_model(username: &str) -> Account {
    Account::Offline(offline_account(username))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_stable() {
        let a = offline_uuid("Steve");
        let b = offline_uuid("Steve");
        assert_eq!(a, b);
        assert_ne!(a, offline_uuid("Alex"));
        // version nibble must be 3, variant nibble must be 8/a/b/c.
        assert_eq!(&a[14..15], "3");
        assert!(["8", "9", "a", "b", "c"].contains(&&a[19..20]));
    }

    #[test]
    fn known_vector() {
        // The well-known offline UUID for "Notch" used across launchers.
        let u = offline_uuid("Notch");
        assert_eq!(u, "b50ad385-829d-3141-a216-7e7d7539ba7f");
    }

    #[test]
    fn builds_model() {
        let a = offline_account_model("Steve");
        match a {
            Account::Offline(o) => {
                assert_eq!(o.username, "Steve");
                assert_eq!(o.uuid, offline_uuid("Steve"));
            }
            _ => panic!("expected offline account"),
        }
    }
}
