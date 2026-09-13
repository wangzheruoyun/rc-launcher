//! Microsoft OAuth 2.0 Device Code flow + the Xbox Live / Minecraft token
//! chain, plus refresh (task 5).
//!
//! Sequence (mirrors the official Minecraft Java launcher + FCL's
//! `MicrosoftService`):
//!
//! 1. `request_device_code`  → ask Microsoft for a `user_code` / `device_code`.
//!    Optionally carries a `redirect_uri` (task 28) so the embedded
//!    `microsoft_auth.html` callback page can intercept the browser redirect.
//! 2. `poll_token`           → poll until the user finishes consent; yields a
//!    Microsoft access + refresh token. The HTTP call is wrapped in retry with
//!    exponential backoff (task 28) so transient China-mainland network blips
//!    don't abort the login.
//! 3. `xbl_authenticate`     → exchange the MS token for an Xbox Live token.
//!    Wrapped in retry (task 28) with a labelled error for the common case of
//!    XBL being unreachable behind the Great Firewall.
//! 4. `xsts_authorize`       → exchange the XBL token for an XSTS token (+uhs).
//!    Wrapped in retry (task 28).
//! 5. `login_with_xbox`      → exchange XSTS for a Minecraft access token.
//!    Wrapped in retry (task 28) with a labelled error for the common case of
//!    Mojang services being unreachable.
//! 6. `fetch_profile`        → fetch the Minecraft Java profile (uuid + name).
//!
//! `authenticate_device_code` orchestrates 2–6; `refresh_account` re-runs
//! 2-style refresh + 3–6 to mint a fresh Minecraft token from a stored
//! refresh token. Both `poll_token` and `refresh_account` retry the token
//! endpoint HTTP call with `robust::retry` so a single DNS hiccup or
//! packet loss during polling doesn't fail the whole login (task 28).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

use crate::auth::model::{now_secs, MicrosoftAccount};
use crate::auth::transport::AuthTransport;
use crate::auth::AuthError;
use crate::auth::AuthResult;
use crate::robust::retry::{retry, RetryPolicy};
use base64::Engine;

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
/// Mojang session-server profile endpoint that returns the `textures` property
/// (skin + cape URLs). The access token is passed as a bearer token.
pub const MC_SKIN_PROFILE_URL: &str =
    "https://sessionserver.mojang.com/session/profile/{uuid}?unsigned=false";
