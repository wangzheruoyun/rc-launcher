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
    /// A third-party account (external Yggdrasil auth server / token relay).
    ThirdParty,
}

impl AccountKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AccountKind::Microsoft => "microsoft",
            AccountKind::Offline => "offline",
            AccountKind::ThirdParty => "thirdparty",
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

/// Which kind of third-party authentication backs a [`ThirdPartyAccount`].
///
/// External Yggdrasil auth servers (Authlib-Injector compatible) and
/// third-party Microsoft-OAuth token relays are the two flavours used by
/// mainland-China players when Microsoft / Xbox services are unreachable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThirdPartyProvider {
    /// An external Yggdrasil auth server (Authlib-Injector compatible).
    AuthlibInjector,
    /// A third-party Microsoft-OAuth token relay (brokers a Minecraft token
    /// from a third-party identity).
    TokenRelay,
}

/// A third-party (external auth server / token relay) account.
///
/// The access token is what the game receives (through the authlib-injector
/// agent at launch for Authlib-Injector, or directly for a token relay). The
/// `server_url` is the auth server base handed to authlib-injector; `relay_payload`
/// holds the opaque third-party code for a [`ThirdPartyProvider::TokenRelay`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThirdPartyAccount {
    /// Game profile UUID (the game login identity). Normalised to dashed form.
    pub uuid: String,
    /// Game profile name (the in-game username).
    pub username: String,
    /// Which kind of provider backs this account.
    pub provider: ThirdPartyProvider,
    /// Auth server base URL (handed to authlib-injector at launch).
    pub server_url: String,
    /// Human-friendly server name (from metadata, or the host).
    pub server_name: String,
    /// Yggdrasil / relay access token used by the game to log in.
    pub access_token: String,
    /// Yggdrasil client token (stable across refreshes; empty for token relays).
    pub client_token: String,
    /// Unix epoch seconds when `access_token` expires (0 = unknown / no expiry
    /// reported by the server).
    pub expires_at: u64,
    /// Opaque third-party code for a token relay (so a relay refresh is possible).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay_payload: Option<String>,
}

impl ThirdPartyAccount {
    /// True when the access token is missing or (when the server reports one)
    /// past its expiry.
    pub fn is_expired(&self, now: u64) -> bool {
        self.access_token.is_empty() || (self.expires_at > 0 && now >= self.expires_at)
    }

    /// Build a redacted clone that never hits disk / crosses the FFI boundary
    /// with live secrets.
    pub fn summary(&self) -> ThirdPartyAccount {
        ThirdPartyAccount {
            uuid: self.uuid.clone(),
            username: self.username.clone(),
            provider: self.provider,
            server_url: self.server_url.clone(),
            server_name: self.server_name.clone(),
            access_token: String::new(),
            client_token: String::new(),
            expires_at: self.expires_at,
            relay_payload: None,
        }
    }
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
    /// A third-party (external auth server / token relay) account.
    ThirdParty(ThirdPartyAccount),
}

impl Account {
    pub fn kind(&self) -> AccountKind {
        match self {
            Account::Microsoft(_) => AccountKind::Microsoft,
            Account::Offline(_) => AccountKind::Offline,
            Account::ThirdParty(_) => AccountKind::ThirdParty,
        }
    }

    pub fn uuid(&self) -> &str {
        match self {
            Account::Microsoft(a) => &a.uuid,
            Account::Offline(a) => &a.uuid,
            Account::ThirdParty(a) => &a.uuid,
        }
    }

    pub fn username(&self) -> &str {
        match self {
            Account::Microsoft(a) => &a.username,
            Account::Offline(a) => &a.username,
            Account::ThirdParty(a) => &a.username,
        }
    }

    /// Build a redacted copy safe to expose to the UI/FFI layer.
    pub fn summary(&self) -> Account {
        match self {
            Account::Microsoft(a) => Account::Microsoft(a.summary()),
            Account::Offline(a) => Account::Offline(a.clone()),
            Account::ThirdParty(a) => Account::ThirdParty(a.summary()),
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

// === Skin model (task 22) ==================================================

/// Source of a player skin texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkinSource {
    /// Official skin/cape downloaded from Mojang's session server.
    Official,
    /// Custom skin uploaded by the player.
    Custom,
}

impl SkinSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            SkinSource::Official => "official",
            SkinSource::Custom => "custom",
        }
    }
}

/// Skin (and cape) metadata for a Minecraft profile (task 22).
///
/// Fetched from Mojang's session profile API
/// (`https://sessionserver.mojang.com/session/profile/{uuid}?unsigned=false`)
/// which returns a `textures` property containing the skin and cape download URLs.
/// The actual PNG bytes are fetched by the UI from `skin_url` / `cape_url` and
/// cached locally for offline display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkinModel {
    /// Player UUID this skin belongs to.
    pub uuid: String,
    /// URL to the skin PNG (64x64 or 64x32). Fetched from Mojang's texture servers.
    pub skin_url: String,
    /// URL to the cape PNG ("抛羽翅"), if the player owns one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cape_url: Option<String>,
    /// Unix epoch seconds when the model was last refreshed from the server.
    pub fetched_at: u64,
    /// When the skin PNG bytes were last cached locally (0 = not cached).
    #[serde(default)]
    pub cached_at: u64,
    /// Where this skin came from.
    pub source: SkinSource,
    /// SHA-256 hash of the cached skin PNG bytes (for integrity / dedup).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    /// The Minecraft "model" field from the profile texture payload: `"slim"`
    /// (Alex) or `"default"` (Steve). Empty string when unknown.
    pub model: String,
}

impl SkinModel {
    /// True when the skin data has been cached locally (offline displayable).
    pub fn is_cached(&self) -> bool {
        self.cached_at > 0 && !self.skin_url.is_empty()
    }

    /// Cache key derived from the UUID — used by the UI to look up the cached
    /// PNG bytes on disk.
    pub fn cache_key(&self) -> String {
        format!("skin_{}", self.uuid.replace('-', ""))
    }
}

#[cfg(test)]
mod skin_tests {
    use super::*;

    #[test]
    fn skin_model_serialises_roundtrip() {
        let s = SkinModel {
            uuid: "abc-123".into(),
            skin_url: "https://textures.minecraft.net/texture/x".into(),
            cape_url: Some("https://textures.minecraft.net/texture/y".into()),
            fetched_at: 100,
            cached_at: 90,
            source: SkinSource::Official,
            hash: Some("deadbeef".into()),
            model: "slim".into(),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: SkinModel = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.source.as_str(), "official");
    }

    #[test]
    fn skin_model_is_cached_respects_cache() {
        let uncached = SkinModel {
            uuid: "u".into(),
            skin_url: "https://x".into(),
            cape_url: None,
            fetched_at: 100,
            cached_at: 0,
            source: SkinSource::Official,
            hash: None,
            model: String::new(),
        };
        assert!(!uncached.is_cached());

        let cached = SkinModel {
            cached_at: 100,
            ..uncached.clone()
        };
        assert!(cached.is_cached());
    }

    #[test]
    fn skin_model_default_cape_url_is_none() {
        let json = r#"{"uuid":"u","skin_url":"https://x","fetched_at":1,"cached_at":0,"source":"official","model":""}"#;
        let s: SkinModel = serde_json::from_str(json).unwrap();
        assert!(s.cape_url.is_none());
        assert!(s.hash.is_none());
    }

    #[test]
    fn skin_source_as_str() {
        assert_eq!(SkinSource::Official.as_str(), "official");
        assert_eq!(SkinSource::Custom.as_str(), "custom");
    }
}
