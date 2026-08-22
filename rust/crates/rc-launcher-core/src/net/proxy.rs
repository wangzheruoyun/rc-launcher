//! Proxy support (task 3): HTTP / HTTPS / SOCKS5.
//!
//! A launcher running behind a campus/corporate network or a transparent
//! accelerator frequently needs an explicit proxy to reach Mojang / Forge.
//! [`ProxyConfig`] is a small, serialisable description that is turned into a
//! `reqwest::Proxy` by [`crate::net::client`].

use crate::error::{RcError, RcResult};

/// Proxy configuration. `None` disables proxying entirely.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ProxyConfig {
    #[default]
    None,
    Http(String),
    Https(String),
    Socks5(String),
}

impl ProxyConfig {
    pub fn none() -> Self {
        ProxyConfig::None
    }
    pub fn http(url: &str) -> Self {
        ProxyConfig::Http(url.to_string())
    }
    pub fn https(url: &str) -> Self {
        ProxyConfig::Https(url.to_string())
    }
    pub fn socks5(url: &str) -> Self {
        ProxyConfig::Socks5(url.to_string())
    }

    /// Build a `reqwest::Proxy`, or `None` when disabled.
    pub fn to_reqwest(&self) -> RcResult<Option<reqwest::Proxy>> {
        match self {
            ProxyConfig::None => Ok(None),
            ProxyConfig::Http(u) => reqwest::Proxy::http(u)
                .map(Some)
                .map_err(|e| RcError::Other(format!("invalid http proxy {u}: {e}"))),
            ProxyConfig::Https(u) => reqwest::Proxy::https(u)
                .map(Some)
                .map_err(|e| RcError::Other(format!("invalid https proxy {u}: {e}"))),
            ProxyConfig::Socks5(u) => reqwest::Proxy::all(u)
                .map(Some)
                .map_err(|e| RcError::Other(format!("invalid socks5 proxy {u}: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_yields_no_proxy() {
        assert!(ProxyConfig::None.to_reqwest().unwrap().is_none());
    }

    #[test]
    fn http_proxy_builds() {
        let p = ProxyConfig::http("http://127.0.0.1:7890")
            .to_reqwest()
            .unwrap();
        assert!(p.is_some());
    }

    #[test]
    fn https_proxy_builds() {
        let p = ProxyConfig::https("http://127.0.0.1:7890")
            .to_reqwest()
            .unwrap();
        assert!(p.is_some());
    }

    #[test]
    fn socks5_proxy_builds() {
        let p = ProxyConfig::socks5("socks5://127.0.0.1:1080")
            .to_reqwest()
            .unwrap();
        assert!(p.is_some());
    }

    #[test]
    fn default_is_none() {
        assert_eq!(ProxyConfig::default(), ProxyConfig::None);
    }
}
