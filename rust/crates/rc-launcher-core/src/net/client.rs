//! Integrated network client (task 3): DNS optimisation + proxy + connection
//! reuse + timeouts/backoff + mirror fallback.
//!
//! [`NetworkClient`] ties together the [`mirror`], [`dns`] and [`proxy`]
//! subsystems into a single `reqwest`-based client that:
//!
//! * resolves the protected (Mojang/Forge) hosts through DoH or static
//!   overrides, defeating DNS poisoning;
//! * enables Happy Eyeballs + connection pooling (reuse) via the `hickory-dns`
//!   resolver and `resolve_to_addrs` overrides;
//! * honours a configurable HTTP/HTTPS/SOCKS5 proxy;
//! * retries each candidate URL with exponential backoff, and transparently
//!   falls back from the origin to the fastest mirrors in priority order.
//!
//! Crucially, [`NetworkClient`] also implements the download crate's
//! [`HttpSource`](crate::download::client::HttpSource), so the resumable
//! [`DownloadManager`](crate::download::DownloadManager) (task 2) automatically
//! inherits mirror fallback, DoH and proxy support with zero extra wiring.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, RANGE};
use reqwest::StatusCode;
use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::download::compute_backoff;
use crate::download::{FetchResult, HttpSource};
use crate::error::{RcError, RcResult};
use crate::net::dns::{self, DnsConfig, DnsMode};
use crate::net::mirror::{MirrorProvider, MirrorSource, MOJANG_HOSTS};
use crate::net::proxy::ProxyConfig;

const DEFAULT_USER_AGENT: &str = concat!("RC-Launcher/", env!("CARGO_PKG_VERSION"));

/// Tunables for [`NetworkClient`].
#[derive(Debug, Clone)]
pub struct NetworkConfig {
    pub dns: DnsConfig,
    pub proxy: ProxyConfig,
    /// Per-connection connect timeout.
    pub connect_timeout: Duration,
    /// Overall request (read) timeout.
    pub read_timeout: Duration,
    pub user_agent: String,
    /// TCP keep-alive (connection reuse).
    pub tcp_keepalive: Option<Duration>,
    /// Idle pooled connection lifetime (connection reuse).
    pub pool_idle_timeout: Option<Duration>,
    /// Max idle connections kept per host (connection reuse).
    pub pool_max_idle_per_host: usize,
    /// Max attempts per candidate URL (mirror fallback multiplies this).
    pub max_retries: u32,
    /// Base exponential-backoff delay (doubled each retry).
    pub retry_base: Duration,
    /// Upper bound for a single backoff delay.
    pub retry_max: Duration,
    /// Jitter fraction (0.0 = none) applied to backoff.
    pub retry_jitter: f64,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            dns: DnsConfig::default(),
            proxy: ProxyConfig::None,
            connect_timeout: Duration::from_secs(15),
            read_timeout: Duration::from_secs(60),
            user_agent: DEFAULT_USER_AGENT.to_string(),
            tcp_keepalive: Some(Duration::from_secs(60)),
            pool_idle_timeout: Some(Duration::from_secs(90)),
            pool_max_idle_per_host: 8,
            max_retries: 4,
            retry_base: Duration::from_millis(400),
            retry_max: Duration::from_secs(20),
            retry_jitter: 0.25,
        }
    }
}

/// A single GET attempt, abstracted so the mirror-fallback + retry algorithm can
/// be unit-tested offline with a [`MockFetcher`](crate) (see tests).
#[async_trait]
pub(crate) trait Fetcher: Send + Sync {
    /// `Ok(true)` on HTTP success (2xx / 206), `Ok(false)` on a
    /// non-retryable HTTP error, `Err` on a transport failure.
    async fn fetch_once(&self, url: &str) -> RcResult<bool>;
}

/// Production fetcher backed by a `reqwest::Client`.
struct RealFetcher<'a> {
    client: &'a reqwest::Client,
}

#[async_trait]
impl Fetcher for RealFetcher<'_> {
    async fn fetch_once(&self, url: &str) -> RcResult<bool> {
        match self.client.get(url).send().await {
            Ok(r) if r.status().is_success() || r.status() == StatusCode::PARTIAL_CONTENT => {
                Ok(true)
            }
            Ok(_) => Ok(false),
            Err(e) => Err(RcError::Network(format!("request to {url}: {e}"))),
        }
    }
}

