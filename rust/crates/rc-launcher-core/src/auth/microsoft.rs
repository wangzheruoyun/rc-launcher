//! Microsoft OAuth 2.0 Device Code flow + the Xbox Live / Minecraft token
//! chain, plus refresh (task 5).
//!
//! Sequence (mirrors the official Minecraft Java launcher + FCL's
//! `MicrosoftService`):
//!
//! 1. `request_device_code`  → ask Microsoft for a `user_code` / `device_code`.
//! 2. `poll_token`           → poll until the user finishes consent; yields a
//!    Microsoft access + refresh token.
//! 3. `xbl_authenticate`     → exchange the MS token for an Xbox Live token.
//! 4. `xsts_authorize`       → exchange the XBL token for an XSTS token (+uhs).
//! 5. `login_with_xbox`      → exchange XSTS for a Minecraft access token.
//! 6. `fetch_profile`        → fetch the Minecraft Java profile (uuid + name).
//!
//! `authenticate_device_code` orchestrates 2–6; `refresh_account` re-runs
//! 2-style refresh + 3–6 to mint a fresh Minecraft token from a stored
//! refresh token.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

use crate::auth::model::{now_secs, MicrosoftAccount};
use crate::auth::transport::AuthTransport;
use crate::auth::AuthError;
use crate::auth::AuthResult;

/// Microsoft consumer tenant authority (personal accounts).
pub const MS_AUTHORITY: &str = "https://login.microsoftonline.com/consumers";
/// Endpoint that issues a device code.
pub const DEVICE_CODE_URL: &str =
    "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
/// Endpoint that redeems a device code / refresh token for tokens.
pub const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
/// Xbox Live user authentication.
pub const XBL_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
/// Xbox Live Secure Token Service.
pub const XSTS_AUTH_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
/// Minecraft services login-with-xbox.
pub const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
/// Minecraft services profile endpoint.
pub const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

/// Public MSA client id shared by many open-source launchers. Override per
/// account if you register your own Azure AD application.
pub const DEFAULT_CLIENT_ID: &str = "00000000402b5328";

/// OAuth scope required for Minecraft: sign-in + offline (refresh) access.
pub const DEFAULT_SCOPE: &str = "XboxLive.signin offline_access";

/// The challenge the UI must show to the user (copy the `message` verbatim).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCodeChallenge {
    /// Short code the user enters at the verification URL.
    pub user_code: String,
    /// Opaque device code used while polling.
    pub device_code: String,
    /// Where the user signs in.
    pub verification_uri: String,
    /// Seconds until the device code expires.
    pub expires_in: u64,
    /// Recommended polling interval in seconds.
    pub interval: u64,
    /// Human-readable instruction (already localized by Microsoft).
    pub message: String,
}

/// Outcome of a single device-code poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollOutcome {
    /// Still pending — poll again after `retry_after` seconds.
    Pending { retry_after: u64 },
    /// The device code expired; restart the flow.
    Expired,
    /// The user denied consent.
    Denied(String),
    /// Polling too fast; increase the interval.
    SlowDown,
    /// Completed — tokens in hand.
    Completed(MicrosoftTokens),
}

/// Microsoft tokens returned by the token endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicrosoftTokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Seconds the access token is valid for.
    pub expires_in: u64,
}

/// Step 1: request a device code.
pub async fn request_device_code(
    t: &dyn AuthTransport,
    client_id: &str,
    scope: &str,
) -> AuthResult<DeviceCodeChallenge> {
    if client_id.is_empty() {
        return Err(AuthError::Config("client_id is empty".into()));
    }
    let resp = t
        .post_form(
            DEVICE_CODE_URL,
            &[("client_id", client_id), ("scope", scope)],
        )
        .await?;
    let body = resp.into_value()?;
    let challenge = DeviceCodeChallenge {
        user_code: field_str(&body, "user_code")?,
        device_code: field_str(&body, "device_code")?,
        verification_uri: field_str(&body, "verification_uri")?,
        expires_in: field_u64(&body, "expires_in")?,
        interval: field_u64(&body, "interval").unwrap_or(5),
        message: field_str(&body, "message").unwrap_or_default(),
    };
    Ok(challenge)
}

