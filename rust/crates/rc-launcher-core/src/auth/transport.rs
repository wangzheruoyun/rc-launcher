//! Pluggable HTTP transport for the auth flows.
//!
//! The authentication logic (device-code polling, XBL/XSTS, refresh) is written
//! against the [`AuthTransport`] trait so it can be unit-tested with a fully
//! scripted [`MockTransport`] (no network), exactly like the `download`
//! module's `HttpSource` mock.
//!
//! Responses carry their HTTP status so callers can distinguish a fatal error
//! from a *normal* non-2xx such as the device-code poll's `authorization_pending`
//! (HTTP 400 with an `error` field).

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::error::{RcError, RcResult};
use crate::net::ProxyConfig;

/// A single HTTP response returned by an [`AuthTransport`].
#[derive(Debug, Clone)]
pub struct AuthResponse {
    /// HTTP status code.
    pub status: u16,
    /// Parsed JSON body (best-effort; may be `Value::Null` if the body was not
    /// valid JSON).
    pub body: Value,
}

impl AuthResponse {
    /// True for 2xx responses.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Consume the response, returning the JSON body on success or an
    /// [`RcError::Auth`] carrying the status + body otherwise.
    pub fn into_value(self) -> RcResult<Value> {
        if self.is_success() {
            Ok(self.body)
        } else {
            let msg = self
                .body
                .as_object()
                .and_then(|o| o.get("error"))
                .and_then(|v| v.as_str())
                .map(|e| e.to_string())
                .unwrap_or_else(|| self.body.to_string());
            Err(RcError::Auth(format!("HTTP {}: {}", self.status, msg)))
        }
    }
}

/// A minimal JSON HTTP client used by the auth flows.
#[async_trait]
pub trait AuthTransport: Send + Sync {
    /// POST `application/x-www-form-urlencoded` and return the response
    /// (including non-2xx statuses).
    async fn post_form(&self, url: &str, form: &[(&str, &str)]) -> RcResult<AuthResponse>;
    /// POST a JSON body and return the response.
    async fn post_json(&self, url: &str, body: &Value) -> RcResult<AuthResponse>;
    /// GET with an optional bearer token and return the response.
    async fn get_json(&self, url: &str, bearer: Option<&str>) -> RcResult<AuthResponse>;
    /// POST a multipart/form request with a single file upload and return the
    /// response (including non-2xx statuses). Used by skin/cape upload (task 22).
    async fn post_multipart(
        &self,
        url: &str,
        text_fields: &[(&str, &str)],
        file_field: &str,
        file_name: &str,
        file_content_type: &str,
        file_data: &[u8],
        bearer: Option<&str>,
    ) -> RcResult<AuthResponse>;
}

