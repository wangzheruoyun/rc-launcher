//! JRE package sources (task 6).
//!
//! A [`JreSource`] yields the raw bytes of a JRE archive slice. Two
//! implementations are provided:
//!
//! * [`LocalDirSource`] — reads the prebuilt packages straight from a directory
//!   laid out like FCL's `assets/app_runtime/java/`. This is what the unit
//!   tests and the bundled-assets path use.
//! * [`RemoteJreSource`] — downloads the packages over HTTP(S) via the
//!   network-optimised [`crate::download::HttpSource`] (task 3), so a
//!   device can fetch a missing ABI from a mirror without shipping every slice
//!   inside the APK.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::download::{FetchResult, HttpSource};
use crate::error::{RcError, RcResult};
use crate::runtime::java_version::JavaVersion;
use crate::runtime::manifest::JreArchive;

/// A backend that can fetch a JRE archive slice's bytes.
#[async_trait]
pub trait JreSource: Send + Sync {
    /// Return the full bytes of `artifact` for `version`.
    async fn read_artifact(&self, version: JavaVersion, artifact: &JreArchive)
        -> RcResult<Vec<u8>>;
}

/// Reads prebuilt packages from a local `java/` directory.
#[derive(Debug, Clone)]
pub struct LocalDirSource {
    /// Root directory containing one `jre<major>/` subdir per version.
    pub base_dir: PathBuf,
}

impl LocalDirSource {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }
}

#[async_trait]
impl JreSource for LocalDirSource {
    async fn read_artifact(
        &self,
        version: JavaVersion,
        artifact: &JreArchive,
    ) -> RcResult<Vec<u8>> {
        let path = self
            .base_dir
            .join(version.as_jre_dir())
            .join(&artifact.file);
        std::fs::read(&path).map_err(RcError::Io)
    }
}

/// Downloads packages over HTTP(S) from a base URL.
///
/// The URL for an archive is `<base_url>/<jre_dir>/<file>` (the same layout FCL
/// publishes on its mirrors). `HttpSource` already honours `Range`, so the
/// download inherits resume + Happy-Eyeballs + mirror selection from task 3.
pub struct RemoteJreSource {
    client: Arc<dyn HttpSource>,
    base_url: String,
}

impl RemoteJreSource {
    pub fn new(client: Arc<dyn HttpSource>, base_url: impl Into<String>) -> Self {
        Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    fn url(&self, version: JavaVersion, artifact: &JreArchive) -> String {
        format!("{}/{}", self.base_url, version.as_jre_dir())
            .trim_end_matches('/')
            .to_string()
            + "/"
            + &artifact.file
    }
}

#[async_trait]
impl JreSource for RemoteJreSource {
    async fn read_artifact(
        &self,
        version: JavaVersion,
        artifact: &JreArchive,
    ) -> RcResult<Vec<u8>> {
        let url = self.url(version, artifact);
        // Fetch the whole archive; ranges are handled internally by HttpSource
        // but here we want the complete object to verify + extract.
        let FetchResult { bytes, .. } = self
            .client
            .fetch_range(&url, 0, None)
            .await
            .map_err(|e| RcError::Download(format!("fetch {url}: {e}")))?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::manifest::{ArchiveKind, JreArchive};

    fn artifact(file: &str) -> JreArchive {
        JreArchive {
            kind: ArchiveKind::Universal,
            abi: None,
            file: file.to_string(),
            sha1: String::new(),
            size: 0,
        }
    }

    #[tokio::test]
    async fn local_source_reads_file() {
        let tmp = std::env::temp_dir().join(format!("rc-jre-src-{}", std::process::id()));
        let jre = tmp.join("jre17");
        std::fs::create_dir_all(&jre).unwrap();
        std::fs::write(jre.join("universal.tar.xz"), b"payload").unwrap();
        let src = LocalDirSource::new(&tmp);
        let data = src
            .read_artifact(JavaVersion::Java17, &artifact("universal.tar.xz"))
            .await
            .unwrap();
        assert_eq!(data, b"payload");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
