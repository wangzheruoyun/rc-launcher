//! Mirror source management for the China mainland (task 3).
//!
//! A [`MirrorSource`] describes how to rewrite a canonical Mojang CDN URL onto a
//! domestic mirror. The built-in mirrors (BMCLAPI / MCBBS / Aliyun) are
//! *path-preserving*: the host of a canonical URL is replaced by the mirror's
//! `base_url` (optionally prefixed by `path_prefix`) while the path + query are
//! kept intact — this matches how those mirrors proxy the Mojang CDN.
//!
//! [`MirrorProvider`] measures each mirror's latency and selects the fastest
//! one; [`crate::net::client::NetworkClient`] then transparently retries failed
//! downloads against the mirrors in priority order.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Canonical Mojang / Minecraft hosts that the built-in mirrors can serve.
pub const MOJANG_HOSTS: &[&str] = &[
    "launcher.mojang.com",
    "launchermeta.mojang.com",
    "piston-meta.mojang.com",
    "piston-data.mojang.com",
    "resources.download.minecraft.net",
    "libraries.minecraft.net",
    "api.mojang.com",
    "authserver.mojang.com",
    "session.minecraft.net",
];

/// A single download mirror.
///
/// Mirrors are configured as *path-preserving* rewrites: the host of a canonical
/// URL is replaced by [`MirrorSource::base_url`] (optionally prefixed by
/// [`MirrorSource::path_prefix`]) while the rest of the path/query is kept
/// intact. This matches BMCLAPI, MCBBS and Aliyun which mirror the Mojang CDN by
/// path, so adding a custom mirror is just a matter of supplying `base_url` and
/// (optionally) a `path_prefix`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirrorSource {
    /// Stable identifier (e.g. `bmclapi`).
    pub id: String,
    /// Human readable name.
    pub name: String,
    /// Mirror base URL (scheme + host, no trailing slash).
    pub base_url: String,
    /// Optional path segment inserted between `base_url` and the original path.
    pub path_prefix: String,
    /// Hosts this mirror can serve. Empty means "any of [`MOJANG_HOSTS`]".
    pub hosts: Vec<String>,
}

impl MirrorSource {
    /// Create a mirror that serves every canonical host (path-preserving).
    pub fn new(id: &str, name: &str, base_url: &str) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            base_url: base_url.to_string(),
            path_prefix: String::new(),
            hosts: Vec::new(),
        }
    }

    /// Set an explicit path prefix (e.g. `minecraft` for Aliyun).
    pub fn with_path_prefix(mut self, prefix: &str) -> Self {
        self.path_prefix = prefix.to_string();
        self
    }

    /// Restrict the mirror to a subset of hosts.
    pub fn with_hosts(mut self, hosts: &[&str]) -> Self {
        self.hosts = hosts.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Whether this mirror can serve a URL with the given host.
    ///
    /// A mirror with an explicit [`MirrorSource::hosts`] list only serves those
    /// hosts. A mirror with an *empty* list is treated as a Mojang-CDN mirror
    /// and only serves the well-known [`MOJANG_HOSTS`] — we must never blindly
    /// redirect an arbitrary URL (e.g. `example.org`) to a mirror that does not
    /// host it.
    pub fn can_mirror(&self, host: &str) -> bool {
        if self.hosts.is_empty() {
            MOJANG_HOSTS.contains(&host)
        } else {
            self.hosts.iter().any(|h| h == host)
        }
    }

    /// Rewrite `url` onto this mirror, returning `None` when the mirror cannot
    /// serve it (unknown host) or the URL cannot be parsed.
    pub fn rewrite(&self, url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        let host = parsed.host_str()?;
        if !self.can_mirror(host) {
            return None;
        }
        let base = self.base_url.trim_end_matches('/');
        let prefix = self.path_prefix.trim_matches('/');
        let path = parsed.path(); // always begins with '/'
        let path_part = if prefix.is_empty() {
            path.to_string()
        } else {
            format!("/{}/{}", prefix, path.trim_start_matches('/'))
        };
        let query = parsed.query().map(|q| format!("?{q}")).unwrap_or_default();
        Some(format!("{base}{path_part}{query}"))
    }
}

