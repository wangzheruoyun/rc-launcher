//! DNS optimisation (task 3): DoH resolvers, static/custom resolvers, Happy
//! Eyeballs awareness and caching.
//!
//! Accessing `mojang.com` / `forge` from the China mainland is frequently slow
//! or blocked by DNS poisoning. This module lets the launcher:
//!
//! * resolve hostnames through trusted **DNS-over-HTTPS** upstreams
//!   (`DnsMode::Doh`) — bypassing the (possibly poisoned) local resolver;
//! * pin explicit **static** address overrides (`DnsMode::Static`) for known
//!   hosts;
//! * fall back to the **system** resolver (`DnsMode::System`);
//! * enable **Happy Eyeballs** (IPv4/IPv6 racing) at the connector layer.
//!
//! The resolved addresses are turned into `reqwest` `resolve_to_addrs`
//! overrides by [`crate::net::client`], so connection reuse and Happy Eyeballs
//! apply to them transparently.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::{RcError, RcResult};

/// How hostnames are resolved before connecting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DnsMode {
    /// Use the platform/system resolver (reqwest default).
    #[default]
    System,
    /// Explicit host -> addresses overrides. Bypasses the system resolver, which
    /// is the primary mitigation for DNS poisoning.
    Static(HashMap<String, Vec<IpAddr>>),
    /// Resolve through one or more DNS-over-HTTPS (DoH) upstreams.
    Doh { servers: Vec<String> },
}

/// Built-in DoH upstreams (domestic first, then global fallback).
pub fn default_doh_servers() -> Vec<String> {
    vec![
        "https://dns.alidns.com/dns-query".to_string(), // Aliyun (domestic)
        "https://doh.pub/dns-query".to_string(),        // DNSPod / Tencent (domestic)
        "https://doh.360.cn/dns-query".to_string(),     // 360 (domestic)
        "https://cloudflare-dns.com/dns-query".to_string(),
        "https://dns.google/resolve".to_string(),
    ]
}

/// Network DNS configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsConfig {
    pub mode: DnsMode,
    /// Enable Happy Eyeballs (race IPv4/IPv6). Relies on the `hickory-dns`
    /// reqwest feature and/or multiple resolved addresses.
    pub happy_eyeballs: bool,
    /// How long resolved addresses are cached by the caller.
    pub cache_ttl: Duration,
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            mode: DnsMode::System,
            happy_eyeballs: true,
            cache_ttl: Duration::from_secs(300),
        }
    }
}

impl DnsConfig {
    pub fn system() -> Self {
        Self::default()
    }
    pub fn static_map(map: HashMap<String, Vec<IpAddr>>) -> Self {
        Self {
            mode: DnsMode::Static(map),
            ..Default::default()
        }
    }
    pub fn doh(servers: Vec<String>) -> Self {
        Self {
            mode: DnsMode::Doh { servers },
            ..Default::default()
        }
    }
}

/// Build a Cloudflare/Google-style DoH JSON query URL (`application/dns-json`).
///
/// `server` may already contain a path/query (e.g. `dns.google/resolve`), in
/// which case we append with `&` instead of `?`.
pub fn doh_query_url(server: &str, name: &str, rtype: &str) -> String {
    let sep = if server.contains('?') { '&' } else { '?' };
    format!(
        "{}{}name={}&type={}",
        server.trim_end_matches('/'),
        sep,
        urlencode(name),
        rtype
    )
}

/// Minimal percent-encoding sufficient for DNS names (alnum, `.`, `-`, `_`, `~`).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Parse a DoH JSON response (RFC 8484 `application/dns-json`).
///
/// Returns the A (`type == 1`) and AAAA (`type == 28`) records as [`IpAddr`]s.
pub fn parse_doh_json(body: &str) -> RcResult<Vec<IpAddr>> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| RcError::Other(format!("invalid DoH JSON: {e}")))?;
    let status = v.get("Status").and_then(|s| s.as_u64()).unwrap_or(2);
    if status != 0 {
        return Err(RcError::Other(format!(
            "DoH query failed with status {status}"
        )));
    }
    let mut ips = Vec::new();
    if let Some(answers) = v.get("Answer").and_then(|a| a.as_array()) {
        for a in answers {
            let rtype = a.get("type").and_then(|t| t.as_u64()).unwrap_or(0);
            // type 1 = A, type 28 = AAAA
            if rtype == 1 || rtype == 28 {
                if let Some(data) = a.get("data").and_then(|d| d.as_str()) {
                    if let Ok(ip) = data.parse::<IpAddr>() {
                        ips.push(ip);
                    }
                }
            }
        }
    }
    Ok(ips)
}

