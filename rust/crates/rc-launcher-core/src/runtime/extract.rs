//! Pure-Rust extraction of FCL's `.tar.xz` JRE packages (task 6).
//!
//! FCL ships each JRE as XZ-compressed tar archives. We decode them with
//! `lzma-rs` (pure Rust — no C dependencies) and unpack with the `tar` crate.
//! The XZ payload is **streamed** (from memory or, for the common on-disk case,
//! straight from the archive file) into a temp tar file so the (potentially
//! large) uncompressed tar never has to live entirely in RAM, then the tar
//! entries are unpacked one by one with an explicit path-traversal guard.
//!
//! Task 25 (performance & memory): [`extract_tar_xz_file`] streams the
//! decompression directly from the archive on disk, so installing a multi-hundred
//! MiB JRE never pins that much RAM at once.

use std::io::{BufReader, BufWriter, Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};

use lzma_rs::xz_decompress;
use tar::Archive;

use crate::error::{RcError, RcResult};

/// Decompress a `.tar.xz` stream (any `Read`) into `dest`, returning the summed
/// size of all unpacked entries (bytes). The XZ payload is streamed through a
/// temp file so the uncompressed tar never lives entirely in RAM.
pub fn extract_tar_xz_reader<R: Read>(reader: R, dest: &Path) -> RcResult<u64> {
    std::fs::create_dir_all(dest).map_err(RcError::Io)?;

    // Stream the XZ payload into a temp tar file, one buffer at a time.
    let tmp = dest.join(format!(".rc-xz-{}.tar", std::process::id()));
    {
        let out = std::fs::File::create(&tmp).map_err(RcError::Io)?;
        let mut writer = BufWriter::new(out);
        let mut reader = BufReader::new(reader);
        xz_decompress(&mut reader, &mut writer).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            RcError::Other(format!("xz decompress failed: {e}"))
        })?;
        writer.flush().map_err(RcError::Io)?;
    }

    // Stream the tar entries out of the temp file into `dest`.
    let written = unpack_tar(&tmp, dest);
    let _ = std::fs::remove_file(&tmp);
    written
}

/// Decompress the `.tar.xz` bytes in `data` into `dest` (convenience wrapper
/// that streams the in-memory data through [`extract_tar_xz_reader`]).
pub fn extract_tar_xz(data: &[u8], dest: &Path) -> RcResult<u64> {
    extract_tar_xz_reader(Cursor::new(data), dest)
}

/// Decompress a `.tar.xz` **file on disk** into `dest`, streaming the
/// decompression so the archive is never fully loaded into RAM (task 25 — large
/// file streaming extract / memory optimisation). This is the path used when
/// provisioning a JRE from the FCL APK runtime assets.
pub fn extract_tar_xz_file(src: &Path, dest: &Path) -> RcResult<u64> {
    let file = std::fs::File::open(src).map_err(RcError::Io)?;
    extract_tar_xz_reader(BufReader::new(file), dest)
}

fn unpack_tar(tar_path: &Path, dest: &Path) -> RcResult<u64> {
    let file = std::fs::File::open(tar_path).map_err(RcError::Io)?;
    let mut archive = Archive::new(BufReader::new(file));
    let mut total: u64 = 0;
    for entry in archive.entries().map_err(RcError::Io)? {
        let mut entry = entry.map_err(RcError::Io)?;
        let rel = entry.path().map_err(RcError::Io)?.into_owned();
        // Explicit traversal guard (the `tar` crate also guards, this is defence
        // in depth and gives us a precise error message).
        let _target = safe_join(dest, &rel)?;
        entry.unpack_in(dest).map_err(RcError::Io)?;
        total += entry.header().size().map_err(RcError::Io)?;
    }
    Ok(total)
}

/// Join `rel` onto `dest`, rejecting anything that would escape `dest`.
fn safe_join(dest: &Path, rel: &Path) -> RcResult<PathBuf> {
    if rel.is_absolute() {
        return Err(RcError::Other(format!(
            "archive entry is an absolute path: {rel:?}"
        )));
    }
    let mut out = dest.to_path_buf();
    for comp in rel.components() {
        match comp {
            Component::Normal(c) => out.push(c),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(RcError::Other(format!(
                    "archive entry escapes root: {rel:?}"
                )))
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(RcError::Other(format!(
                    "archive entry has a root/prefix: {rel:?}"
                )))
            }
        }
    }
    Ok(out)
}

/// Ensure the Java launcher is executable (Android needs the bit set
/// explicitly because the archive's mode may be masked on extraction).
pub fn ensure_java_executable(home: &Path) -> RcResult<()> {
    let java = home.join("bin").join("java");
    if !java.exists() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&java).map_err(RcError::Io)?.permissions();
        perms.set_mode(perms.mode() | 0o111);
        std::fs::set_permissions(&java, perms).map_err(RcError::Io)?;
    }
    #[cfg(not(unix))]
    {
        let _ = &java;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn safe_join_rejects_traversal() {
        let root = Path::new("/tmp/rc");
        assert!(safe_join(root, Path::new("../etc/passwd")).is_err());
        assert!(safe_join(root, Path::new("/abs")).is_err());
        let ok = safe_join(root, Path::new("bin/java")).unwrap();
        assert_eq!(ok, Path::new("/tmp/rc/bin/java"));
    }

    /// Build a tiny `.tar.xz` in memory and verify both the in-memory and the
    /// from-disk (streaming) extractors unpack it identically.
    fn build_tar_xz() -> Vec<u8> {
        let mut tar_buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_buf);
            let data = b"hello world";
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "a.txt", &data[..])
                .unwrap();
            builder.finish().unwrap();
        }
        let mut xz = Vec::new();
        {
            // Scope the writer so it (and its borrow of `xz`) is dropped before
            // `xz` is moved out of the function.
            let mut w = BufWriter::new(&mut xz);
            lzma_rs::xz_compress(&mut tar_buf.as_slice(), &mut w).unwrap();
            w.flush().unwrap();
        }
        xz
    }

    #[test]
    fn extract_tar_xz_file_streams_from_disk() {
        let xz = build_tar_xz();
        let src = std::env::temp_dir().join(format!("rc_extract_{}.tar.xz", std::process::id()));
        {
            let mut f = std::fs::File::create(&src).unwrap();
            f.write_all(&xz).unwrap();
        }
        let dest = std::env::temp_dir().join(format!("rc_extract_out_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dest);
        let total = extract_tar_xz_file(&src, &dest).unwrap();
        assert_eq!(total, 11);
        let got = std::fs::read(dest.join("a.txt")).unwrap();
        assert_eq!(got, b"hello world");
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn extract_tar_xz_reader_matches_file() {
        let xz = build_tar_xz();
        let dest = std::env::temp_dir().join(format!("rc_extract_mem_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dest);
        let total = extract_tar_xz(&xz, &dest).unwrap();
        assert_eq!(total, 11);
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"hello world");
        let _ = std::fs::remove_dir_all(&dest);
    }
}