/// Production transport backed by a `reqwest::Client`. Prefer constructing it
/// from the China-mainland-optimised [`crate::net::NetworkClient`] so the auth
/// endpoints inherit DNS optimisation / proxy support (see task 3).
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    /// Build a transport from an existing `reqwest::Client`.
    pub fn from_client(client: &reqwest::Client) -> Self {
        Self {
            client: client.clone(),
        }
    }

    /// Build a standalone transport with launcher defaults (rustls TLS, UA,
    /// sensible timeouts). Use [`ReqwestTransport::from_client`] when you
    /// already have a [`crate::net::NetworkClient`].
    pub fn with_defaults() -> RcResult<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("RC-Launcher/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| RcError::Auth(format!("failed to build http client: {e}")))?;
        Ok(Self { client })
    }

    /// Build a transport that honours a proxy configuration (task 28).
    ///
    /// This is the entry point for the China-mainland proxy / mirror fallback:
    /// when the user has configured an HTTP / HTTPS / SOCKS5 proxy in Settings,
    /// the auth transport (token exchange with Microsoft / Xbox / Mojang) uses
    /// it so the XBL/XSTS/Minecraft calls can punch through the Great Firewall.
    /// When `proxy` is [`ProxyConfig::None`] this is equivalent to
    /// [`Self::with_defaults`].
    pub fn with_proxy(proxy: &ProxyConfig) -> RcResult<Self> {
        let prox = proxy.to_reqwest()?;
        let mut builder = reqwest::Client::builder()
            .user_agent(concat!("RC-Launcher/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60));
        if let Some(p) = prox {
            builder = builder.proxy(p);
        }
        let client = builder
            .build()
            .map_err(|e| RcError::Auth(format!("failed to build http client: {e}")))?;
        Ok(Self { client })
    }

    async fn send_form(&self, url: &str, form: &[(&str, &str)]) -> RcResult<AuthResponse> {
        let resp = self
            .client
            .post(url)
            .form(form)
            .send()
            .await
            .map_err(|e| RcError::Auth(format!("POST {url}: {e}")))?;
        parse(resp, url).await
    }

    async fn send_json(&self, url: &str, body: &Value) -> RcResult<AuthResponse> {
        let resp = self
            .client
            .post(url)
            .json(body)
            .send()
            .await
            .map_err(|e| RcError::Auth(format!("POST {url}: {e}")))?;
        parse(resp, url).await
    }

    async fn send_get(&self, url: &str, bearer: Option<&str>) -> RcResult<AuthResponse> {
        let mut req = self.client.get(url);
        if let Some(tok) = bearer {
            req = req.bearer_auth(tok);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| RcError::Auth(format!("GET {url}: {e}")))?;
        parse(resp, url).await
    }

    async fn send_multipart(
        &self,
        url: &str,
        text_fields: &[(&str, &str)],
        file_field: &str,
        file_name: &str,
        file_content_type: &str,
        file_data: &[u8],
        bearer: Option<&str>,
    ) -> RcResult<AuthResponse> {
        let mut form = reqwest::multipart::Form::new();
        for (k, v) in text_fields {
            form = form.text(k.to_string(), v.to_string());
        }
        let part = reqwest::multipart::Part::bytes(file_data.to_vec())
            .file_name(file_name.to_string())
            .mime_str(file_content_type)
            .map_err(|e| RcError::Auth(format!("multipart part: {e}")))?;
        form = form.part(file_field.to_string(), part);
        let mut req = self.client.put(url).multipart(form);
        if let Some(tok) = bearer {
            req = req.bearer_auth(tok);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| RcError::Auth(format!("PUT {url}: {e}")))?;
        parse(resp, url).await
    }
}

async fn parse(resp: reqwest::Response, url: &str) -> RcResult<AuthResponse> {
    let status = resp.status().as_u16();
    let text = resp
        .text()
        .await
        .map_err(|e| RcError::Auth(format!("read body {url}: {e}")))?;
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    Ok(AuthResponse { status, body })
}

#[async_trait]
impl AuthTransport for ReqwestTransport {
    async fn post_form(&self, url: &str, form: &[(&str, &str)]) -> RcResult<AuthResponse> {
        self.send_form(url, form).await
    }
    async fn post_json(&self, url: &str, body: &Value) -> RcResult<AuthResponse> {
        self.send_json(url, body).await
    }
    async fn get_json(&self, url: &str, bearer: Option<&str>) -> RcResult<AuthResponse> {
        self.send_get(url, bearer).await
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
        self.send_multipart(
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

/// A scripted transport for unit tests. Each URL has a queue of canned
/// [`AuthResponse`]s consumed in order.
pub struct MockTransport {
    scripts: Mutex<HashMap<String, VecDeque<AuthResponse>>>,
}

impl MockTransport {
    pub fn new() -> Self {
        Self {
            scripts: Mutex::new(HashMap::new()),
        }
    }

    /// Queue a response for `url`.
    pub fn script(&self, url: &str, status: u16, body: Value) -> &Self {
        self.scripts
            .lock()
            .unwrap()
            .entry(url.to_string())
            .or_default()
            .push_back(AuthResponse { status, body });
        self
    }

    /// Queue a successful (200) JSON response for `url`.
    pub fn script_ok(&self, url: &str, body: Value) -> &Self {
        self.script(url, 200, body)
    }

    /// Queue an error (400) JSON response for `url`.
    pub fn script_err(&self, url: &str, error: &str, description: &str) -> &Self {
        let mut m = serde_json::Map::new();
        m.insert("error".into(), Value::String(error.into()));
        m.insert(
            "error_description".into(),
            Value::String(description.into()),
        );
        self.script(url, 400, Value::Object(m))
    }
}

impl Default for MockTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AuthTransport for MockTransport {
    async fn post_form(&self, url: &str, _form: &[(&str, &str)]) -> RcResult<AuthResponse> {
        self.dispatch(url)
    }
    async fn post_json(&self, url: &str, _body: &Value) -> RcResult<AuthResponse> {
        self.dispatch(url)
    }
    async fn get_json(&self, url: &str, _bearer: Option<&str>) -> RcResult<AuthResponse> {
        self.dispatch(url)
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
        self.dispatch(url)
    }
}

impl MockTransport {
    fn dispatch(&self, url: &str) -> RcResult<AuthResponse> {
        let mut map = self.scripts.lock().unwrap();
        let q = map
            .get_mut(url)
            .ok_or_else(|| RcError::Auth(format!("mock: no script for {url}")))?;
        q.pop_front()
            .ok_or_else(|| RcError::Auth(format!("mock: ran out of scripts for {url}")))
    }
}