/// Default mirror list for the China mainland (BMCLAPI / MCBBS / Aliyun).
pub fn default_mirrors() -> Vec<MirrorSource> {
    vec![
        MirrorSource::new("bmclapi", "BMCLAPI", "https://bmclapi2.bangbang93.com"),
        MirrorSource::new("mcbbs", "MCBBS", "https://download.mcbbs.net"),
        MirrorSource::new("aliyun", "Aliyun", "https://mirrors.aliyun.com/minecraft"),
    ]
}

/// Result of measuring a mirror's reachability / latency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorLatency {
    pub id: String,
    pub ok: bool,
    pub latency: Duration,
}

/// Selects and ranks mirrors by measured latency (fastest preferred).
pub struct MirrorProvider {
    mirrors: Vec<MirrorSource>,
    best: Mutex<Option<String>>,
    latencies: Mutex<HashMap<String, Duration>>,
}

impl MirrorProvider {
    pub fn new(mirrors: Vec<MirrorSource>) -> Self {
        Self {
            mirrors,
            best: Mutex::new(None),
            latencies: Mutex::new(HashMap::new()),
        }
    }

    /// A provider seeded with [`default_mirrors`].
    pub fn new_default() -> Self {
        Self::new(default_mirrors())
    }

    /// All configured mirrors.
    pub fn list(&self) -> &[MirrorSource] {
        &self.mirrors
    }

    /// Look up a mirror by id.
    pub fn get(&self, id: &str) -> Option<&MirrorSource> {
        self.mirrors.iter().find(|m| m.id == id)
    }

    /// Preferred mirror id (fastest measured), if known.
    pub fn best_id(&self) -> Option<String> {
        self.best.lock().unwrap().clone()
    }

    /// Pin the preferred mirror id.
    pub fn set_best(&self, id: &str) {
        *self.best.lock().unwrap() = Some(id.to_string());
    }

    /// Record a measured latency for a mirror.
    pub fn record(&self, id: &str, latency: Duration) {
        self.latencies
            .lock()
            .unwrap()
            .insert(id.to_string(), latency);
    }

    /// Rewrite `url` using the preferred mirror, if any; otherwise `None`.
    pub fn rewrite_best(&self, url: &str) -> Option<String> {
        let best = self.best_id()?;
        let m = self.get(&best)?;
        m.rewrite(url)
    }

    /// Rewrite `url` against every mirror that can serve it (preferred first).
    pub fn rewrite_all(&self, url: &str) -> Vec<String> {
        let mut out = Vec::new();
        let best = self.best_id();
        if let Some(id) = &best {
            if let Some(m) = self.get(id) {
                if let Some(u) = m.rewrite(url) {
                    out.push(u);
                }
            }
        }
        for m in &self.mirrors {
            if Some(&m.id) == best.as_ref() {
                continue;
            }
            if let Some(u) = m.rewrite(url) {
                out.push(u);
            }
        }
        out
    }

    /// Measure latency for every mirror by issuing a tiny ranged GET and
    /// recording time-to-first-byte. Any HTTP response (incl. 404) counts as
    /// reachable — we only care about how fast the mirror answers.
    pub async fn measure(&self, client: &reqwest::Client) -> Vec<MirrorLatency> {
        let mut out = Vec::with_capacity(self.mirrors.len());
        for m in &self.mirrors {
            let probe = probe_url(m);
            let start = std::time::Instant::now();
            let ok = match client
                .get(&probe)
                .header(reqwest::header::RANGE, "bytes=0-0")
                .send()
                .await
            {
                Ok(r) => {
                    r.status().is_success()
                        || r.status() == reqwest::StatusCode::PARTIAL_CONTENT
                        || r.status() == reqwest::StatusCode::NOT_FOUND
                }
                Err(_) => false,
            };
            let latency = start.elapsed();
            if ok {
                self.record(&m.id, latency);
            }
            out.push(MirrorLatency {
                id: m.id.clone(),
                ok,
                latency,
            });
        }
        out
    }