/// Try each candidate URL in order; for each, retry transport failures with
/// exponential backoff. Returns the index of the first successful candidate, or
/// the last error if all fail. HTTP errors (e.g. 404) are *not* retried — they
/// move on to the next candidate (mirror) immediately.
pub(crate) async fn try_candidates<F: Fetcher + ?Sized>(
    fetcher: &F,
    candidates: &[String],
    cfg: &NetworkConfig,
) -> RcResult<usize> {
    let mut last_err: Option<RcError> = None;
    for (i, cand) in candidates.iter().enumerate() {
        let mut attempt: u32 = 0;
        loop {
            match fetcher.fetch_once(cand).await {
                Ok(true) => return Ok(i),
                Ok(false) => {
                    last_err = Some(RcError::Download(format!(
                        "non-retryable HTTP error for {cand}"
                    )));
                    break;
                }
                Err(e) => {
                    attempt += 1;
                    if attempt > cfg.max_retries {
                        last_err = Some(e);
                        break;
                    }
                    let backoff =
                        compute_backoff(attempt, cfg.retry_base, cfg.retry_max, cfg.retry_jitter);
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }
    Err(last_err.unwrap_or_else(|| RcError::Other("all candidates failed".into())))
}

/// The integrated, China-mainland-optimised network client.
pub struct NetworkClient {
    client: reqwest::Client,
    config: NetworkConfig,
    mirrors: MirrorProvider,
    /// Resolved address overrides applied to the client (for transparency/logs).
    overrides: HashMap<String, Vec<IpAddr>>,
}

impl NetworkClient {
    /// Start building a client with sensible defaults.
    pub fn builder() -> NetworkClientBuilder {
        NetworkClientBuilder::default()
    }

    pub fn config(&self) -> &NetworkConfig {
        &self.config
    }
    pub fn mirror_provider(&self) -> &MirrorProvider {
        &self.mirrors
    }
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }
    pub fn dns_overrides(&self) -> &HashMap<String, Vec<IpAddr>> {
        &self.overrides
    }

    /// Candidate URLs for `url`: original first, then mirrors (preferred first).
    pub fn candidate_urls(&self, url: &str) -> Vec<String> {
        let mut out = vec![url.to_string()];
        out.extend(self.mirrors.rewrite_all(url));
        out
    }

    /// Fetch a URL, trying each candidate (origin + mirrors) with retries and
    /// exponential backoff. Returns the first successful response.
    pub async fn get(&self, url: &str) -> RcResult<reqwest::Response> {
        let candidates = self.candidate_urls(url);
        let idx = try_candidates(
            &RealFetcher {
                client: &self.client,
            },
            &candidates,
            &self.config,
        )
        .await?;
        let chosen = &candidates[idx];
        self.client
            .get(chosen)
            .send()
            .await
            .map_err(|e| RcError::Network(format!("request to {chosen}: {e}")))
    }

    /// Convenience: fetch the full body bytes.
    pub async fn fetch_bytes(&self, url: &str) -> RcResult<Vec<u8>> {
        let resp = self.get(url).await?;
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| RcError::Network(format!("read body: {e}")))
    }

    /// Convenience: fetch and parse JSON.
    pub async fn fetch_json<T: serde::de::DeserializeOwned>(&self, url: &str) -> RcResult<T> {
        let resp = self.get(url).await?;
        let txt = resp
            .text()
            .await
            .map_err(|e| RcError::Network(format!("read body: {e}")))?;
        serde_json::from_str(&txt).map_err(RcError::Json)
    }

    /// Measure mirrors and pin the fastest reachable one.
    pub async fn auto_select_mirror(&self) -> Option<String> {
        self.mirrors.speed_test(&self.client).await
    }
}

/// Builder for [`NetworkClient`].
#[derive(Clone)]
pub struct NetworkClientBuilder {
    config: NetworkConfig,
    mirrors: Vec<MirrorSource>,
    /// Hosts to protect via DoH/static resolution (default: [`MOJANG_HOSTS`]).
    protected_hosts: Vec<String>,
}

impl Default for NetworkClientBuilder {
    fn default() -> Self {
        Self {
            config: NetworkConfig::default(),
            mirrors: crate::net::mirror::default_mirrors(),
            protected_hosts: MOJANG_HOSTS.iter().map(|s| s.to_string()).collect(),
        }
    }
}

impl NetworkClientBuilder {
    pub fn config(mut self, c: NetworkConfig) -> Self {
        self.config = c;
        self
    }
    pub fn dns(mut self, d: DnsConfig) -> Self {
        self.config.dns = d;
        self
    }
    pub fn proxy(mut self, p: ProxyConfig) -> Self {
        self.config.proxy = p;
        self
    }
    pub fn mirrors(mut self, m: Vec<MirrorSource>) -> Self {
        self.mirrors = m;
        self
    }
    pub fn add_mirror(mut self, m: MirrorSource) -> Self {
        self.mirrors.push(m);
        self
    }
    pub fn protected_hosts(mut self, h: Vec<String>) -> Self {
        self.protected_hosts = h;
        self
    }

    /// Finalise: resolve DNS overrides (if any) and build the client.
    pub async fn build(self) -> RcResult<NetworkClient> {
        let proxy = self.config.proxy.to_reqwest()?;

        // 1) Bootstrap client (used for DoH resolution). It honours the proxy so
        //    DoH can reach upstreams from the China mainland, but does NOT apply
        //    any address overrides of its own.
        let mut bootstrap = reqwest::Client::builder()
            .user_agent(&self.config.user_agent)
            .connect_timeout(self.config.connect_timeout)
            .timeout(self.config.read_timeout)
            .pool_max_idle_per_host(self.config.pool_max_idle_per_host);
        if let Some(p) = &proxy {
            bootstrap = bootstrap.proxy(p.clone());
        }
        let bootstrap = bootstrap
            .build()
            .map_err(|e| RcError::Other(format!("build bootstrap client: {e}")))?;

        // 2) Compute DNS overrides for the protected hosts.
        let mut overrides: HashMap<String, Vec<IpAddr>> = HashMap::new();
        match &self.config.dns.mode {
            DnsMode::Static(map) => {
                for h in &self.protected_hosts {
                    if let Some(ips) = map.get(h) {
                        if !ips.is_empty() {
                            overrides.insert(h.clone(), ips.clone());
                        }
                    }
                }
            }
            DnsMode::Doh { .. } => {
                for h in &self.protected_hosts {
                    match dns::resolve_host(&self.config.dns, h, &bootstrap).await {
                        Ok(ips) => {
                            overrides.insert(h.clone(), ips);
                        }
                        Err(_e) => {
                            // Non-fatal: the connector falls back to the system
                            // resolver at connect time for this host.
                        }
                    }
                }
            }
            DnsMode::System => {}
        }

        // 3) Final client with overrides, connection reuse, timeouts, proxy.
        let mut builder = reqwest::Client::builder()
            .user_agent(&self.config.user_agent)
            .connect_timeout(self.config.connect_timeout)
            .timeout(self.config.read_timeout)
            .pool_max_idle_per_host(self.config.pool_max_idle_per_host);
        if let Some(ka) = self.config.tcp_keepalive {
            builder = builder.tcp_keepalive(ka);
        }
        if let Some(idle) = self.config.pool_idle_timeout {
            builder = builder.pool_idle_timeout(idle);
        }
        if self.config.dns.happy_eyeballs {
            // Happy Eyeballs (IPv4/IPv6 racing) at the resolver/connector layer.
            builder = builder.hickory_dns(true);
        }
        if let Some(p) = &proxy {
            builder = builder.proxy(p.clone());
        }
        for (host, ips) in &overrides {
            let addrs: Vec<SocketAddr> = ips.iter().map(|ip| SocketAddr::new(*ip, 0)).collect();
            builder = builder.resolve_to_addrs(host, &addrs);
        }
        let client = builder
            .build()
            .map_err(|e| RcError::Other(format!("build client: {e}")))?;

        Ok(NetworkClient {
            client,
            config: self.config,
            mirrors: MirrorProvider::new(self.mirrors),
            overrides,
        })
    }
}

/// Parse the total size out of a `Content-Range` header (`bytes 0-100/2000`).
fn parse_content_range_total(v: &str) -> Option<u64> {
    let slash = v.rfind('/')?;
    v[slash + 1..].trim().parse::<u64>().ok()
}

/// [`HttpSource`] lets the resumable [`DownloadManager`] (task 2) use this
/// network client directly — automatically gaining mirror fallback, DoH and
/// proxy support, with Range-based resume preserved.
#[async_trait]
impl HttpSource for NetworkClient {
    async fn fetch_range(&self, url: &str, start: u64, end: Option<u64>) -> RcResult<FetchResult> {
        let range_value = match end {
            Some(e) => format!("bytes={start}-{e}"),
            None => format!("bytes={start}-"),
        };
        let resp = self.send_range(url, &range_value).await?;
        let status = resp.status();
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
            .map_err(|e| RcError::Network(format!("read body: {e}")))?;
        Ok(FetchResult {
            bytes: bytes.to_vec(),
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
        let resp = self.send_range(url, &range_value).await?;
        // Stream the body incrementally so a multi-MiB chunk never lives
        // entirely in RAM (task 25 — large-file streaming download).
        let mut stream = resp.bytes_stream();
        let mut total: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| RcError::Network(format!("read body: {e}")))?;
            if chunk.is_empty() {
                continue;
            }
            writer.write_all(&chunk).await.map_err(RcError::Io)?;
            total += chunk.len() as u64;
        }
        Ok(total)
    }
}

impl NetworkClient {
    /// Send a `Range` request, trying each candidate (origin + mirrors) with
    /// retries and exponential backoff, returning the first `200`/`206`
    /// `Response`. The caller is responsible for reading the body — either into
    /// a buffered `Vec<u8>` ([`HttpSource::fetch_range`]) or straight into a
    /// file ([`HttpSource::fetch_range_into`]). Shared so both paths inherit the
    /// same mirror fallback + resilience.
    async fn send_range(&self, url: &str, range_value: &str) -> RcResult<reqwest::Response> {
        let candidates = self.candidate_urls(url);
        let mut last_err: Option<RcError> = None;
        for cand in &candidates {
            let mut attempt: u32 = 0;
            loop {
                let resp = match self
                    .client
                    .get(cand)
                    .header(RANGE, range_value)
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        attempt += 1;
                        if attempt > self.config.max_retries {
                            last_err = Some(RcError::Network(format!("request to {cand}: {e}")));
                            break;
                        }
                        let backoff = compute_backoff(
                            attempt,
                            self.config.retry_base,
                            self.config.retry_max,
                            self.config.retry_jitter,
                        );
                        tokio::time::sleep(backoff).await;
                        continue;
                    }
                };
                let status = resp.status();
                if status != StatusCode::PARTIAL_CONTENT && status != StatusCode::OK {
                    last_err = Some(RcError::Download(format!("HTTP {status} for {cand}")));
                    break;
                }
                return Ok(resp);
            }
        }
        Err(last_err
            .unwrap_or_else(|| RcError::Download(format!("all candidates failed for {url}"))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::mirror::MirrorProvider;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// Local, cloneable scripted outcome for the offline fetcher (`RcError` is not
    /// `Clone`, so we avoid storing it directly).
    #[derive(Clone)]
    enum Fake {
        Ok(bool),
        Err,
    }

    /// Offline fetcher that replays a per-URL script of outcomes.
    ///
    /// URLs with a script repeat their *last* outcome once the script is
    /// exhausted (so a script of `[Err]` keeps failing). URLs with no script
    /// default to success.
    struct MockFetcher {
        queue: Mutex<HashMap<String, Vec<Fake>>>,
        idx: Mutex<HashMap<String, usize>>,
        calls: Mutex<Vec<String>>,
    }

    impl MockFetcher {
        fn new() -> Self {
            Self {
                queue: Mutex::new(Default::default()),
                idx: Mutex::new(Default::default()),
                calls: Mutex::new(Vec::new()),
            }
        }
        fn script(self, url: &str, outcomes: Vec<RcResult<bool>>) -> Self {
            let mapped: Vec<Fake> = outcomes
                .into_iter()
                .map(|r| match r {
                    Ok(b) => Fake::Ok(b),
                    Err(_) => Fake::Err,
                })
                .collect();
            self.queue.lock().unwrap().insert(url.to_string(), mapped);
            self
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Fetcher for MockFetcher {
        async fn fetch_once(&self, url: &str) -> RcResult<bool> {
            self.calls.lock().unwrap().push(url.to_string());
            let seq = {
                let mut q = self.queue.lock().unwrap();
                q.entry(url.to_string()).or_default().clone()
            };
            if seq.is_empty() {
                return Ok(true);
            }
            let i = {
                let mut idx = self.idx.lock().unwrap();
                let e = idx.entry(url.to_string()).or_insert(0);
                let v = *e;
                if *e < seq.len() {
                    *e += 1;
                }
                v
            };
            let clamped = i.min(seq.len() - 1);
            match &seq[clamped] {
                Fake::Ok(b) => Ok(*b),
                Fake::Err => Err(RcError::Network(format!("mock transport error for {url}"))),
            }
        }
    }

    fn test_cfg() -> NetworkConfig {
        NetworkConfig {
            max_retries: 3,
            retry_base: Duration::from_millis(1),
            retry_max: Duration::from_millis(2),
            retry_jitter: 0.0,
            ..Default::default()
        }
    }

    fn candidates_for(mp: &MirrorProvider, url: &str) -> Vec<String> {
        let mut c = vec![url.to_string()];
        c.extend(mp.rewrite_all(url));
        c
    }

    #[tokio::test]
    async fn mirror_fallback_tries_origin_then_mirror() {
        let mp = MirrorProvider::new_default();
        let origin = "https://launcher.mojang.com/v1/objects/a/b.jar".to_string();
        let bm = mp.rewrite_all(&origin)[0].clone(); // first mirror = bmclapi
        let mock = MockFetcher::new()
            .script(&origin, vec![Err(RcError::Network("boom".into()))])
            .script(&bm, vec![Ok(true)]);
        let candidates = candidates_for(&mp, &origin);
        let idx = try_candidates(&mock, &candidates, &test_cfg())
            .await
            .unwrap();
        // The mirror (index 1) is the one that ultimately succeeds.
        assert_eq!(idx, 1);
        let calls = mock.calls();
        // The origin is tried first and retried (backoff) before we fall back.
        assert_eq!(calls[0], origin);
        assert!(
            calls.iter().filter(|c| *c == &origin).count() > 1,
            "origin should be retried"
        );
        // The fastest mirror is eventually tried and succeeds.
        assert!(calls.contains(&bm));
    }

    #[tokio::test]
    async fn retries_with_backoff_on_transport_error() {
        let origin = "https://launcher.mojang.com/x".to_string();
        let mock = MockFetcher::new().script(
            &origin,
            vec![
                Err(RcError::Network("e1".into())),
                Err(RcError::Network("e2".into())),
                Ok(true),
            ],
        );
        let candidates = vec![origin.clone()];
        let idx = try_candidates(&mock, &candidates, &test_cfg())
            .await
            .unwrap();
        assert_eq!(idx, 0);
        assert_eq!(mock.calls().len(), 3);
    }

    #[tokio::test]
    async fn all_fail_returns_error() {
        let origin = "https://launcher.mojang.com/x".to_string();
        let bm = "https://bmclapi2.bangbang93.com/x".to_string();
        let mock = MockFetcher::new()
            .script(&origin, vec![Err(RcError::Network("e".into()))])
            .script(&bm, vec![Err(RcError::Network("e".into()))]);
        let candidates = vec![origin.clone(), bm.clone()];
        let r = try_candidates(&mock, &candidates, &test_cfg()).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn build_client_without_network() {
        // Default config uses the System resolver -> no network during build.
        let client = NetworkClient::builder().build().await.unwrap();
        let url = "https://launcher.mojang.com/v1/objects/a/b.jar";
        let cands = client.candidate_urls(url);
        assert_eq!(cands[0], url);
        assert!(cands.iter().any(|c| c.contains("bmclapi2.bangbang93.com")));
        assert!(cands.iter().any(|c| c.contains("download.mcbbs.net")));
        assert!(cands
            .iter()
            .any(|c| c.contains("mirrors.aliyun.com/minecraft")));
    }

    #[tokio::test]
    async fn static_dns_overrides_applied() {
        use std::net::Ipv4Addr;
        let mut map = HashMap::new();
        map.insert(
            "launcher.mojang.com".to_string(),
            vec![IpAddr::V4(Ipv4Addr::new(1, 2, 3, 4))],
        );
        let client = NetworkClient::builder()
            .dns(DnsConfig::static_map(map))
            .build()
            .await
            .unwrap();
        assert!(client.dns_overrides().contains_key("launcher.mojang.com"));
    }

    #[tokio::test]
    async fn proxy_config_builds() {
        let client = NetworkClient::builder()
            .proxy(ProxyConfig::socks5("socks5://127.0.0.1:1080"))
            .build()
            .await
            .unwrap();
        // The proxy is applied internally; we just assert the client built.
        assert!(client.config().proxy.to_reqwest().unwrap().is_some());
    }
}