/// Resolve `host` using the configured mode. `client` (a plain bootstrap client
/// that already honours the proxy) is used for DoH queries.
/// Resolve `host` using the configured mode. `client` (a plain bootstrap client
/// that already honours the proxy) is used for DoH queries. `cache`, when
/// provided, is consulted first (TTL-guarded) and refreshed on every successful
/// resolution, so repeated lookups of the same host avoid redundant DoH / network
/// work — a real speed-up when many URLs share one Mojang host.
pub async fn resolve_host(
    config: &DnsConfig,
    host: &str,
    client: &reqwest::Client,
    cache: Option<&DnsCache>,
) -> RcResult<Vec<IpAddr>> {
    // 1) Cache hit (TTL-guarded) short-circuits the whole resolution.
    if let Some(c) = cache {
        if let Some(ips) = c.get(host) {
            return Ok(ips);
        }
    }

    let result: RcResult<Vec<IpAddr>> = match &config.mode {
        DnsMode::Static(map) => {
            if let Some(ips) = map.get(host) {
                if !ips.is_empty() {
                    Ok(ips.clone())
                } else {
                    Err(RcError::Network(format!("no static addresses for {host}")))
                }
            } else {
                // Unknown host -> fall back to the system resolver.
                resolve_via_system(host).await
            }
        }
        DnsMode::System => resolve_via_system(host).await,
        DnsMode::Doh { servers } => {
            match doh_resolve(servers, host, client).await {
                Ok(ips) => Ok(ips),
                Err(e) => {
                    // Graceful degradation: if *every* DoH upstream failed
                    // (network jitter / blocked), fall back to the system
                    // resolver rather than hard-failing the launch.
                    resolve_via_system(host)
                        .await
                        .map_err(|_| RcError::Network(e))
                }
            }
        }
    };

    // 2) Populate the cache on success so subsequent resolutions are instant.
    if let (Some(c), Ok(ips)) = (cache, &result) {
        c.insert(host, ips.clone(), config.cache_ttl);
    }
    result
}

/// System/resolver lookup used as both the `System` mode and the fallback for
/// `Static` (unknown host) and `Doh` (all upstreams failed).
async fn resolve_via_system(host: &str) -> RcResult<Vec<IpAddr>> {
    let addrs = tokio::net::lookup_host((host, 0))
        .await
        .map_err(|e| RcError::Network(format!("system resolve {host}: {e}")))?
        .map(|sa| sa.ip())
        .collect::<Vec<_>>();
    if addrs.is_empty() {
        return Err(RcError::Network(format!("no addresses for {host}")));
    }
    Ok(addrs)
}

/// Resolve `host` through the DoH upstreams, returning the first server that
/// yields any A/AAAA record. On total failure returns the last error string so
/// the caller can fall back to the system resolver.
async fn doh_resolve(
    servers: &[String],
    host: &str,
    client: &reqwest::Client,
) -> Result<Vec<IpAddr>, String> {
    let mut last_err: Option<String> = None;
    for server in servers {
        let mut collected: Vec<IpAddr> = Vec::new();
        for rtype in ["A", "AAAA"] {
            let url = doh_query_url(server, host, rtype);
            let resp = match client.get(&url).send().await {
                Ok(r) if r.status().is_success() => r,
                Ok(r) => {
                    last_err = Some(format!("{server} -> {}", r.status()));
                    continue;
                }
                Err(e) => {
                    last_err = Some(format!("{server} -> {e}"));
                    continue;
                }
            };
            let body = match resp.text().await {
                Ok(t) => t,
                Err(e) => {
                    last_err = Some(format!("DoH body error: {e}"));
                    continue;
                }
            };
            match parse_doh_json(&body) {
                Ok(ips) => collected.extend(ips),
                Err(e) => last_err = Some(e.to_string()),
            }
        }
        if !collected.is_empty() {
            return Ok(collected);
        }
    }
    Err(last_err.unwrap_or_else(|| format!("DoH resolution of {host} failed")))
}

/// A tiny, thread-safe, TTL-guarded DNS cache.
///
/// Resolving Mojang/Forge hosts through DoH or the system resolver costs one or
/// more round-trips, yet many distinct URLs share the same *host*. Caching the
/// resolved addresses for [`DnsConfig::cache_ttl`] keeps repeated downloads of
/// the same host from re-resolving on every chunk / mirror probe — a meaningful
/// speed-up on lossy China-mainland networks.
#[derive(Debug, Default)]
pub struct DnsCache {
    map: Mutex<HashMap<String, CacheEntry>>,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    addrs: Vec<IpAddr>,
    expires_at: Instant,
}

