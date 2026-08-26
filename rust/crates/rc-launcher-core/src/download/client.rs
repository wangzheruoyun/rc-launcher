//! HTTP source abstraction over `reqwest` (task 2).
//!
//! The downloader talks to an [`HttpSource`] trait so it can be exercised with
//! an in-memory mock in unit tests (no network). The production implementation
//! ([`ReqwestSource`]) honours HTTP `Range` requests, which is what enables
//! resume + parallel chunking.

use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, RANGE, RETRY_AFTER};
use reqwest::StatusCode;
use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::error::{RcError, RcResult};

/// Result of fetching a (sub)range of a remote resource.
#[derive(Debug, Clone)]
pub struct FetchResult {
    /// Body bytes for the requested range.
    pub bytes: Vec<u8>,
    /// Total size of the *whole* resource (from `Content-Range` / `Content-Length`).
    pub total_size: u64,
    /// Whether the server actually honoured the `Range` header
    /// (`206 Partial Content`). When `false` the body is the full resource and
    /// the downloader must fall back to a single sequential fetch.
    pub supports_range: bool,
}

/// A range-capable HTTP backend.
#[async_trait]
pub trait HttpSource: Send + Sync {
    /// Fetch bytes from `start` (inclusive) to `end` (inclusive). When `end` is
    /// `None` the remainder of the resource is fetched.
    async fn fetch_range(&self, url: &str, start: u64, end: Option<u64>) -> RcResult<FetchResult>;

    /// Stream the bytes for `start..=end` directly into `writer`, returning the
    /// number of bytes written. This is the memory-efficient path used by the
    /// downloader: it never materialises the whole range as a `Vec<u8>`, so a
    /// multi-MiB chunk does not pin that much RAM at once (task 25 — large-file
    /// streaming download).
    ///
    /// The default implementation buffers the range via [`HttpSource::fetch_range`]
    /// and writes it in one `write_all`; production backends (`ReqwestSource`,
    /// `NetworkClient`) override this to stream the response body incrementally,
    /// writing each `Bytes` chunk to disk as it arrives.
    async fn fetch_range_into(
        &self,
        url: &str,
        start: u64,
        end: Option<u64>,
        writer: &mut (dyn AsyncWrite + Send + Unpin),
    ) -> RcResult<u64> {
        let r = self.fetch_range(url, start, end).await?;
        writer.write_all(&r.bytes).await.map_err(RcError::Io)?;
        Ok(r.bytes.len() as u64)
    }
}

/// Production [`HttpSource`] backed by a `reqwest` client.
pub struct ReqwestSource {
    client: reqwest::Client,
}

impl ReqwestSource {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    /// Build a client with sensible launcher defaults (rustls TLS, UA).
    pub fn with_defaults() -> RcResult<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("RC-Launcher/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| RcError::Download(format!("failed to build http client: {e}")))?;
        Ok(Self { client })
    }

    /// Build a client with explicit connect/read timeouts.
    pub fn with_timeouts(connect: Duration, read: Duration) -> RcResult<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("RC-Launcher/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(connect)
            .timeout(read)
            .build()
            .map_err(|e| RcError::Download(format!("failed to build http client: {e}")))?;
        Ok(Self { client })
    }
}

