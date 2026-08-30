//! Third-party account login (task 10).
//!
//! Extends the auth subsystem with the external / third-party authentication
//! services that mainland-China players rely on when Microsoft / Xbox / Mojang
//! services are unreachable or throttled:
//!
//! * **External Yggdrasil auth server (Authlib-Injector compatible)** — a
//!   self-hosted or community "auth server" that speaks the Mojang Yggdrasil
//!   protocol (`/authserver/authenticate`, `/authserver/refresh`,
//!   `/authserver/validate`, `/authserver/invalidate`, `/authserver/signout`)
//!   and publishes an optional metadata document at its root. The launcher
//!   authenticates with a username + password (or a stored access token) and
//!   obtains a game profile (uuid + name). At launch the server URL is handed
//!   to the game through the `authlib-injector` agent
//!   (`-javaagent:authlib-injector.jar=<url>`, user type `authlibInjector`).
//! * **Third-party Microsoft-OAuth token relay** — a broker that mints a
//!   Minecraft-compatible access token from a third-party identity, so a user
//!   who cannot reach `login.microsoftonline.com` directly can still log in.
//!   The relay is a [`ThirdPartyProvider::TokenRelay`] endpoint that exchanges
//!   an opaque third-party code for `{ access_token, uuid, name, expires_in }`.
//!
//! Both flows are written against the same pluggable [`transport::AuthTransport`]
//! trait as the Microsoft flow, so they are unit-tested with a scripted
//! [`MockTransport`] (no network).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::model::{now_secs, Account, ThirdPartyAccount, ThirdPartyProvider};
use crate::auth::transport::{AuthResponse, AuthTransport};
use crate::auth::{AuthError, AuthResult};

/// A link surfaced from an auth server's metadata document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThirdPartyLink {
    pub label: String,
    pub url: String,
}

/// Metadata discovered from an external auth server (`discover_server`).
///
/// Returned to the UI by `AccountManager::begin_third_party` so it can show the
/// server name and a "register" link before the user types credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThirdPartyServerInfo {
    /// Normalised auth-server base URL (what launch passes to authlib-injector).
    pub server_url: String,
    /// Human-friendly server name (from metadata, else the host).
    pub server_name: String,
    /// Optional registration / homepage links surfaced to the UI.
    #[serde(default)]
    pub links: Vec<ThirdPartyLink>,
}

/// Configuration for completing a third-party login (`authenticate`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThirdPartyLogin {
    /// Which kind of third-party provider backs the account.
    pub provider: ThirdPartyProvider,
    /// Auth server base URL (Authlib-Injector) or relay base URL (TokenRelay).
    pub server_url: String,
    /// Username / email for Authlib-Injector; display name for TokenRelay.
    pub username: String,
    /// Password for Authlib-Injector (never persisted in clear text beyond the
    /// encrypted store; only used to mint the access token).
    pub password: String,
    /// An opaque third-party token/code for a [`ThirdPartyProvider::TokenRelay`].
    #[serde(default)]
    pub relay_code: Option<String>,
}

impl ThirdPartyLogin {
    /// Build an Authlib-Injector login request.
    pub fn authlib_injector(
        server_url: impl Into<String>,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            provider: ThirdPartyProvider::AuthlibInjector,
            server_url: server_url.into(),
            username: username.into(),
            password: password.into(),
            relay_code: None,
        }
    }

    /// Build a TokenRelay login request.
    pub fn token_relay(
        server_url: impl Into<String>,
        username: impl Into<String>,
        relay_code: impl Into<String>,
    ) -> Self {
        Self {
            provider: ThirdPartyProvider::TokenRelay,
            server_url: server_url.into(),
            username: username.into(),
            password: String::new(),
            relay_code: Some(relay_code.into()),
        }
    }
}

/// Compute a Yggdrasil endpoint URL for `action` under `server_url`.
///
/// Authlib-Injector servers either expose the Yggdrasil API directly under the
/// configured base (`https://example.com/authserver/...`) or under a sub-path
/// (`https://example.com/.../authserver/...`). We accept both shapes.
fn yggdrasil_url(server_url: &str, action: &str) -> String {
    let base = server_url.trim_end_matches('/');
    if base.ends_with("/authserver") {
        format!("{base}/{action}")
    } else {
        format!("{base}/authserver/{action}")
    }
}