/// Mojang endpoint for uploading a custom skin (PUT, multipart/form).
pub const MC_SKIN_UPLOAD_URL: &str = "https://api.minecraftservices.com/minecraft/profile/skin";

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
    /// Optional custom callback/redirect address for the browser redirect
    /// (task 28). When set, Microsoft redirects the user to this address after
    /// they complete sign-in, allowing the embedded `microsoft_auth.html`
    /// callback page to intercept the result. `None` uses Microsoft's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect_uri: Option<String>,
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
///
/// `redirect_uri` (task 28) optionally supplies a custom callback address so
/// the embedded `microsoft_auth.html` page can intercept the browser redirect
/// after the user completes sign-in. Microsoft accepts it on the device-code
/// endpoint; when omitted Microsoft falls back to its default behaviour.
pub async fn request_device_code(
    t: &dyn AuthTransport,
    client_id: &str,
    scope: &str,
    redirect_uri: Option<&str>,
) -> AuthResult<DeviceCodeChallenge> {
    if client_id.is_empty() {
        return Err(AuthError::Config("client_id is empty".into()));
    }
    let mut fields: Vec<(&str, &str)> = vec![("client_id", client_id), ("scope", scope)];
    if let Some(uri) = redirect_uri {
        fields.push(("redirect_uri", uri));
    }
    // Retry the device-code request with exponential backoff (task 28).
    // A transient network error (DNS hiccup, packet loss, Great-Firewall reset)
    // when requesting the device code should not immediately abort login; only
    // genuine transport-level failures are replayed (RcError::Transient).
    let resp = retry(&RetryPolicy::default(), || {
        let t = t;
        let fields = &fields;
        async move { t.post_form(DEVICE_CODE_URL, fields).await }
    })
    .await
    .map_err(|e| AuthError::Network(format!("Microsoft device-code request failed: {e}")))?;
    let body = resp.into_value()?;
    let challenge = DeviceCodeChallenge {
        user_code: field_str(&body, "user_code")?,
        device_code: field_str(&body, "device_code")?,
        verification_uri: field_str(&body, "verification_uri")?,
        expires_in: field_u64(&body, "expires_in")?,
        interval: field_u64(&body, "interval").unwrap_or(5),
        message: field_str(&body, "message").unwrap_or_default(),
        redirect_uri: redirect_uri.map(|s| s.to_string()),
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
    // Retry the token-endpoint HTTP call with exponential backoff (task 28).
    // Transient network errors (DNS hiccup, packet loss, Great-Firewall reset)
    // during device-code polling must not abort the whole login flow.
    // `authorization_pending` / `expired_token` / `access_denied` are returned
    // as OK(PollOutcome::*) by `poll_token` below, so they are *not* retried
    // here — only genuine network-level failures (RcError::Transient) are.
    let resp = retry(&RetryPolicy::default(), || {
        let t = t;
        async move {
            t.post_form(
                TOKEN_URL,
                &[
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("client_id", client_id),
                    ("device_code", device_code),
                ],
            )
            .await
        }
    })
    .await
    .map_err(|e| AuthError::Network(format!("Microsoft token polling failed: {e}")))?;

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
    // Retry the XBL HTTP call with exponential backoff (task 28). Transient
    // network errors (DNS hiccup, packet loss, Great-Firewall reset) during the
    // XBL/XSTS exchange must not abort the whole login flow. Only genuine
    // transport-level failures are retried; HTTP 4xx/5xx from XBL itself are
    // returned immediately as application errors.
    let xbl = retry(&RetryPolicy::default(), || {
        let t = t;
        let body = &xbl_body;
        async move { t.post_json(XBL_AUTH_URL, body).await }
    })
    .await
    .map_err(|e| {
        AuthError::Network(format!(
            "{}: {}",
            crate::i18n::t("auth.login.xblBlocked"),
            e
        ))
    })?
    .into_value()?;
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
    // XSTS authorization: retry on transient network failures (task 28).
    let xsts_resp = retry(&RetryPolicy::default(), || {
        let t = t;
        let body = &xsts_body;
        async move { t.post_json(XSTS_AUTH_URL, body).await }
    })
    .await
    .map_err(|e| {
        AuthError::Network(format!(
            "{}: {}",
            crate::i18n::t("auth.login.xblBlocked"),
            e
        ))
    })?;
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
    // Retry the login_with_xbox HTTP call with exponential backoff (task 28).
    // Minecraft services (api.minecraftservices.com) are often throttled or
    // blocked in mainland China; retrying with backoff absorbs transient
    // failures so a single DNS hiccup doesn't fail the login.
    let mc = retry(&RetryPolicy::default(), || {
        let t = t;
        let body = &mc_body;
        async move { t.post_json(MC_LOGIN_URL, body).await }
    })
    .await
    .map_err(|e| {
        AuthError::Network(format!(
            "{}: {}",
            crate::i18n::t("auth.login.minecraftBlocked"),
            e
        ))
    })?
    .into_value()?;
    let mc_token = field_str(&mc, "access_token")?;

    // 6) profile
    // Retry the profile fetch with backoff (task 28). sessionserver.mojang.com
    // is frequently throttled in mainland China.
    let profile = retry(&RetryPolicy::default(), || {
        let t = t;
        let token = &mc_token;
        async move { t.get_json(MC_PROFILE_URL, Some(token)).await }
    })
    .await
    .map_err(|e| {
        AuthError::Network(format!(
            "{}: {}",
            crate::i18n::t("auth.login.minecraftBlocked"),
            e
        ))
    })?
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
    // Retry the token-exchange HTTP call with backoff (task 28).
    // A refresh happens on app startup / before launch, so a single network
    // blip should not doom the entire session.
    let resp = retry(&RetryPolicy::default(), || {
        let t = t;
        let account = account;
        async move {
            t.post_form(
                TOKEN_URL,
                &[
                    ("grant_type", "refresh_token"),
                    ("client_id", &account.client_id),
                    ("refresh_token", &account.refresh_token),
                    ("scope", DEFAULT_SCOPE),
                ],
            )
            .await
        }
    })
    .await
    .map_err(|e| AuthError::Network(format!("Microsoft token refresh failed: {e}")))?;
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

/// Fetch skin/cape metadata for a player from Mojang's session profile API
/// (task 22). Returns a [`crate::auth::model::SkinModel`] with the skin and
/// cape download URLs, the model type ("slim" / "default") and a timestamp.
/// The actual PNG bytes are *not* downloaded here — the UI fetches them from
/// `skin_url` / `cape_url` and caches them locally.
pub async fn fetch_skin_data(
    t: &dyn AuthTransport,
    uuid: &str,
    access_token: &str,
) -> AuthResult<crate::auth::model::SkinModel> {
    use crate::auth::model::{SkinModel, SkinSource};

    let url = MC_SKIN_PROFILE_URL.replace("{uuid}", &uuid.replace('-', ""));
    let resp = t.get_json(&url, Some(access_token)).await?;
    let body = resp.into_value()?;

    // The response has a `properties` array with a `textures` entry whose
    // `value` is base64-encoded JSON.
    let properties = body
        .get("properties")
        .and_then(|v| v.as_array())
        .ok_or_else(|| AuthError::Other("missing properties in profile".into()))?;

    let mut textures_json = None;
    for prop in properties {
        if prop.get("name").and_then(|v| v.as_str()) == Some("textures") {
            textures_json = prop
                .get("value")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            break;
        }
    }
    let encoded =
        textures_json.ok_or_else(|| AuthError::Other("textures property not found".into()))?;

    // Decode base64 + parse the inner JSON.
    let decoded_bytes = base64::engine::general_purpose::STANDARD
        .decode(&encoded)
        .map_err(|e| AuthError::Other(format!("base64 decode textures: {e}")))?;
    let decoded: Value = serde_json::from_slice(&decoded_bytes)
        .map_err(|e| AuthError::Other(format!("parse textures json: {e}")))?;

    let textures = decoded
        .get("textures")
        .and_then(|t| t.get("skin"))
        .and_then(|s| s.get("url"))
        .and_then(|u| u.as_str())
        .ok_or_else(|| AuthError::Other("skin url not found in textures".into()))?
        .to_string();

    let model = decoded
        .get("textures")
        .and_then(|t| t.get("skin"))
        .and_then(|s| s.get("metadata"))
        .and_then(|m| m.get("model"))
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();

    let cape_url = decoded
        .get("textures")
        .and_then(|t| t.get("cape"))
        .and_then(|c| c.get("url"))
        .and_then(|u| u.as_str())
        .map(|s| s.to_string());

    Ok(SkinModel {
        uuid: uuid.to_string(),
        skin_url: textures,
        cape_url,
        fetched_at: now_secs(),
        cached_at: 0,
        source: SkinSource::Official,
        hash: None,
        model,
    })
}

/// Upload a custom skin for a Microsoft account (task 22). The caller provides
/// the skin PNG bytes; `model` is "slim" or "classic" (as expected by Mojang's
/// PUT endpoint — note this differs from the profile payload's "default"/"slim").
pub async fn upload_skin(
    t: &dyn AuthTransport,
    access_token: &str,
    model: &str,
    skin_data: &[u8],
) -> AuthResult<()> {
    let resp = t
        .post_multipart(
            MC_SKIN_UPLOAD_URL,
            &[("model", model)],
            "skin",
            "skin.png",
            "image/png",
            skin_data,
            Some(access_token),
        )
        .await?;
    if !resp.is_success() {
        let msg = resp
            .body
            .get("errorMessage")
            .and_then(|v| v.as_str())
            .unwrap_or("skin upload failed");
        return Err(AuthError::Other(format!(
            "skin upload failed: HTTP {}: {}",
            resp.status, msg
        )));
    }
    Ok(())
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
        let c = request_device_code(&m, DEFAULT_CLIENT_ID, DEFAULT_SCOPE, None)
            .await
            .unwrap();
        assert_eq!(c.user_code, "ABCD-EFGH");
        assert_eq!(c.verification_uri, "https://microsoft.com/link");
        assert_eq!(c.expires_in, 900);
        assert_eq!(c.redirect_uri, None);
    }

    #[tokio::test]
    async fn request_device_code_passes_redirect_uri() {
        let m = MockTransport::new();
        m.script_ok(
            DEVICE_CODE_URL,
            json!({
                "user_code": "WXYZ",
                "device_code": "dc2",
                "verification_uri": "https://microsoft.com/link",
                "expires_in": 900,
                "interval": 5,
                "message": "go"
            }),
        );
        let c = request_device_code(
            &m,
            DEFAULT_CLIENT_ID,
            DEFAULT_SCOPE,
            Some("msauth://com.rc.launcher/callback"),
        )
        .await
        .unwrap();
        assert_eq!(c.user_code, "WXYZ");
        assert_eq!(
            c.redirect_uri.as_deref(),
            Some("msauth://com.rc.launcher/callback")
        );
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

    // --- Task 28: retry + block-hint tests for the token-exchange chain ---

    use crate::auth::transport::AuthResponse;
    use crate::error::{RcError, RcResult};
    use async_trait::async_trait;
    use std::sync::atomic::AtomicU32;

    /// Transport that always returns a transient `RcError::Connection`,
    /// simulating a network that is firewalled / throttled.
    struct AlwaysFailTransport;

    #[async_trait]
    impl AuthTransport for AlwaysFailTransport {
        async fn post_form(&self, url: &str, _form: &[(&str, &str)]) -> RcResult<AuthResponse> {
            Err(RcError::Connection(format!("connection refused to {url}")))
        }
        async fn post_json(&self, url: &str, _body: &Value) -> RcResult<AuthResponse> {
            Err(RcError::Connection(format!("connection refused to {url}")))
        }
        async fn get_json(&self, url: &str, _bearer: Option<&str>) -> RcResult<AuthResponse> {
            Err(RcError::Connection(format!("connection refused to {url}")))
        }
        async fn post_multipart(
            &self,
            url: &str,
            _text_fields: &[(&str, &str)],
            _file_field: &str,
            _file_name: &str,
            _file_content_type: &str,
            _file_data: &[u8],
            _bearer: Option<&str>,
        ) -> RcResult<AuthResponse> {
            Err(RcError::Connection(format!("connection refused to {url}")))
        }
    }

    /// Transport that fails the first `fail_n` calls to `fail_url` with a
    /// transient error, then delegates to `inner`.
    struct FlakyTransport {
        inner: MockTransport,
        fail_url: String,
        failures_left: AtomicU32,
    }

    impl FlakyTransport {
        fn new(inner: MockTransport, fail_url: String, fail_n: u32) -> Self {
            Self {
                inner,
                fail_url,
                failures_left: AtomicU32::new(fail_n),
            }
        }
    }

    #[async_trait]
    impl AuthTransport for FlakyTransport {
        async fn post_json(&self, url: &str, body: &Value) -> RcResult<AuthResponse> {
            if url == self.fail_url
                && self.failures_left.load(std::sync::atomic::Ordering::SeqCst) > 0
            {
                self.failures_left
                    .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                return Err(RcError::Connection(format!("connection refused to {url}")));
            }
            self.inner.post_json(url, body).await
        }
        async fn post_form(&self, url: &str, form: &[(&str, &str)]) -> RcResult<AuthResponse> {
            if url == self.fail_url
                && self.failures_left.load(std::sync::atomic::Ordering::SeqCst) > 0
            {
                self.failures_left
                    .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                return Err(RcError::Connection(format!("connection refused to {url}")));
            }
            self.inner.post_form(url, form).await
        }
        async fn get_json(&self, url: &str, bearer: Option<&str>) -> RcResult<AuthResponse> {
            self.inner.get_json(url, bearer).await
        }
        async fn post_multipart(
            &self,
            url: &str,
            text_fields: &[(&str, &str)],
            file_field: &str,
            file_name: &str,
            file_content_type: &str,
            file_data: &[u8],
            bearer: Option<&str>,
        ) -> RcResult<AuthResponse> {
            self.inner
                .post_multipart(
                    url,
                    text_fields,
                    file_field,
                    file_name,
                    file_content_type,
                    file_data,
                    bearer,
                )
                .await
        }
    }

    #[tokio::test]
    async fn request_device_code_retries_on_transient_error() {
        // first call to DEVICE_CODE_URL fails (transient), retry succeeds.
        let m = MockTransport::new();
        m.script_ok(
            DEVICE_CODE_URL,
            json!({
                "user_code": "TEST",
                "device_code": "dc-test",
                "verification_uri": "https://microsoft.com/devicelogin",
                "expires_in": 900,
                "interval": 5,
                "message": "go"
            }),
        );
        let flaky = FlakyTransport::new(m, DEVICE_CODE_URL.to_string(), 1);
        let challenge = request_device_code(&flaky, DEFAULT_CLIENT_ID, DEFAULT_SCOPE, None).await;
        assert!(
            challenge.is_ok(),
            "request_device_code should succeed after retry: {challenge:?}"
        );
        let c = challenge.unwrap();
        assert_eq!(c.user_code, "TEST");
        assert_eq!(c.redirect_uri, None);
    }

    #[tokio::test]
    async fn request_device_code_error_includes_block_hint_on_exhaustion() {
        let _g = crate::i18n::GLOBAL_I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::i18n::set_language(crate::i18n::Language::En);
        let flaky = AlwaysFailTransport;
        let result = request_device_code(&flaky, DEFAULT_CLIENT_ID, DEFAULT_SCOPE, None).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("network"),
            "error should be network-level: {err:?}"
        );
        crate::i18n::set_language(crate::i18n::current_language());
    }

    #[tokio::test]
    async fn xbox_chain_retries_on_transient_error() {
        // XBL call fails once (transient), then succeeds on retry.
        let m = MockTransport::new();
        m.script_ok(
            XBL_AUTH_URL,
            json!({ "Token":"xbl", "DisplayClaims": { "xui": [ { "uhs":"UHS" } ] } }),
        );
        m.script_ok(
            XSTS_AUTH_URL,
            json!({ "Token":"xsts", "DisplayClaims": { "xui": [ { "uhs":"UHS" } ] } }),
        );
        let flaky = FlakyTransport::new(m, XBL_AUTH_URL.to_string(), 1);
        // RetryPolicy::default() retries on RcError::Connection (Transient).
        // The first XBL call fails, the retry succeeds.
        let result = xbox_chain(&flaky, "ms-token").await;
        assert!(
            result.is_ok(),
            "xbox_chain should succeed after retry: {result:?}"
        );
        let (token, uhs) = result.unwrap();
        assert_eq!(token, "xsts");
        assert_eq!(uhs, "UHS");
    }

    #[tokio::test]
    async fn xbox_chain_error_includes_block_hint_on_exhaustion() {
        let _g = crate::i18n::GLOBAL_I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::i18n::set_language(crate::i18n::Language::ZhCn);

        let flaky = AlwaysFailTransport;
        let result = xbox_chain(&flaky, "ms-token").await;
        assert!(result.is_err());
        match result {
            Err(AuthError::Network(msg)) => {
                // The error message must include the i18n block hint.
                assert!(
                    msg.contains("Xbox Live"),
                    "error message should mention Xbox Live: {msg}"
                );
            }
            other => panic!("expected AuthError::Network, got {other:?}"),
        }

        let restore = crate::i18n::current_language();
        crate::i18n::set_language(crate::i18n::Language::En);
        let result_en = xbox_chain(&flaky, "ms-token").await;
        assert!(result_en.is_err());
        if let Err(AuthError::Network(msg)) = result_en {
            assert!(
                msg.contains("Xbox Live"),
                "English error should mention Xbox Live: {msg}"
            );
        }
        crate::i18n::set_language(restore);
    }

    #[tokio::test]
    async fn minecraft_chain_error_includes_block_hint_on_exhaustion() {
        let _g = crate::i18n::GLOBAL_I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::i18n::set_language(crate::i18n::Language::En);
        let flaky = AlwaysFailTransport;
        let result = minecraft_chain(&flaky, "UHS", "xsts-token").await;
        assert!(result.is_err());
        match result {
            Err(AuthError::Network(msg)) => {
                assert!(
                    msg.contains("Minecraft"),
                    "error message should mention Minecraft: {msg}"
                );
            }
            other => panic!("expected AuthError::Network, got {other:?}"),
        }
        crate::i18n::set_language(crate::i18n::current_language());
    }

    #[tokio::test]
    async fn build_account_surfaces_xbl_block_hint_on_failure() {
        let _g = crate::i18n::GLOBAL_I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        crate::i18n::set_language(crate::i18n::Language::En);
        let flaky = AlwaysFailTransport;
        let result = build_microsoft_account(&flaky, DEFAULT_CLIENT_ID, &tok(), None).await;
        assert!(result.is_err());
        let rc_err: RcError = result.unwrap_err().into();
        assert!(
            rc_err.cn_login_fallback_hint().is_some(),
            "network-level auth error should carry a cn_fallback hint"
        );
        crate::i18n::set_language(crate::i18n::current_language());
    }
}