#[async_trait]
impl HttpSource for ReqwestSource {
    async fn fetch_range(&self, url: &str, start: u64, end: Option<u64>) -> RcResult<FetchResult> {
        let range_value = match end {
            Some(e) => format!("bytes={start}-{e}"),
            None => format!("bytes={start}-"),
        };

        let resp = self
            .client
            .get(url)
            .header(RANGE, range_value)
            .send()
            .await
            .map_err(|e| RcError::Network(format!("request failed for {url}: {e}")))?;

        let status = resp.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            // Surface a rate-limit together with the server's own `Retry-After`
            // hint so the downloader (and the unified retry/backoff layer, task
            // 19) can honour it instead of blindly hammering the endpoint.
            let retry_after = resp
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_retry_after);
            return Err(RcError::RateLimited { retry_after });
        }
        if status != StatusCode::PARTIAL_CONTENT && status != StatusCode::OK {
            return Err(RcError::Download(format!(
                "unexpected HTTP status {status} for {url}"
            )));
        }

        let supports_range = status == StatusCode::PARTIAL_CONTENT;

        let total_size = resp
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_content_range_total)
            .or_else(|| {
                resp.headers()
                    .get(CONTENT_LENGTH)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
            })
            .unwrap_or(0);

        let bytes = resp
            .bytes()
            .await
            .map_err(|e| RcError::Network(format!("failed to read body for {url}: {e}")))?
            .to_vec();

        if let Some(e) = end {
            let expected = (e - start + 1) as usize;
            if supports_range && bytes.len() != expected {
                return Err(RcError::Download(format!(
                    "range length mismatch for {url}: expected {expected} bytes, got {}",
                    bytes.len()
                )));
            }
        }

        Ok(FetchResult {
            bytes,
            total_size,
            supports_range,
        })
    }

    async fn fetch_range_into(
        &self,
        url: &str,
        start: u64,
        end: Option<u64>,
        writer: &mut (dyn AsyncWrite + Send + Unpin),
    ) -> RcResult<u64> {
        let range_value = match end {
            Some(e) => format!("bytes={start}-{e}"),
            None => format!("bytes={start}-"),
        };
        let resp = self
            .client
            .get(url)
            .header(RANGE, range_value)
            .send()
            .await
            .map_err(|e| RcError::Network(format!("request failed for {url}: {e}")))?;
        let status = resp.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            // Surface a rate-limit together with the server's own `Retry-After`
            // hint so the downloader (and the unified retry/backoff layer, task
            // 19) can honour it instead of blindly hammering the endpoint.
            let retry_after = resp
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_retry_after);
            return Err(RcError::RateLimited { retry_after });
        }
        if status != StatusCode::PARTIAL_CONTENT && status != StatusCode::OK {
            return Err(RcError::Download(format!(
                "unexpected HTTP status {status} for {url}"
            )));
        }
        // Stream the body incrementally: each `Bytes` chunk is written to the
        // destination as it arrives, so peak RAM stays at one network buffer
        // regardless of the chunk size.
        let mut stream = resp.bytes_stream();
        let mut total: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk
                .map_err(|e| RcError::Network(format!("failed to read body for {url}: {e}")))?;
            if chunk.is_empty() {
                continue;
            }
            writer.write_all(&chunk).await.map_err(RcError::Io)?;
            total += chunk.len() as u64;
        }
        Ok(total)
    }
}

/// Parse the total size out of a `Content-Range` header, e.g.
/// `bytes 0-1023/2048` -> `Some(2048)`.
fn parse_content_range_total(value: &str) -> Option<u64> {
    let slash = value.rfind('/')?;
    value[slash + 1..].trim().parse::<u64>().ok()
}

/// Parse an HTTP `Retry-After` value into a `Duration`.
///
/// Supports the delta-seconds form (`Retry-After: 120`), which is what Mojang
/// and the mirror CDNs emit. Returns `None` for unparseable values so the
/// caller keeps its default backoff policy.
fn parse_retry_after(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_content_range_total() {
        assert_eq!(parse_content_range_total("bytes 0-1023/2048"), Some(2048));
        assert_eq!(parse_content_range_total("bytes 100-199/1000"), Some(1000));
        assert_eq!(parse_content_range_total("none"), None);
    }

    #[test]
    fn build_client_defaults() {
        let c = ReqwestSource::with_defaults();
        assert!(c.is_ok());
    }

    #[test]
    fn parses_retry_after_delta_seconds() {
        assert_eq!(parse_retry_after("120"), Some(Duration::from_secs(120)));
        assert_eq!(parse_retry_after(" 30 "), Some(Duration::from_secs(30)));
        assert_eq!(parse_retry_after("not-a-number"), None);
        assert_eq!(parse_retry_after(""), None);
    }
}