/// Normalise a profile id to a dashed UUID (Yggdrasil returns 32 raw hex).
fn normalize_uuid(raw: &str) -> String {
    let s = raw.trim();
    if s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        format!(
            "{}-{}-{}-{}-{}",
            &s[0..8],
            &s[8..12],
            &s[12..16],
            &s[16..20],
            &s[20..32]
        )
    } else {
        s.to_string()
    }
}

/// A stable, random client token for the Yggdrasil `clientToken` field.
fn client_token() -> String {
    let mut b = [0u8; 16];
    let _ = getrandom::getrandom(&mut b);
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

fn host_of(url: &str) -> String {
    url.split("//")
        .nth(1)
        .map(|rest| rest.split('/').next().unwrap_or(rest).to_string())
        .unwrap_or_else(|| url.to_string())
}

fn field_str<'a>(v: &'a Value, key: &str) -> AuthResult<&'a str> {
    v.get(key).and_then(|x| x.as_str()).ok_or_else(|| {
        AuthError::ThirdParty(format!("missing string field `{key}` in auth response"))
    })
}

fn first_profile<'a>(v: &'a Value) -> AuthResult<&'a Value> {
    v.get("selectedProfile")
        .or_else(|| {
            v.get("availableProfiles")
                .and_then(|a| a.as_array())
                .and_then(|a| a.first())
        })
        .ok_or_else(|| AuthError::ThirdParty("auth server returned no game profile".into()))
}

fn yggdrasil_err(resp: &AuthResponse, action: &str) -> AuthError {
    let detail = resp
        .body
        .get("error")
        .and_then(|v| v.as_str())
        .or_else(|| resp.body.get("errorMessage").and_then(|v| v.as_str()))
        .or_else(|| resp.body.get("message").and_then(|v| v.as_str()))
        .unwrap_or("authentication failed");
    AuthError::ThirdParty(format!(
        "{} failed (HTTP {}): {}",
        action, resp.status, detail
    ))
}

fn relay_err(resp: &AuthResponse, action: &str) -> AuthError {
    let detail = resp
        .body
        .get("error")
        .and_then(|v| v.as_str())
        .or_else(|| resp.body.get("message").and_then(|v| v.as_str()))
        .unwrap_or("relay rejected the request");
    AuthError::ThirdParty(format!(
        "token relay {} failed (HTTP {}): {}",
        action, resp.status, detail
    ))
}

/// Discover metadata from an external auth server.
///
/// Tolerates servers that do not publish a metadata document (we fall back to
/// the host name) so the UI can always offer the login, even for minimal
/// community servers.
pub async fn discover_server(
    t: &dyn AuthTransport,
    server_url: &str,
) -> AuthResult<ThirdPartyServerInfo> {
    let base = server_url.trim().trim_end_matches('/').to_string();
    if base.is_empty() {
        return Err(AuthError::Config("auth server url is empty".into()));
    }
    let resp = t.get_json(&base, None).await?;
    let (name, links) = if resp.is_success() {
        let name = resp
            .body
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| host_of(&base));
        let links = parse_links(&resp.body);
        (name, links)
    } else {
        (host_of(&base), Vec::new())
    };
    Ok(ThirdPartyServerInfo {
        server_url: base,
        server_name: name,
        links,
    })
}

fn parse_links(body: &Value) -> Vec<ThirdPartyLink> {
    let mut out = Vec::new();
    // authlib-injector style: meta.links = { "register": "https://...", ... }
    if let Some(links) = body
        .get("meta")
        .and_then(|m| m.get("links"))
        .and_then(|l| l.as_object())
    {
        for (label, url) in links {
            if let Some(url) = url.as_str() {
                out.push(ThirdPartyLink {
                    label: label.clone(),
                    url: url.to_string(),
                });
            }
        }
    }
    // Also accept a top-level `links` object.
    if let Some(links) = body.get("links").and_then(|l| l.as_object()) {
        for (label, url) in links {
            if let Some(url) = url.as_str() {
                out.push(ThirdPartyLink {
                    label: label.clone(),
                    url: url.to_string(),
                });
            }
        }
    }
    out
}

/// Authenticate a third-party login, returning a fully-populated account.
pub async fn authenticate(
    t: &dyn AuthTransport,
    login: &ThirdPartyLogin,
) -> AuthResult<ThirdPartyAccount> {
    if login.server_url.trim().is_empty() {
        return Err(AuthError::Config("auth server url is empty".into()));
    }
    match login.provider {
        ThirdPartyProvider::AuthlibInjector => authenticate_yggdrasil(t, login).await,
        ThirdPartyProvider::TokenRelay => authenticate_relay(t, login).await,
    }
}