    /// Pick the fastest *reachable* mirror from a set of measurements.
    pub fn select_best(&self, measured: &[MirrorLatency]) -> Option<String> {
        measured
            .iter()
            .filter(|m| m.ok)
            .min_by_key(|m| m.latency)
            .map(|m| m.id.clone())
    }

    /// Measure all mirrors and pin the fastest reachable one.
    pub async fn speed_test(&self, client: &reqwest::Client) -> Option<String> {
        let measured = self.measure(client).await;
        let best = self.select_best(&measured);
        if let Some(b) = &best {
            self.set_best(b);
        }
        best
    }
}

/// A small, always-present file used to measure time-to-first-byte. A 404 is
/// still a valid latency measurement (the mirror answered quickly).
fn probe_url(m: &MirrorSource) -> String {
    let base = m.base_url.trim_end_matches('/');
    let prefix = m.path_prefix.trim_matches('/');
    if prefix.is_empty() {
        format!("{base}/favicon.ico")
    } else {
        format!("{base}/{prefix}/favicon.ico")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_known_mirrors() {
        let m = default_mirrors();
        assert!(m.iter().any(|x| x.id == "bmclapi"));
        assert!(m.iter().any(|x| x.id == "mcbbs"));
        assert!(m.iter().any(|x| x.id == "aliyun"));
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn rewrites_bmclapi_path() {
        let bm = MirrorSource::new("bmclapi", "BMCLAPI", "https://bmclapi2.bangbang93.com");
        let out = bm
            .rewrite("https://launcher.mojang.com/v1/objects/abc/x.jar")
            .unwrap();
        assert_eq!(out, "https://bmclapi2.bangbang93.com/v1/objects/abc/x.jar");
    }

    #[test]
    fn rewrites_with_path_prefix() {
        let al = MirrorSource::new("aliyun", "Aliyun", "https://mirrors.aliyun.com/minecraft");
        let out = al
            .rewrite("https://piston-meta.mojang.com/v1/packages/abc/y.json")
            .unwrap();
        assert_eq!(
            out,
            "https://mirrors.aliyun.com/minecraft/v1/packages/abc/y.json"
        );
    }

    #[test]
    fn can_mirror_filters_hosts() {
        let m =
            MirrorSource::new("x", "X", "https://example.com").with_hosts(&["launcher.mojang.com"]);
        assert!(m.can_mirror("launcher.mojang.com"));
        assert!(!m.can_mirror("libraries.minecraft.net"));
    }

    #[test]
    fn select_best_picks_fastest_reachable() {
        let p = MirrorProvider::new(default_mirrors());
        let measured = vec![
            MirrorLatency {
                id: "bmclapi".into(),
                ok: true,
                latency: Duration::from_millis(200),
            },
            MirrorLatency {
                id: "mcbbs".into(),
                ok: true,
                latency: Duration::from_millis(50),
            },
            MirrorLatency {
                id: "aliyun".into(),
                ok: false,
                latency: Duration::from_millis(10),
            },
        ];
        assert_eq!(p.select_best(&measured).as_deref(), Some("mcbbs"));
    }

    #[test]
    fn rewrite_all_prefers_best() {
        let p = MirrorProvider::new(default_mirrors());
        p.set_best("mcbbs");
        let urls = p.rewrite_all("https://launcher.mojang.com/v1/objects/a/b.jar");
        assert!(!urls.is_empty());
        assert!(urls[0].contains("download.mcbbs.net"));
        assert!(urls.iter().any(|u| u.contains("bmclapi2.bangbang93.com")));
    }

    #[test]
    fn preserve_query_string() {
        let bm = MirrorSource::new("bmclapi", "BMCLAPI", "https://bmclapi2.bangbang93.com");
        let out = bm
            .rewrite("https://resources.download.minecraft.net/ab/abcd?x=1")
            .unwrap();
        assert_eq!(out, "https://bmclapi2.bangbang93.com/ab/abcd?x=1");
    }

    #[test]
    fn non_mojang_host_is_not_rewritten() {
        let bm = MirrorSource::new("bmclapi", "BMCLAPI", "https://bmclapi2.bangbang93.com");
        assert!(bm.rewrite("https://example.org/foo").is_none());
    }
}
