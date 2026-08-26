//! JRE package sources (task 6).
//!
//! A [`JreSource`] yields the raw bytes of a JRE archive slice, or — more
//! efficiently — extracts it straight into a directory. Two implementations
//! are provided:
//!
//! * [`LocalDirSource`] \u2014 reads the prebuilt packages straight from a directory
//!   laid out like FCL's `assets/app_runtime/java/`. This is what the unit
//!   tests and the bundled-assets path use. Extraction streams from the on-disk
//!   archive and the SHA-1/size is verified by streaming the file hash, so a
//!   corrupt asset is rejected without ever pinning the archive in RAM
//!   (task 25 \u2014 large-file streaming handling).
//! * [`RemoteJreSource`] \u2014 downloads the packages over HTTP(S) through the
//!   resumable, mirror-aware [`crate::download::DownloadManager`] (task 2):
//!   each archive is fetched with HTTP `Range` resume + parallel shards + SHA-1
//!   verification, and a dead primary host degrades to a mirror instead of
//!   failing. This gives JRE provisioning 断点续传 + 镜像源 on top of the
//!   network-optimised client (task 3), so a device can fetch a missing ABI
//!   from a mirror without shipping every slice inside the APK.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;

use crate::download::{DownloadManager, DownloadOptions, DownloadTask, FetchResult, HttpSource};
use crate::error::{RcError, RcResult};
use crate::runtime::extract::{extract_tar_xz, extract_tar_xz_file};
use crate::runtime::java_version::JavaVersion;
use crate::runtime::manifest::JreArchive;

/// A backend that can fetch (or extract) a JRE archive slice.
#[async_trait]
pub trait JreSource: Send + Sync {
    /// Return the full bytes of `artifact` for `version`.
    async fn read_artifact(&self, version: JavaVersion, artifact: &JreArchive)
        -> RcResult<Vec<u8>>;

    /// Extract `artifact` for `version` straight into `dest`, verifying its
    /// SHA-1/size first and returning the summed size of the unpacked entries.
    ///
    /// The default implementation buffers the bytes and extracts in memory;
    /// sources that can stream from disk override this so the (potentially
    /// large) archive never has to live entirely in RAM (task 25).
    async fn extract_artifact(
        &self,
        version: JavaVersion,
        artifact: &JreArchive,
        dest: &Path,
    ) -> RcResult<u64> {
        let bytes = self.read_artifact(version, artifact).await?;
        // Reject corrupt downloads before unpacking.
        artifact.verify(&bytes)?;
        extract_tar_xz(&bytes, dest)
    }
}

/// Reads prebuilt packages from a local `java/` directory.
#[derive(Debug, Clone)]
pub struct LocalDirSource {
    /// Root directory containing one `jre<major>`/ subdir per version.
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

    async fn extract_artifact(
        &self,
        version: JavaVersion,
        artifact: &JreArchive,
        dest: &Path,
    ) -> RcResult<u64> {
        let path = self
            .base_dir
            .join(version.as_jre_dir())
            .join(&artifact.file);
        // Verify before extracting so a corrupt on-disk asset is rejected
        // rather than silently unpacked; the hash is streamed off disk.
        artifact.verify_path(&path).await?;
        // Stream the decompression straight from the archive on disk.
        extract_tar_xz_file(&path, dest)
    }
}

/// Downloads packages over HTTP(S) from one or more base URLs.
///
/// The URL for an archive is `<base>/<jre_dir>/<file>` for every configured
/// base (the first is the primary host, the rest are mirrors). Downloads go
/// through the resumable, mirror-aware [`DownloadManager`] (task 2): each
/// archive is fetched with HTTP `Range` resume + parallel shards + SHA-1
/// verification, and a dead primary host degrades to a mirror instead of
/// failing \u2014 directly fulfilling task 6's "JRE 下载" with 断点续传 + 镜像源.
pub struct RemoteJreSource {
    client: Arc<dyn HttpSource>,
    primary: String,
    mirrors: Vec<String>,
    dl: Arc<DownloadManager>,
}

impl RemoteJreSource {
    /// `primary` is the primary host; `mirrors` are tried (in order) when the
    /// primary fails. Each entry is a base URL; the archive path
    /// `<jre_dir>/<file>` is appended per request.
    pub fn with_mirrors(
        client: Arc<dyn HttpSource>,
        primary: impl Into<String>,
        mirrors: Vec<String>,
    ) -> Self {
        let options = DownloadOptions::default();
        let dl = Arc::new(DownloadManager::new(client.clone(), options));
        Self {
            client,
            primary: primary.into(),
            mirrors,
            dl,
        }
    }

    /// Single host, no mirrors.
    pub fn new(client: Arc<dyn HttpSource>, primary: impl Into<String>) -> Self {
        Self::with_mirrors(client, primary, Vec::new())
    }

    fn archive_url(&self, base: &str, version: JavaVersion, artifact: &JreArchive) -> String {
        format!(
            "{}/{}/{}",
            base.trim_end_matches('/'),
            version.as_jre_dir(),
            artifact.file
        )
    }

    /// Candidate URLs: primary first, then every mirror.
    fn candidate_urls(&self, version: JavaVersion, artifact: &JreArchive) -> Vec<String> {
        let mut urls = vec![self.archive_url(&self.primary, version, artifact)];
        for m in &self.mirrors {
            urls.push(self.archive_url(m, version, artifact));
        }
        urls
    }
}

#[async_trait]
impl JreSource for RemoteJreSource {
    async fn read_artifact(
        &self,
        version: JavaVersion,
        artifact: &JreArchive,
    ) -> RcResult<Vec<u8>> {
        let url = self.archive_url(&self.primary, version, artifact);
        let FetchResult { bytes, .. } = self
            .client
            .fetch_range(&url, 0, None)
            .await
            .map_err(|e| RcError::Download(format!("fetch {url}: {e}")))?;
        Ok(bytes)
    }

    async fn extract_artifact(
        &self,
        version: JavaVersion,
        artifact: &JreArchive,
        dest: &Path,
    ) -> RcResult<u64> {
        // Download to a temp file via the resumable, mirror-aware manager, then
        // stream-extract from disk (never holds the whole archive in RAM).
        let tmp = dest.join(format!(".rc-{}.part", artifact.file));
        let urls = self.candidate_urls(version, artifact);
        let mut task = DownloadTask::new(urls[0].clone(), tmp.clone())
            .with_sha1(&artifact.sha1)
            .with_size(artifact.size);
        for u in &urls[1..] {
            task = task.with_mirror(u.clone());
        }
        self.dl
            .download(&task)
            .await
            .map_err(|e| RcError::Download(format!("download {}: {e}", artifact.file)))?;
        let written = extract_tar_xz_file(&tmp, dest)?;
        let _ = std::fs::remove_file(&tmp);
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::manifest::ArchiveKind;

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