/// Step 2 (one poll): redeem the device code. Returns a [`PollOutcome`] so the
/// caller can surface `Pending` to the UI and retry, or handle `Expired` /
/// `Denied` / `SlowDown`.
pub async fn poll_token(
    t: &dyn AuthTransport,
    client_id: &str,
    device_code: &str,
) -> AuthResult<PollOutcome> {
    let resp = t
        .post_form(
            TOKEN_URL,
            &[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", client_id),
                ("device_code", device_code),
            ],
        )
        .await?;

    if resp.is_success() {
        let tokens = parse_tokens(&resp.body)?;
        return Ok(PollOutcome::Completed(tokens));
    }

    // Non-2xx: the device-code flow uses HTTP 400 with an `error` field to
    // signal pending / slow_down / expired / denied.
    let err = resp
        .body
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    match err {
        "authorization_pending" => {
            let retry = resp
                .body
                .get("interval")
                .and_then(|v| v.as_u64())
                .or(Some(5));
            Ok(PollOutcome::Pending {
                retry_after: retry.unwrap_or(5),
            })
        }
        "slow_down" => Ok(PollOutcome::SlowDown),
        "expired_token" => Ok(PollOutcome::Expired),
        "access_denied" => Ok(PollOutcome::Denied(
            resp.body
                .get("error_description")
                .and_then(|v| v.as_str())
                .unwrap_or("access denied")
                .to_string(),
        )),
        other => Err(AuthError::Denied(format!(
            "{other}: {}",
            resp.body
                .get("error_description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
        ))),
    }
}

/// High-level helper: block until the device-code flow completes (or times out
/// / is denied). `on_pending` is invoked between polls (e.g. to update UI);
/// `timeout` bounds the whole loop. Uses the recommended `interval` plus
/// exponential backoff on `slow_down`.
pub async fn complete_device_code(
    t: &dyn AuthTransport,
    client_id: &str,
    challenge: &DeviceCodeChallenge,
    timeout: Duration,
    on_pending: impl Fn(u64),
) -> AuthResult<MicrosoftTokens> {
    let deadline = std::time::Instant::now() + timeout;
    let mut interval = Duration::from_secs(challenge.interval.max(1));
    loop {
        if std::time::Instant::now() >= deadline {
            return Err(AuthError::Expired);
        }
        match poll_token(t, client_id, &challenge.device_code).await? {
            PollOutcome::Completed(tokens) => return Ok(tokens),
            PollOutcome::Pending { retry_after } => {
                let wait = Duration::from_secs(retry_after.max(1));
                on_pending(retry_after);
                tokio::time::sleep(wait).await;
            }
            PollOutcome::SlowDown => {
                // Double the interval on slow_down (RFC 8628 guidance).
                interval = interval.saturating_mul(2);
                tokio::time::sleep(interval).await;
            }
            PollOutcome::Expired => return Err(AuthError::Expired),
            PollOutcome::Denied(reason) => return Err(AuthError::Denied(reason)),
        }
    }
}

/// Steps 3–4: Microsoft token → Xbox Live token → XSTS token (+uhs).
async fn xbox_chain(t: &dyn AuthTransport, ms_access_token: &str) -> AuthResult<(String, String)> {
    // 3) XBL
    let xbl_body = json!({
        "Properties": {
            "AuthMethod": "RPS",
            "SiteName": "user.auth.xboxlive.com",
            "RpsTicket": format!("d={ms_access_token}"),
        },
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT",
    });
    let xbl = t.post_json(XBL_AUTH_URL, &xbl_body).await?.into_value()?;
    let xbl_token = field_str(&xbl, "Token")?;
    let uhs = xbl
        .get("DisplayClaims")
        .and_then(|d| d.get("xui"))
        .and_then(|x| x.get(0))
        .and_then(|u| u.get("uhs"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| AuthError::Xbox("missing uhs in XBL response".into()))?
        .to_string();

    // 4) XSTS
    let xsts_body = json!({
        "Properties": {
            "SandboxId": "RETAIL",
            "UserTokens": [xbl_token],
        },
        "RelyingParty": "rp://api.minecraftservices.com/",
        "TokenType": "JWT",
    });
    let xsts_resp = t.post_json(XSTS_AUTH_URL, &xsts_body).await?;
    if !xsts_resp.is_success() {
        return Err(xsts_error(&xsts_resp.body));
    }
    let xsts = xsts_resp.into_value()?;
    let xsts_token = field_str(&xsts, "Token")?;
    let xsts_uhs = xsts
        .get("DisplayClaims")
        .and_then(|d| d.get("xui"))
        .and_then(|x| x.get(0))
        .and_then(|u| u.get("uhs"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| AuthError::Xbox("missing uhs in XSTS response".into()))?
        .to_string();

    if xsts_uhs != uhs {
        return Err(AuthError::Xbox("uhs mismatch between XBL and XSTS".into()));
    }
    Ok((xsts_token, xsts_uhs))
}

/// Translate an XSTS error body (`XErr` code) into a friendly error.
fn xsts_error(body: &Value) -> AuthError {
    let code = body.get("XErr").and_then(|v| v.as_u64()).or_else(|| {
        body.get("errors")
            .and_then(|e| e.get(0))
            .and_then(|e| e.get("code"))
            .and_then(|c| c.as_u64())
    });
    match code {
        Some(2148916233) => AuthError::Xbox(
            "This Microsoft account is not linked to an Xbox account (age/region).".into(),
        ),
        Some(2148916238) => {
            AuthError::Xbox("This account is a child account and needs adult approval.".into())
        }
        Some(2148929847) => {
            AuthError::Xbox("Xbox account is banned or blocked from Minecraft services.".into())
        }
        _ => AuthError::Xbox(format!(
            "Xbox Live authentication failed: {}",
            body.get("Message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown XErr")
        )),
    }
}

/// Steps 5–6: XSTS → Minecraft token → profile (uuid + name).
async fn minecraft_chain(
    t: &dyn AuthTransport,
    uhs: &str,
    xsts_token: &str,
) -> AuthResult<(String, String, String)> {
    // 5) login_with_xbox
    let identity = format!("XBL3.0 x={uhs};{xsts_token}");
    let mc_body = json!({ "identityToken": identity });
    let mc = t.post_json(MC_LOGIN_URL, &mc_body).await?.into_value()?;
    let mc_token = field_str(&mc, "access_token")?;

    // 6) profile
    let profile = t
        .get_json(MC_PROFILE_URL, Some(&mc_token))
        .await?
        .into_value()?;
    let uuid = field_str(&profile, "id")?;
    let name = field_str(&profile, "name")?;
    Ok((mc_token, uuid, name))
}

/// Build a full [`MicrosoftAccount`] from freshly obtained Microsoft tokens.
pub async fn build_microsoft_account(
    t: &dyn AuthTransport,
    client_id: &str,
    tokens: &MicrosoftTokens,
    xuid: Option<String>,
) -> AuthResult<MicrosoftAccount> {
    let (xsts_token, uhs) = xbox_chain(t, &tokens.access_token).await?;
    let (mc_token, uuid, name) = minecraft_chain(t, &uhs, &xsts_token).await?;
    let now = now_secs();
    Ok(MicrosoftAccount {
        uuid,
        username: name,
        client_id: client_id.to_string(),
        access_token: mc_token,
        refresh_token: tokens.refresh_token.clone(),
        xuid,
        expires_at: now.saturating_add(tokens.expires_in),
        ms_expires_at: now.saturating_add(tokens.expires_in),
    })
}

/// Convenience: run the full device-code flow (request handled by caller so the
/// UI can show the challenge; this completes polling + token chain).
pub async fn authenticate_device_code(
    t: &dyn AuthTransport,
    client_id: &str,
    challenge: &DeviceCodeChallenge,
    timeout: Duration,
    on_pending: impl Fn(u64),
) -> AuthResult<MicrosoftAccount> {
    let tokens = complete_device_code(t, client_id, challenge, timeout, on_pending).await?;
    build_microsoft_account(t, client_id, &tokens, None).await
}

/// Refresh a Microsoft account from its stored refresh token, re-running the
/// XBL → XSTS → Minecraft chain to obtain a fresh Minecraft access token.
pub async fn refresh_account(
    t: &dyn AuthTransport,
    account: &MicrosoftAccount,
) -> AuthResult<MicrosoftAccount> {
    if account.refresh_token.is_empty() {
        return Err(AuthError::Config("no refresh token stored".into()));
    }
    let resp = t
        .post_form(
            TOKEN_URL,
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &account.client_id),
                ("refresh_token", &account.refresh_token),
                ("scope", DEFAULT_SCOPE),
            ],
        )
        .await?;
    // A 400 here means the refresh token was revoked/expired.
    if !resp.is_success() {
        return Err(AuthError::Denied(format!(
            "refresh failed: {}",
            resp.body
                .get("error_description")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
        )));
    }
    let tokens = parse_tokens(&resp.body)?;
    build_microsoft_account(t, &account.client_id, &tokens, account.xuid.clone()).await
}

// --- helpers -------------------------------------------------------------

fn parse_tokens(body: &Value) -> AuthResult<MicrosoftTokens> {
    Ok(MicrosoftTokens {
        access_token: field_str(body, "access_token")?,
        refresh_token: field_str(body, "refresh_token")?,
        expires_in: field_u64(body, "expires_in").unwrap_or(3600),
    })
}

fn field_str(v: &Value, key: &str) -> AuthResult<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| AuthError::Other(format!("missing string field `{key}`")))
}