impl DnsCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up `host`, returning cached addresses only while they have not
    /// expired (TTL-guarded). Expired entries are treated as a miss.
    pub fn get(&self, host: &str) -> Option<Vec<IpAddr>> {
        let guard = self.map.lock().unwrap();
        let entry = guard.get(host)?;
        if entry.expires_at >= Instant::now() {
            Some(entry.addrs.clone())
        } else {
            None
        }
    }

    /// Insert/refresh `host` -> `addrs` with the given TTL.
    pub fn insert(&self, host: &str, addrs: Vec<IpAddr>, ttl: Duration) {
        let mut guard = self.map.lock().unwrap();
        guard.insert(
            host.to_string(),
            CacheEntry {
                addrs,
                expires_at: Instant::now() + ttl,
            },
        );
    }

    /// Drop every cached entry (e.g. when the network changes).
    pub fn clear(&self) {
        self.map.lock().unwrap().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    #[test]
    fn doh_query_url_appends_params() {
        let u = doh_query_url("https://dns.google/resolve", "example.com", "A");
        assert!(u.starts_with("https://dns.google/resolve?"));
        assert!(u.contains("name=example.com"));
        assert!(u.contains("type=A"));

        let u2 = doh_query_url("https://cloudflare-dns.com/dns-query", "foo.bar", "AAAA");
        assert!(u2.starts_with("https://cloudflare-dns.com/dns-query?"));
        assert!(u2.contains("name=foo.bar"));
        assert!(u2.contains("type=AAAA"));
    }

    #[test]
    fn parse_doh_json_extracts_ips() {
        let body = r#"{"Status":0,"Answer":[{"name":"example.com.","type":1,"TTL":300,"data":"93.184.216.34"},{"name":"example.com.","type":28,"TTL":300,"data":"2606:2800:220:1:248:1893:25c8:1946"}]}"#;
        let ips = parse_doh_json(body).unwrap();
        assert_eq!(ips.len(), 2);
        assert!(ips.contains(&"93.184.216.34".parse::<IpAddr>().unwrap()));
        assert!(ips.contains(
            &"2606:2800:220:1:248:1893:25c8:1946"
                .parse::<IpAddr>()
                .unwrap()
        ));
    }

    #[test]
    fn parse_doh_json_rejects_error_status() {
        let body = r#"{"Status":3,"Answer":[]}"#;
        assert!(parse_doh_json(body).is_err());
    }

    #[test]
    fn parse_doh_json_ignores_non_address_records() {
        let body = r#"{"Status":0,"Answer":[{"name":"x.","type":16,"TTL":1,"data":"txt"}]}"#;
        assert!(parse_doh_json(body).unwrap().is_empty());
    }

    #[test]
    fn default_doh_servers_include_domestic() {
        let s = default_doh_servers();
        assert!(s.iter().any(|x| x.contains("alidns")));
        assert!(s.iter().any(|x| x.contains("doh.pub")));
    }

    #[test]
    fn dns_config_defaults() {
        let c = DnsConfig::default();
        assert!(matches!(c.mode, DnsMode::System));
        assert!(c.happy_eyeballs);
    }

    #[test]
    fn dns_cache_stores_and_expires() {
        let cache = DnsCache::new();
        let ip = "93.184.216.34".parse::<IpAddr>().unwrap();
        // Not present initially.
        assert!(cache.get("example.com").is_none());
        // Insert with a 50ms TTL.
        cache.insert("example.com", vec![ip], Duration::from_millis(50));
        assert_eq!(cache.get("example.com").unwrap(), vec![ip]);
        // After the TTL elapses it is a miss again.
        std::thread::sleep(Duration::from_millis(70));
        assert!(cache.get("example.com").is_none());
        // clear() wipes everything.
        cache.insert("a.com", vec![ip], Duration::from_secs(60));
        cache.clear();
        assert!(cache.get("a.com").is_none());
    }

    #[test]
    fn resolve_host_static_returns_override() {
        let mut map = HashMap::new();
        let ip = "1.2.3.4".parse::<IpAddr>().unwrap();
        map.insert("launcher.mojang.com".to_string(), vec![ip]);
        let cfg = DnsConfig::static_map(map);
        let client = reqwest::Client::new();
        let cache = DnsCache::new();
        let out = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(resolve_host(&cfg, "launcher.mojang.com", &client, Some(&cache)))
            .unwrap();
        assert_eq!(out, vec![ip]);
        // The static result is cached for next time.
        assert_eq!(cache.get("launcher.mojang.com").unwrap(), vec![ip]);
    }

    #[test]
    fn resolve_host_static_falls_back_to_system() {
        let cfg = DnsConfig::static_map(HashMap::new());
        let client = reqwest::Client::new();
        let cache = DnsCache::new();
        // "localhost" is not in the (empty) static map, so we fall back to the
        // system resolver — which always resolves localhost offline.
        let out = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(resolve_host(&cfg, "localhost", &client, Some(&cache)))
            .unwrap();
        assert!(!out.is_empty());
    }

    #[test]
    fn resolve_host_doh_falls_back_to_system() {
        // Point DoH at an impossible upstream so every server fails; resolution
        // must then gracefully degrade to the system resolver for "localhost".
        let cfg = DnsConfig::doh(vec!["https://127.0.0.1:1/dns-query".to_string()]);
        let client = reqwest::Client::new();
        let cache = DnsCache::new();
        let out = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(resolve_host(&cfg, "localhost", &client, Some(&cache)))
            .unwrap();
        assert!(!out.is_empty());
    }
}