async fn authenticate_yggdrasil(
    t: &dyn AuthTransport,
    login: &ThirdPartyLogin,
) -> AuthResult<ThirdPartyAccount> {
    if login.username.trim().is_empty() || login.password.is_empty() {
        return Err(AuthError::Config(
            "authlib-injector login requires a username and password".into(),
        ));
    }
    let body = json!({
        "username": login.username,
        "password": login.password,
        // The client token is echoed back by the server; we default to a random
        // one and then adopt the server's value below.
        "clientToken": client_token(),
        "requestUser": true,
        "agent": { "name": "Minecraft", "version": 1 },
    });
    let url = yggdrasil_url(&login.server_url, "authenticate");
    let resp = t.post_json(&url, &body).await?;
    if !resp.is_success() {
        return Err(yggdrasil_err(&resp, "authenticate"));
    }
    let v = &resp.body;
    let access_token = field_str(v, "accessToken")?.to_string();
    let profile = first_profile(v)?;
    let uuid = normalize_uuid(field_str(profile, "id")?);
    let username = field_str(profile, "name")?.to_string();
    // Prefer the server-returned client token so refreshes are stable.
    let client = v
        .get("clientToken")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(client_token);
    Ok(ThirdPartyAccount {
        uuid,
        username,
        provider: ThirdPartyProvider::AuthlibInjector,
        server_url: login.server_url.trim_end_matches('/').to_string(),
        server_name: String::new(),
        access_token,
        client_token: client,
        expires_at: 0,
        relay_payload: None,
    })
}

async fn authenticate_relay(
    t: &dyn AuthTransport,
    login: &ThirdPartyLogin,
) -> AuthResult<ThirdPartyAccount> {
    let code = login
        .relay_code
        .clone()
        .ok_or_else(|| AuthError::Config("token relay login requires a relay code".into()))?;
    let base = login.server_url.trim_end_matches('/').to_string();
    let body = json!({ "code": code, "username": login.username });
    let resp = t.post_json(&format!("{base}/exchange"), &body).await?;
    if !resp.is_success() {
        return Err(relay_err(&resp, "exchange"));
    }
    let v = &resp.body;
    let access_token = field_str(v, "access_token")?.to_string();
    let uuid = normalize_uuid(field_str(v, "uuid")?);
    let username = field_str(v, "name")?.to_string();
    let expires_at = v
        .get("expires_in")
        .and_then(|x| x.as_u64())
        .map(|e| now_secs().saturating_add(e))
        .unwrap_or(0);
    Ok(ThirdPartyAccount {
        uuid,
        username,
        provider: ThirdPartyProvider::TokenRelay,
        server_url: base,
        server_name: String::new(),
        access_token,
        client_token: String::new(),
        expires_at,
        relay_payload: Some(code),
    })
}

/// Refresh a stored third-party account from its access / client token.
///
/// Authlib-Injector servers support the Yggdrasil `refresh` endpoint; token
/// relays support a `/refresh` endpoint. Both re-mint the access token in place
/// (the profile uuid / name are preserved).
pub async fn refresh_third_party(
    t: &dyn AuthTransport,
    account: &ThirdPartyAccount,
) -> AuthResult<ThirdPartyAccount> {
    match account.provider {
        ThirdPartyProvider::AuthlibInjector => {
            if account.client_token.is_empty() || account.access_token.is_empty() {
                return Err(AuthError::Config(
                    "missing client/access token; please log in again".into(),
                ));
            }
            let body = json!({
                "accessToken": account.access_token,
                "clientToken": account.client_token,
                "requestUser": true,
                "selectedProfile": { "id": account.uuid.replace('-', ""), "name": account.username },
            });
            let url = yggdrasil_url(&account.server_url, "refresh");
            let resp = t.post_json(&url, &body).await?;
            if !resp.is_success() {
                return Err(yggdrasil_err(&resp, "refresh"));
            }
            let v = &resp.body;
            let access_token = field_str(v, "accessToken")?.to_string();
            let profile = first_profile(v)?;
            let uuid = normalize_uuid(field_str(profile, "id")?);
            let username = field_str(profile, "name")?.to_string();
            Ok(ThirdPartyAccount {
                access_token,
                uuid,
                username,
                ..account.clone()
            })
        }
        ThirdPartyProvider::TokenRelay => {
            let base = account.server_url.trim_end_matches('/').to_string();
            let body = json!({
                "access_token": account.access_token,
                "refresh_token": account.relay_payload,
            });
            let resp = t.post_json(&format!("{base}/refresh"), &body).await?;
            if !resp.is_success() {
                return Err(relay_err(&resp, "refresh"));
            }
            let v = &resp.body;
            let access_token = field_str(v, "access_token")?.to_string();
            let expires_at = v
                .get("expires_in")
                .and_then(|x| x.as_u64())
                .map(|e| now_secs().saturating_add(e))
                .unwrap_or(account.expires_at);
            let mut a = account.clone();
            a.access_token = access_token;
            a.expires_at = expires_at;
            Ok(a)
        }
    }
}

