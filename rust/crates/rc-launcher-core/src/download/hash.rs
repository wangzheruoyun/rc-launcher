//! Streaming SHA-1 / MD5 helpers for download verification (task 2).
//!
//! Verification streams the (potentially large) file in fixed-size blocks so it
//! never has to be held fully in memory. The same routines also hash small
//! in-memory buffers (used by tests and the FFI layer).

use std::path::Path;

use md5::Md5;
use sha1::Digest;
use sha1::Sha1;
use sha2::Sha256;

use crate::error::{RcError, RcResult};
use tokio::fs as tfs;
use tokio::io::AsyncReadExt;

/// Hash an in-memory byte slice with SHA-1, returned as lowercase hex.
pub fn sha1_bytes(data: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(data);
    hex(&h.finalize())
}

/// Hash an in-memory byte slice with MD5, returned as lowercase hex.
pub fn md5_bytes(data: &[u8]) -> String {
    let mut h = Md5::new();
    h.update(data);
    hex(&h.finalize())
}

/// Hash an in-memory byte slice with SHA-256, returned as lowercase hex.
pub fn sha256_bytes(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex(&h.finalize())
}

/// SHA-256 of a file on disk (streamed in fixed-size blocks).
pub async fn sha256_path(path: &Path) -> RcResult<String> {
    hash_path(path, Algo::Sha256).await
}

/// SHA-1 of a file on disk (streamed in fixed-size blocks).
pub async fn sha1_path(path: &Path) -> RcResult<String> {
    hash_path(path, Algo::Sha1).await
}

/// MD5 of a file on disk (streamed in fixed-size blocks).
pub async fn md5_path(path: &Path) -> RcResult<String> {
    hash_path(path, Algo::Md5).await
}

enum Algo {
    Sha1,
    Md5,
    Sha256,
}

async fn hash_path(path: &Path, algo: Algo) -> RcResult<String> {
    let mut file = tfs::File::open(path).await.map_err(RcError::Io)?;
    let mut buf = [0u8; 64 * 1024];
    match algo {
        Algo::Sha1 => {
            let mut h = Sha1::new();
            loop {
                let n = file.read(&mut buf).await.map_err(RcError::Io)?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
            Ok(hex(&h.finalize()))
        }
        Algo::Md5 => {
            let mut h = Md5::new();
            loop {
                let n = file.read(&mut buf).await.map_err(RcError::Io)?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
            Ok(hex(&h.finalize()))
        }
        Algo::Sha256 => {
            let mut h = Sha256::new();
            loop {
                let n = file.read(&mut buf).await.map_err(RcError::Io)?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
            Ok(hex(&h.finalize()))
        }
    }
}

/// Lowercase hex encoding without external dependencies.
pub fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

/// Case-insensitive hex string equality (mirror sources sometimes uppercase).
pub fn hex_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .all(|(x, y)| x.eq_ignore_ascii_case(&y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_known_vector() {
        assert_eq!(
            sha1_bytes(b"abc"),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
    }

    #[test]
    fn sha256_known_vector() {
        // SHA-256("abc")
        assert_eq!(
            sha256_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn md5_known_vector() {
        assert_eq!(md5_bytes(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn hex_eq_ignores_case() {
        assert!(hex_eq("ABCDEF", "abcdef"));
        assert!(!hex_eq("abcdef", "abcdeg"));
    }

    #[tokio::test]
    async fn sha256_path_matches_bytes() {
        let dir = std::env::temp_dir().join(format!("rc_sha256_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.bin");
        let data = vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        std::fs::write(&p, &data).unwrap();
        assert_eq!(sha256_path(&p).await.unwrap(), sha256_bytes(&data));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn path_hash_matches_bytes() {
        let dir = std::env::temp_dir().join(format!("rc_hash_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.bin");
        let data = vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        std::fs::write(&p, &data).unwrap();
        assert_eq!(sha1_path(&p).await.unwrap(), sha1_bytes(&data));
        assert_eq!(md5_path(&p).await.unwrap(), md5_bytes(&data));
    }
}
