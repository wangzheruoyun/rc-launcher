//! Account & authentication (task 5).
//!
//! This module is the Rust-core counterpart of FCL's `FCLCore/auth`
//! subsystem. It implements:
//!
//! * **Microsoft OAuth 2.0 Device Code flow** (`microsoft`): the full
//!   `device_code → poll → XBL → XSTS → Minecraft` token chain required to
//!   authenticate a Bedrock/Microsoft account and obtain a Minecraft Java
//!   edition access token, plus **refresh** of the Microsoft refresh token so
//!   sessions survive without re-consent.
//! * **Offline accounts** (`offline`): username-only accounts with a
//!   deterministic offline UUID (the same scheme the vanilla client uses).
//! * **Secure token storage** (`vault` + `store`): tokens are persisted through
//!   a `SecretVault` abstraction. On Android the production backend seals the
//!   blob with a key held in the **Android Keystore** (see [`vault`]); on the
//!   host an in-memory / AES-GCM file backend is used for tests and dev.
//! * **Account management** (`manager`): add / remove / list / select accounts
//!   and transparently **refresh** Microsoft tokens before they expire.
//!
//! * **Third-party login** (`third_party`): external Yggdrasil auth servers
//!   (Authlib-Injector compatible) and third-party Microsoft-OAuth token relays,
//!   so mainland-China players can log in when Microsoft / Xbox services are
//!   blocked. All flows implement the same `AuthError` / CN-network-fallback
//!   contract as the Microsoft flow.
//!
//! All network access goes through an [`transport::AuthTransport`] trait so the
//! token chain can be exercised with a scripted [`transport::MockTransport`] in
//! unit tests without touching the network (mirroring the `HttpSource` mock
//! pattern used by the `download` module).

pub mod manager;
pub mod microsoft;
pub mod model;
pub mod offline;
pub mod store;
pub mod third_party;
pub mod transport;
pub mod vault;

pub use manager::AccountManager;
pub use microsoft::{DeviceCodeChallenge, MicrosoftTokens, PollOutcome};
pub use model::{
    Account, AccountKind, MicrosoftAccount, OfflineAccount, ThirdPartyAccount, ThirdPartyProvider,
};
pub use offline::offline_uuid;
pub use store::{FileTokenStorage, MemoryTokenStorage, TokenStorage};
pub use third_party::{
    authenticate, discover_server, refresh_third_party, validate_third_party, ThirdPartyLink,
    ThirdPartyLogin, ThirdPartyServerInfo,
};
pub use vault::{
    AesGcmVault, EnvKeyProvider, InsecureVault, KeyProvider, SecretVault, StaticKeyProvider,
};

use thiserror::Error;

/// Errors produced by the authentication subsystem. These are converted into
/// the unified [`crate::error::RcError::Auth`] variant at the FFI boundary.
#[derive(Debug, Error)]
pub enum AuthError {
    #[error("network error: {0}")]
    Network(String),

    #[error("HTTP {status} from {endpoint}: {body}")]
    Http {
        status: u16,
        endpoint: String,
        body: String,
    },

    /// The device-code poll is still pending; the caller should retry after
    /// `seconds`.
    #[error("authorization pending; retry after {seconds}s")]
    Pending { seconds: u64 },

    #[error("the device code has expired; restart the login flow")]
    Expired,

    #[error("authorization denied: {0}")]
    Denied(String),

    #[error("polling too fast; slow down")]
    SlowDown,

    #[error("Xbox / Minecraft account error: {0}")]
    Xbox(String),

    #[error("this Microsoft account does not own Minecraft: {0}")]
    NoMinecraftProfile(String),

    #[error("invalid auth configuration: {0}")]
    Config(String),

    #[error("token storage error: {0}")]
    Storage(String),

    #[error("serialization error: {0}")]
    Json(String),

    #[error("{0}")]
    Other(String),

    /// A third-party (external auth server / token relay) authentication error.
    #[error("third-party auth error: {0}")]
    ThirdParty(String),
}

/// i18n key for the mainland-China network fallback hint surfaced when a login
/// fails due to blocked / throttled access to Microsoft / Xbox / Mojang (or the
/// configured third-party auth server). Resolved through [`crate::i18n`].
pub const CN_FALLBACK_HINT_KEY: &str = "auth.login.cnFallback";

impl AuthError {
    /// Map to the unified core error type.
    pub fn into_rc(self) -> crate::error::RcError {
        match self {
            // Network-level auth failures stay network errors so retry / proxy
            // fallback logic applies identically to the Microsoft flow.
            AuthError::Network(s) => crate::error::RcError::Network(s),
            AuthError::Http {
                status,
                endpoint,
                body,
            } => crate::error::RcError::Auth(format!("HTTP {status} from {endpoint}: {body}")),
            other => crate::error::RcError::Auth(other.to_string()),
        }
    }

    /// True when this error is a network-level failure (so a mainland-China
    /// proxy / mirror fallback hint is actionable).
    pub fn is_network_level(&self) -> bool {
        matches!(self, AuthError::Network(_) | AuthError::Http { .. })
    }

    /// A user-facing hint (i18n-resolved) suggesting proxy / mirror fallback for
    /// mainland-China network conditions. Returns `None` when the error is not a
    /// network failure, so the UI only shows it where it is actually actionable.
    pub fn cn_fallback_hint(&self) -> Option<String> {
        if self.is_network_level() {
            Some(crate::i18n::t(CN_FALLBACK_HINT_KEY))
        } else {
            None
        }
    }
}

impl From<AuthError> for crate::error::RcError {
    fn from(e: AuthError) -> Self {
        e.into_rc()
    }
}

/// Convenience alias used inside the auth subsystem.
pub type AuthResult<T> = Result<T, AuthError>;

impl From<serde_json::Error> for AuthError {
    fn from(e: serde_json::Error) -> Self {
        AuthError::Json(e.to_string())
    }
}

impl From<std::io::Error> for AuthError {
    fn from(e: std::io::Error) -> Self {
        AuthError::Storage(e.to_string())
    }
}

impl From<crate::error::RcError> for AuthError {
    fn from(e: crate::error::RcError) -> Self {
        // Preserve the network-level signal so the mainland-China proxy / mirror
        // fallback hint can be offered for exactly the right failures.
        match e {
            crate::error::RcError::Network(s) => AuthError::Network(s),
            crate::error::RcError::Timeout(s) => AuthError::Network(s),
            crate::error::RcError::Connection(s) => AuthError::Network(s),
            crate::error::RcError::Offline(s) => AuthError::Network(s),
            other => AuthError::Other(other.to_string()),
        }
    }
}