/// Validate a stored third-party account's access token.
///
/// Returns `false` (not an error) when the server reports the token as invalid
/// (HTTP 403) so the manager can transparently re-authenticate. Token relays
/// without a validate endpoint treat a non-empty token as valid.
pub async fn validate_third_party(
    t: &dyn AuthTransport,
    account: &ThirdPartyAccount,
) -> AuthResult<bool> {
    match account.provider {
        ThirdPartyProvider::AuthlibInjector => {
            if account.access_token.is_empty() {
                return Ok(false);
            }
            let body = json!({
                "accessToken": account.access_token,
                "clientToken": account.client_token,
            });
            let url = yggdrasil_url(&account.server_url, "validate");
            let resp = t.post_json(&url, &body).await?;
            if resp.status == 204 || resp.is_success() {
                Ok(true)
            } else if resp.status == 403 {
                Ok(false)
            } else {
                Err(yggdrasil_err(&resp, "validate"))
            }
        }
        ThirdPartyProvider::TokenRelay => Ok(!account.access_token.is_empty()),
    }
}

/// Build an [`Account::ThirdParty`] from a freshly authenticated account.
pub fn into_account(a: ThirdPartyAccount) -> Account {
    Account::ThirdParty(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::transport::MockTransport;
    use serde_json::json;

    fn yggdrasil_server() -> String {
        "https://auth.example.com".to_string()
    }

    #[tokio::test]
    async fn discover_with_metadata() {
        let m = MockTransport::new();
        m.script_ok(
            &yggdrasil_server(),
            json!({
                "name": "Example Auth",
                "meta": { "links": { "register": "https://auth.example.com/reg", "homepage": "https://auth.example.com" } }
            }),
        );
        let info = discover_server(&m, &yggdrasil_server()).await.unwrap();
        assert_eq!(info.server_name, "Example Auth");
        assert_eq!(info.links.len(), 2);
        // serde_json Map sorts keys, so assert on the set of labels, not order.
        let labels: Vec<&str> = info.links.iter().map(|l| l.label.as_str()).collect();
        assert!(labels.contains(&"register"));
        assert!(labels.contains(&"homepage"));
    }

    #[tokio::test]
    async fn discover_without_metadata_falls_back_to_host() {
        let m = MockTransport::new();
        m.script(&yggdrasil_server(), 404, json!({ "error": "nf" }));
        let info = discover_server(&m, &yggdrasil_server()).await.unwrap();
        assert_eq!(info.server_name, "auth.example.com");
        assert!(info.links.is_empty());
    }

    #[tokio::test]
    async fn authenticate_yggdrasil_parses_profile() {
        let m = MockTransport::new();
        m.script_ok(
            &yggdrasil_url(&yggdrasil_server(), "authenticate"),
            json!({
                "accessToken": "AT",
                "clientToken": "CT",
                "availableProfiles": [ { "id": "b50ad385829d3141a2167e7d7539ba7f", "name": "Notch" } ],
                "selectedProfile": { "id": "b50ad385829d3141a2167e7d7539ba7f", "name": "Notch" }
            }),
        );
        let login = ThirdPartyLogin::authlib_injector(&yggdrasil_server(), "Notch", "pw");
        let acc = authenticate(&m, &login).await.unwrap();
        assert_eq!(acc.uuid, "b50ad385-829d-3141-a216-7e7d7539ba7f");
        assert_eq!(acc.username, "Notch");
        assert_eq!(acc.access_token, "AT");
        assert_eq!(acc.client_token, "CT");
        assert_eq!(acc.provider, ThirdPartyProvider::AuthlibInjector);
    }

    #[tokio::test]
    async fn authenticate_yggdrasil_bad_password_is_third_party_error() {
        let m = MockTransport::new();
        m.script_err(
            &yggdrasil_url(&yggdrasil_server(), "authenticate"),
            "illegal_password",
            "bad",
        );
        let login = ThirdPartyLogin::authlib_injector(&yggdrasil_server(), "x", "wrong");
        let err = authenticate(&m, &login).await.unwrap_err();
        assert!(matches!(err, AuthError::ThirdParty(_)));
    }

    #[tokio::test]
    async fn refresh_yggdrasil_mints_new_token() {
        let m = MockTransport::new();
        m.script_ok(
            &yggdrasil_url(&yggdrasil_server(), "refresh"),
            json!({
                "accessToken": "AT2",
                "clientToken": "CT",
                "selectedProfile": { "id": "b50ad385829d3141a2167e7d7539ba7f", "name": "Notch" }
            }),
        );
        let acc = ThirdPartyAccount {
            uuid: "b50ad385-829d-3141-a216-7e7d7539ba7f".into(),
            username: "Notch".into(),
            provider: ThirdPartyProvider::AuthlibInjector,
            server_url: yggdrasil_server(),
            server_name: String::new(),
            access_token: "AT".into(),
            client_token: "CT".into(),
            expires_at: 0,
            relay_payload: None,
        };
        let refreshed = refresh_third_party(&m, &acc).await.unwrap();
        assert_eq!(refreshed.access_token, "AT2");
        assert_eq!(refreshed.uuid, acc.uuid);
    }

    #[tokio::test]
    async fn validate_yggdrasil_403_is_false() {
        let m = MockTransport::new();
        m.script(
            &yggdrasil_url(&yggdrasil_server(), "validate"),
            403,
            json!({}),
        );
        let acc = ThirdPartyAccount {
            uuid: "u".into(),
            username: "n".into(),
            provider: ThirdPartyProvider::AuthlibInjector,
            server_url: yggdrasil_server(),
            server_name: String::new(),
            access_token: "AT".into(),
            client_token: "CT".into(),
            expires_at: 0,
            relay_payload: None,
        };
        assert!(!validate_third_party(&m, &acc).await.unwrap());
    }

    #[tokio::test]
    async fn authenticate_relay_returns_minecraft_token() {
        let m = MockTransport::new();
        m.script_ok(
            "https://relay.example.com/exchange",
            json!({ "access_token": "MC", "uuid": "b50ad385829d3141a2167e7d7539ba7f", "name": "RelayPlayer", "expires_in": 3600 }),
        );
        let login = ThirdPartyLogin::token_relay(
            "https://relay.example.com",
            "RelayPlayer",
            "third-party-code",
        );
        let acc = authenticate(&m, &login).await.unwrap();
        assert_eq!(acc.provider, ThirdPartyProvider::TokenRelay);
        assert_eq!(acc.access_token, "MC");
        assert!(acc.expires_at > 0);
        assert_eq!(acc.relay_payload.as_deref(), Some("third-party-code"));
    }

    #[tokio::test]
    async fn network_failure_surfaces_as_network_error() {
        // A real network failure (connection refused / timeout) arrives at the auth
        // layer as `RcError::Network` and must be preserved as a network-level
        // `AuthError` so the mainland-China proxy / mirror fallback hint fires.
        let rc = crate::error::RcError::Network("connection refused to auth server".into());
        let auth_err: AuthError = rc.into();
        assert!(
            auth_err.is_network_level(),
            "expected network-level error, got {auth_err:?}"
        );

        // The same must hold for timeout / connection / offline variants.
        for rc in [
            crate::error::RcError::Timeout("t".into()),
            crate::error::RcError::Connection("c".into()),
            crate::error::RcError::Offline("o".into()),
        ] {
            assert!(AuthError::from(rc).is_network_level());
        }

        // A non-network auth error (e.g. bad credentials) must NOT be flagged as
        // network-level, so the hint is not shown where it is unactionable.
        assert!(!AuthError::ThirdParty("bad password".into()).is_network_level());
    }

    #[test]
    fn endpoint_url_shapes() {
        assert_eq!(
            yggdrasil_url("https://a.com", "authenticate"),
            "https://a.com/authserver/authenticate"
        );
        assert_eq!(
            yggdrasil_url("https://a.com/authserver/", "refresh"),
            "https://a.com/authserver/refresh"
        );
    }
}