fn field_u64(v: &Value, key: &str) -> AuthResult<u64> {
    v.get(key)
        .and_then(|x| x.as_u64())
        .ok_or_else(|| AuthError::Other(format!("missing u64 field `{key}`")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::transport::MockTransport;
    use crate::auth::AuthError;

    fn tok() -> MicrosoftTokens {
        MicrosoftTokens {
            access_token: "ms-at".into(),
            refresh_token: "ms-rt".into(),
            expires_in: 3600,
        }
    }

    #[tokio::test]
    async fn request_device_code_parses_fields() {
        let m = MockTransport::new();
        m.script_ok(
            DEVICE_CODE_URL,
            json!({
                "user_code": "ABCD-EFGH",
                "device_code": "dc",
                "verification_uri": "https://microsoft.com/link",
                "expires_in": 900,
                "interval": 5,
                "message": "To sign in, use a web browser..."
            }),
        );
        let c = request_device_code(&m, DEFAULT_CLIENT_ID, DEFAULT_SCOPE)
            .await
            .unwrap();
        assert_eq!(c.user_code, "ABCD-EFGH");
        assert_eq!(c.verification_uri, "https://microsoft.com/link");
        assert_eq!(c.expires_in, 900);
    }

    #[tokio::test]
    async fn poll_pending_then_completed() {
        let m = MockTransport::new();
        m.script_err(TOKEN_URL, "authorization_pending", "please wait");
        m.script_ok(
            TOKEN_URL,
            json!({ "access_token":"a","refresh_token":"r","expires_in":3600 }),
        );
        let p1 = poll_token(&m, DEFAULT_CLIENT_ID, "dc").await.unwrap();
        assert!(matches!(p1, PollOutcome::Pending { .. }));
        let p2 = poll_token(&m, DEFAULT_CLIENT_ID, "dc").await.unwrap();
        match p2 {
            PollOutcome::Completed(t) => {
                assert_eq!(t.access_token, "a");
                assert_eq!(t.refresh_token, "r");
            }
            _ => panic!("expected completed"),
        }
    }

    #[tokio::test]
    async fn poll_expired_and_denied() {
        let m = MockTransport::new();
        m.script_err(TOKEN_URL, "expired_token", "expired");
        let e = poll_token(&m, DEFAULT_CLIENT_ID, "dc").await.unwrap();
        assert!(matches!(e, PollOutcome::Expired));

        let m2 = MockTransport::new();
        m2.script_err(TOKEN_URL, "access_denied", "nope");
        let d = poll_token(&m2, DEFAULT_CLIENT_ID, "dc").await.unwrap();
        assert!(matches!(d, PollOutcome::Denied(_)));
    }

    #[tokio::test]
    async fn full_chain_builds_account() {
        let m = MockTransport::new();
        // XBL
        m.script_ok(
            XBL_AUTH_URL,
            json!({ "Token":"xbl", "DisplayClaims": { "xui": [ { "uhs":"UHS" } ] } }),
        );
        // XSTS
        m.script_ok(
            XSTS_AUTH_URL,
            json!({ "Token":"xsts", "DisplayClaims": { "xui": [ { "uhs":"UHS" } ] } }),
        );
        // MC login
        m.script_ok(
            MC_LOGIN_URL,
            json!({ "access_token":"mc", "expires_in":86400 }),
        );
        // MC profile
        m.script_ok(
            MC_PROFILE_URL,
            json!({ "id":"real-uuid","name":"RealName" }),
        );
        let acc = build_microsoft_account(&m, DEFAULT_CLIENT_ID, &tok(), Some("xuid".into()))
            .await
            .unwrap();
        assert_eq!(acc.uuid, "real-uuid");
        assert_eq!(acc.username, "RealName");
        assert_eq!(acc.access_token, "mc");
        assert_eq!(acc.xuid.as_deref(), Some("xuid"));
    }

    #[tokio::test]
    async fn xsts_uhs_mismatch_is_error() {
        let m = MockTransport::new();
        m.script_ok(
            XBL_AUTH_URL,
            json!({ "Token":"xbl", "DisplayClaims": { "xui": [ { "uhs":"UHS1" } ] } }),
        );
        m.script_ok(
            XSTS_AUTH_URL,
            json!({ "Token":"xsts", "DisplayClaims": { "xui": [ { "uhs":"UHS2" } ] } }),
        );
        let r = build_microsoft_account(&m, DEFAULT_CLIENT_ID, &tok(), None).await;
        assert!(matches!(r, Err(AuthError::Xbox(_))));
    }

    #[tokio::test]
    async fn refresh_uses_refresh_token() {
        let m = MockTransport::new();
        // refresh token endpoint
        m.script_ok(
            TOKEN_URL,
            json!({ "access_token":"new-ms","refresh_token":"new-rt","expires_in":3600 }),
        );
        m.script_ok(
            XBL_AUTH_URL,
            json!({ "Token":"xbl", "DisplayClaims": { "xui": [ { "uhs":"U" } ] } }),
        );
        m.script_ok(
            XSTS_AUTH_URL,
            json!({ "Token":"xsts", "DisplayClaims": { "xui": [ { "uhs":"U" } ] } }),
        );
        m.script_ok(
            MC_LOGIN_URL,
            json!({ "access_token":"mc2","expires_in":86400 }),
        );
        m.script_ok(MC_PROFILE_URL, json!({ "id":"u","name":"N" }));

        let acc = MicrosoftAccount {
            uuid: "u".into(),
            username: "N".into(),
            client_id: DEFAULT_CLIENT_ID.into(),
            access_token: "old".into(),
            refresh_token: "rt".into(),
            xuid: None,
            expires_at: 0,
            ms_expires_at: 0,
        };
        let refreshed = refresh_account(&m, &acc).await.unwrap();
        assert_eq!(refreshed.access_token, "mc2");
        assert_eq!(refreshed.refresh_token, "new-rt");
    }
}
