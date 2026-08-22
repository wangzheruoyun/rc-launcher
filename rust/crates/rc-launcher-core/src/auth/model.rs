//! Account data models shared across the auth subsystem.
//!
//! Everything here is `serde`-serialisable so accounts can be persisted by the
//! [`crate::auth::store`] backends and exchanged with the Compose/FFI layer as
//! JSON.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Discriminator for the account type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountKind {
    Microsoft,
    Offline,
}

impl AccountKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AccountKind::Microsoft => "microsoft",
            AccountKind::Offline => "offline",
        }
    }
}

/// A Microsoft (Mojang) account authenticated through the device-code flow.
///
/// We persist only the long-lived credentials. The short-lived XBL/XSTS tokens
/// are recomputed from the Microsoft access/refresh token on demand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicrosoftAccount {
    /// Minecraft Java profile UUID (the game login identity).
    pub uuid: String,
    /// Minecraft Java profile name (the in-game username).
    pub username: String,
    /// OAuth client id used for this account.
    pub client_id: String,
    /// Minecraft services access token (used by the launcher to start the game).
    pub access_token: String,
    /// Microsoft refresh token (long-lived; used to mint new access tokens).
    pub refresh_token: String,
    /// Xbox Live / Microsoft user id (used for skins etc.).
    pub xuid: Option<String>,
    /// Unix epoch seconds when [`MicrosoftAccount::access_token`] expires.
    pub expires_at: u64,
    /// Unix epoch seconds when the underlying Microsoft access token expires
    /// (drives proactive refresh of the whole chain).
    pub ms_expires_at: u64,
}

impl MicrosoftAccount {
    /// True when the Minecraft access token is (or will soon be) invalid.
    pub fn is_expired(&self, now: u64) -> bool {
        now >= self.expires_at
    }

    /// True when the token should be refreshed: it is expired or within
    /// `threshold` seconds of expiry (proactive refresh).
    pub fn needs_refresh(&self, now: u64, threshold: u64) -> bool {
        now.saturating_add(threshold) >= self.expires_at
            || now.saturating_add(threshold) >= self.ms_expires_at
    }

    /// Produce a redacted clone that never hits disk / crosses the FFI
    /// boundary with live secrets (useful for UI lists).
    pub fn summary(&self) -> MicrosoftAccount {
        MicrosoftAccount {
            uuid: self.uuid.clone(),
            username: self.username.clone(),
            client_id: self.client_id.clone(),
            access_token: String::new(),
            refresh_token: String::new(),
            xuid: self.xuid.clone(),
            expires_at: self.expires_at,
            ms_expires_at: self.ms_expires_at,
        }
    }
}

/// An offline (cracked / no-network) account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfflineAccount {
    /// Deterministic offline UUID derived from the username.
    pub uuid: String,
    /// The chosen username shown in-game.
    pub username: String,
}

/// A unified account: either Microsoft-authenticated or offline.
///
/// Serialised with a `type` tag (`"microsoft"` / `"offline"`) so the stored
/// JSON is self-describing and round-trips through [`crate::auth::store`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Account {
    Microsoft(MicrosoftAccount),
    Offline(OfflineAccount),
}

impl Account {
    pub fn kind(&self) -> AccountKind {
        match self {
            Account::Microsoft(_) => AccountKind::Microsoft,
            Account::Offline(_) => AccountKind::Offline,
        }
    }

    pub fn uuid(&self) -> &str {
        match self {
            Account::Microsoft(a) => &a.uuid,
            Account::Offline(a) => &a.uuid,
        }
    }

    pub fn username(&self) -> &str {
        match self {
            Account::Microsoft(a) => &a.username,
            Account::Offline(a) => &a.username,
        }
    }

    /// Build a redacted copy safe to expose to the UI/FFI layer.
    pub fn summary(&self) -> Account {
        match self {
            Account::Microsoft(a) => Account::Microsoft(a.summary()),
            Account::Offline(a) => Account::Offline(a.clone()),
        }
    }
}

/// Current unix epoch seconds. Centralised so tests can inject a fixed clock
/// if needed (via the callers passing `now`).
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_account_serialises_with_tag() {
        let a = Account::Offline(OfflineAccount {
            uuid: "abc".into(),
            username: "Steve".into(),
        });
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"type\":\"offline\""));
        let back: Account = serde_json::from_str(&json).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn microsoft_account_tag_and_roundtrip() {
        let a = Account::Microsoft(MicrosoftAccount {
            uuid: "uuid".into(),
            username: "Notch".into(),
            client_id: "cid".into(),
            access_token: "at".into(),
            refresh_token: "rt".into(),
            xuid: Some("123".into()),
            expires_at: 100,
            ms_expires_at: 90,
        });
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"type\":\"microsoft\""));
        let back: Account = serde_json::from_str(&json).unwrap();
        assert_eq!(back, a);
        if let Account::Microsoft(m) = back {
            assert!(m.needs_refresh(80, 30));
            assert!(!m.needs_refresh(10, 30));
            assert!(m.is_expired(100));
        } else {
            panic!("expected microsoft");
        }
    }
}
