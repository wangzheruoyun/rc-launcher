//! Network layer for China-mainland optimisation (task 3).
//!
//! This module implements the network-optimisation subsystem required to make
//! Minecraft Java downloads reliable from the China mainland:
//!
//! * [`mirror`] — built-in BMCLAPI / MCBBS / Aliyun mirrors with automatic
//!   speed-testing and best-mirror selection, plus path-preserving URL
//!   rewriting.
//! * [`dns`] — DNS optimisation: DoH resolvers, static/custom resolvers (to
//!   bypass DNS poisoning), Happy Eyeballs awareness and caching.
//! * [`proxy`] — configurable HTTP / HTTPS / SOCKS5 proxy.
//! * [`client`] — an integrated [`NetworkClient`] that combines the above with
//!   connection reuse, timeouts and exponential-backoff retries, and
//!   transparently retries downloads against the mirrors. It also implements the
//!   download crate's [`HttpSource`](crate::download::client::HttpSource), so the
//!   resumable [`DownloadManager`](crate::download::DownloadManager) (task 2)
//!   automatically gains mirror fallback, DoH and proxy support.

mod client;
mod dns;
mod mirror;
mod proxy;

pub use client::{NetworkClient, NetworkClientBuilder, NetworkConfig};
pub use dns::{default_doh_servers, parse_doh_json, DnsCache, DnsConfig, DnsMode};
pub use mirror::{
    default_mirrors, extended_mirrors, MirrorLatency, MirrorMode, MirrorProvider, MirrorSource,
    MOJANG_HOSTS,
};
pub use proxy::ProxyConfig;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_known_mirrors() {
        let m = default_mirrors();
        assert!(m.iter().any(|x| x.id == "bmclapi"));
        assert!(m.iter().any(|x| x.id == "mcbbs"));
        assert!(m.iter().any(|x| x.id == "aliyun"));
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn mirror_source_is_serialisable() {
        let m = default_mirrors();
        let json = serde_json::to_string(&m).unwrap();
        let back: Vec<MirrorSource> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), m.len());
        assert_eq!(back[0].id, "bmclapi");
    }

    #[test]
    fn dns_config_is_serialisable() {
        let c = DnsConfig::default();
        let json = serde_json::to_string(&c).unwrap();
        let _: DnsConfig = serde_json::from_str(&json).unwrap();
    }
}
