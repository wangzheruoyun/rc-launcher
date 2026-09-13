//! In-app small file manager (task 19).
//!
//! A safe, allocation-bounded wrapper around the filesystem operations the
//! launcher UI needs to expose a **small** file manager (browse / copy / move /
//! rename / delete / extract-archive / import). It is deliberately *not* a
//! full file manager; the goal is to support the typical player actions on
//! the well-known Minecraft directories (the per-instance `saves/`,
//! `mods/`, `resourcepacks/`, `shaderpacks/` and the game root) without
//! exposing the rest of the device's filesystem.
//!
//! ## Allowed-roots model
//!
//! Every entry point takes a JSON request that carries a list of *allowed
//! roots* (absolute paths) plus the target path. Both the parent (e.g. the
//! directory being listed) and the target (e.g. the destination of a copy /
//! move) are normalised via `canonicalize` (or, for paths that do not yet
//! exist, by joining and walking the parents) and then asserted to live under
//! **one** of the allowed roots. This is the same defence-in-depth pattern
//! FCL uses in its `FileFinder` (a path is rejected if it falls outside the
//! configured game dir / instance dir) and prevents the classic `../../etc/
//! passwd` path-traversal mistake even when the UI hands us a `..` segment.
//!
//! ## Two-step confirmation
//!
//! Destructive operations (`delete`, `move` over an existing path) require an
//! explicit `confirm: true` in the request JSON. The first call without the
//! flag returns a typed **preview** (the would-be list of victims + total
//! size) so the UI can show a confirmation dialog with the exact outcome.
//! This is the same pattern as `rm -i` and matches FCL's "second confirm
//! before deleting a save world" UX.
//!
//! ## Two-ABI scope
//!
//! All paths are ordinary `PathBuf`; the JNI layer is responsible for handing
//! us an absolute Android path (typically
//! `/data/data/com.rc.launcher/files/...` or the per-instance isolated dir
//! from `effectiveGameDir`). The core does **not** know about Android
//! `content://` URIs — those are resolved to bytes on the Kotlin side and
//! imported through [`import_bytes`].
//!
//! ## Errors
//!
//! Every fallible function returns [`RcResult`]. Path-traversal attempts,
//! escaping an allowed root, missing confirmations, and a missing source
//! file are all surfaced as `RcError::Other` with a precise message; the
//! unified severity (Recoverable for "user mistake" / Fatal for actual IO
//! failures) drives the UI's banner choice.
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::error::{RcError, RcResult};

/// A file or directory entry in a listing.
///
/// Serialised to JSON for the Kotlin side; field names are snake_case to
/// match the rest of the FFI surface. `kind` is one of `file`, `dir`, `link`
/// (symlink), or `other` (everything else, e.g. socket / fifo).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FsEntry {
    /// Entry name without any parent components (e.g. `mods`).
    pub name: String,
    /// `file`, `dir`, `link`, or `other`.
    pub kind: String,
    /// Size in bytes; `0` for non-files. Always reported, even for very
    /// large entries, so the UI can sum without an extra round trip.
    pub size: u64,
    /// Last-modified epoch millis, or `0` when unavailable.
    pub mtime_ms: i64,
    /// Absolute (normalised) path, useful as the `target` of follow-up
    /// operations.
    pub path: String,
    /// Whether the entry is hidden (leading `.`). UI may use this to grey
    /// the row out.
    pub hidden: bool,
}

/// A response from a directory listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsListing {
    /// The directory that was actually listed (canonicalised absolute path).
    pub path: String,
    /// Parent directory, or empty string when at a root.
    pub parent: String,
    /// Sorted (`dir` first, then files, both alphabetical) entries.
    pub entries: Vec<FsEntry>,
    /// Convenience: count of directories in [entries].
    pub dir_count: usize,
    /// Convenience: count of files in [entries].
    pub file_count: usize,
}

/// Preview of a destructive operation: what *would* be removed / overwritten
/// if the user confirmed. Returned when the request JSON carries
/// `confirm: false` (or omits the flag) for [`delete_paths`] or [`move_path`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsOpPreview {
    /// Operation id, e.g. `"delete"` or `"move"`.
    pub op: String,
    /// The targets that would be removed / overwritten.
    pub targets: Vec<FsEntry>,
    /// Sum of [`FsEntry::size`] of every `file` target. `0` for "no files".
    pub total_bytes: u64,
    /// Whether at least one target is a directory (so a recursive delete
    /// would wipe more than the user might expect).
    pub has_directories: bool,
}

/// Result of a successful operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsOpResult {
    /// Operation id (`"copy"`, `"move"`, `"delete"`, `"rename"`,
    /// `"extract"`, `"import"`).
    pub op: String,
    /// Absolute path of the resulting file / directory.
    pub path: String,
    /// Number of files touched (recursive: counts every file; single-file:
    /// 1 when successful, 0 otherwise).
    pub files_touched: u64,
    /// Total bytes written.
    pub bytes_written: u64,
}

// ---------------------------------------------------------------------------
// Allowed-roots model
// ---------------------------------------------------------------------------

/// Returned by [`resolve_under_roots`]. Encodes the *normalised* absolute
/// path of the input plus the matched root (handy for diagnostics).
#[derive(Debug, Clone)]
struct ResolvedPath {
    absolute: PathBuf,
    root: PathBuf,
}

/// Normalise `path` and assert it is contained in **one** of `roots`.
///
/// Symlinks are followed with `canonicalize` when the target exists; for
/// non-existent paths (e.g. the *destination* of a copy that has not been
/// created yet), the parent directory is canonicalised and the leaf is
/// re-attached so a `..` segment in the leaf still gets caught.
fn resolve_under_roots(path: &str, roots: &[String]) -> RcResult<ResolvedPath> {
    if roots.is_empty() {
        return Err(RcError::Other(
            "no allowed roots configured for the file manager".into(),
        ));
    }
    let raw = PathBuf::from(path);
    if raw.as_os_str().is_empty() {
        return Err(RcError::Other("empty path".into()));
    }
    let absolute = if raw.is_absolute() {
        raw
    } else {
        std::env::current_dir().map_err(RcError::Io)?.join(raw)
    };
    let normalised = normalise_existing(&absolute)?;

    for root in roots {
        let r = PathBuf::from(root);
        let r_norm = normalise_existing(&r)?;
        if normalised.starts_with(&r_norm) {
            return Ok(ResolvedPath {
                absolute: normalised,
                root: r_norm,
            });
        }
    }
    Err(RcError::Other(format!(
        "path escapes every allowed root: {path}"
    )))
}

/// Normalise an existing or not-yet-existing path: canonicalise the longest
/// existing prefix and re-attach the missing tail verbatim.
fn normalise_existing(path: &Path) -> RcResult<PathBuf> {
    if path.exists() {
        return fs::canonicalize(path).map_err(RcError::Io);
    }
    // Walk up until we find a parent that exists, canonicalise it, then
    // re-attach the suffix.
    let mut existing: PathBuf = path.to_path_buf();
    let mut suffix: Vec<PathBuf> = Vec::new();
    loop {
        if existing.exists() {
            let canon = fs::canonicalize(&existing).map_err(RcError::Io)?;
            let mut out = canon;
            for part in suffix.into_iter().rev() {
                out.push(part);
            }
            return Ok(out);
        }
        let Some(name) = existing.file_name() else {
            return Err(RcError::Other(format!(
                "path has no existing parent: {path:?}"
            )));
        };
        suffix.push(PathBuf::from(name));
        existing.pop();
    }
}

/// Reject `..` / absolute / drive-prefix components in a *user-supplied*
/// leaf (e.g. a new name typed into a "rename" dialog). Returns the joined
/// path on success.
fn safe_join(root: &Path, leaf: &str) -> RcResult<PathBuf> {
    if leaf.is_empty() {
        return Err(RcError::Other("empty name".into()));
    }
    let p = Path::new(leaf);
    if p.is_absolute() {
        return Err(RcError::Other(format!("name is an absolute path: {leaf}")));
    }
    for c in p.components() {
        match c {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => {
                return Err(RcError::Other(format!(
                    "name escapes root via '..': {leaf}"
                )))
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(RcError::Other(format!(
                    "name has a root / drive prefix: {leaf}"
                )))
            }
        }
    }
    Ok(root.join(p))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// List a directory. `request` is `{ "path": String, "roots": [String] }`.
pub fn list_dir(request: serde_json::Value) -> RcResult<FsListing> {
    #[derive(Deserialize)]
    struct Req {
        path: String,
        roots: Vec<String>,
    }
    let req: Req = serde_json::from_value(request)?;
    let resolved = resolve_under_roots(&req.path, &req.roots)?;
    let meta = fs::metadata(&resolved.absolute).map_err(RcError::Io)?;
    if !meta.is_dir() {
        return Err(RcError::Other(format!(
            "not a directory: {}",
            resolved.absolute.display()
        )));
    }
    let read = fs::read_dir(&resolved.absolute).map_err(RcError::Io)?;
    let mut entries: Vec<FsEntry> = Vec::new();
    for ent in read {
        let ent = match ent {
            Ok(e) => e,
            Err(e) => {
                // Skip unreadable entries rather than aborting the listing.
                eprintln!("[fs_ops] read_dir entry failed: {e}");
                continue;
            }
        };
        let name = ent.file_name().to_string_lossy().into_owned();
        let path = ent.path();
        let md = match ent.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let kind = if md.file_type().is_symlink() {
            "link"
        } else if md.is_dir() {
            "dir"
        } else if md.is_file() {
            "file"
        } else {
            "other"
        };
        let mtime_ms = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        entries.push(FsEntry {
            hidden: name.starts_with('.'),
            name,
            kind: kind.into(),
            size: md.len(),
            mtime_ms,
            path: path.to_string_lossy().into_owned(),
        });
    }
    // Dirs first, then files, both case-insensitive alphabetical.
    entries.sort_by(|a, b| match (a.kind.as_str(), b.kind.as_str()) {
        ("dir", "file") => std::cmp::Ordering::Less,
        ("file", "dir") => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    let dir_count = entries.iter().filter(|e| e.kind == "dir").count();
    let file_count = entries.iter().filter(|e| e.kind == "file").count();
    let parent = resolved
        .absolute
        .parent()
        .map(|p| {
            if p == resolved.root {
                String::new()
            } else {
                p.to_string_lossy().into_owned()
            }
        })
        .unwrap_or_default();
    Ok(FsListing {
        path: resolved.absolute.to_string_lossy().into_owned(),
        parent,
        entries,
        dir_count,
        file_count,
    })
}

/// Make a directory under an allowed root. `request` is
/// `{ "parent": String, "name": String, "roots": [String] }`.
pub fn mkdir(request: serde_json::Value) -> RcResult<FsOpResult> {
    #[derive(Deserialize)]
    struct Req {
        parent: String,
        name: String,
        roots: Vec<String>,
    }
    let req: Req = serde_json::from_value(request)?;
    let parent = resolve_under_roots(&req.parent, &req.roots)?;
    let target = safe_join(&parent.absolute, &req.name)?;
    if target.exists() {
        return Err(RcError::Other(format!(
            "already exists: {}",
            target.display()
        )));
    }
    fs::create_dir_all(&target).map_err(RcError::Io)?;
    Ok(FsOpResult {
        op: "mkdir".into(),
        path: target.to_string_lossy().into_owned(),
        files_touched: 0,
        bytes_written: 0,
    })
}

/// Copy a file or directory tree. `request` is
/// `{ "source": String, "destination": String, "roots": [String] }`.
///
/// * `source` must exist.
/// * `destination` may *not* exist (the new path).
/// * When `source` is a directory, the whole tree is copied recursively.
pub fn copy_path(request: serde_json::Value) -> RcResult<FsOpResult> {
    #[derive(Deserialize)]
    struct Req {
        source: String,
        destination: String,
        roots: Vec<String>,
    }
    let req: Req = serde_json::from_value(request)?;
    let src = resolve_under_roots(&req.source, &req.roots)?;
    let dst = resolve_under_roots(&req.destination, &req.roots)?;
    if !src.absolute.exists() {
        return Err(RcError::MissingFile(
            src.absolute.to_string_lossy().into_owned(),
        ));
    }
    if dst.absolute.exists() {
        return Err(RcError::Other(format!(
            "destination already exists: {}",
            dst.absolute.display()
        )));
    }
    let (files, bytes) = copy_recursive(&src.absolute, &dst.absolute)?;
    Ok(FsOpResult {
        op: "copy".into(),
        path: dst.absolute.to_string_lossy().into_owned(),
        files_touched: files,
        bytes_written: bytes,
    })
}

fn copy_recursive(src: &Path, dst: &Path) -> RcResult<(u64, u64)> {
    let md = fs::metadata(src).map_err(RcError::Io)?;
    if md.is_dir() {
        fs::create_dir_all(dst).map_err(RcError::Io)?;
        let mut files = 0u64;
        let mut bytes = 0u64;
        for entry in fs::read_dir(src).map_err(RcError::Io)? {
            let entry = entry.map_err(RcError::Io)?;
            let (f, b) = copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
            files += f;
            bytes += b;
        }
        Ok((files, bytes))
    } else {
        let parent = dst
            .parent()
            .ok_or_else(|| RcError::Other(format!("destination has no parent: {dst:?}")))?;
        fs::create_dir_all(parent).map_err(RcError::Io)?;
        let written = copy_file_streaming(src, dst)?;
        Ok((1, written))
    }
}

fn copy_file_streaming(src: &Path, dst: &Path) -> RcResult<u64> {
    let mut r = fs::File::open(src).map_err(RcError::Io)?;
    let mut w = fs::File::create(dst).map_err(RcError::Io)?;
    let mut buf = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = r.read(&mut buf).map_err(RcError::Io)?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n]).map_err(RcError::Io)?;
        total += n as u64;
    }
    w.flush().map_err(RcError::Io)?;
    Ok(total)
}

/// Move (rename) a file or directory. Without `confirm: true` and with an
/// existing destination, returns an [`FsOpPreview`] so the UI can show
/// "this will overwrite X" before applying.
pub fn move_path(request: serde_json::Value) -> RcResult<serde_json::Value> {
    #[derive(Deserialize)]
    struct Req {
        source: String,
        destination: String,
        roots: Vec<String>,
        #[serde(default)]
        confirm: bool,
    }
    let req: Req = serde_json::from_value(request)?;
    let src = resolve_under_roots(&req.source, &req.roots)?;
    let dst = resolve_under_roots(&req.destination, &req.roots)?;
    if !src.absolute.exists() {
        return Err(RcError::MissingFile(
            src.absolute.to_string_lossy().into_owned(),
        ));
    }
    if dst.absolute.exists() && !req.confirm {
        let preview = preview_overwrite(&dst.absolute, "move")?;
        return Ok(serde_json::to_value(preview)?);
    }
    let bytes = dir_size(&src.absolute).unwrap_or(0);
    let files = count_files(&src.absolute);
    fs::rename(&src.absolute, &dst.absolute).map_err(RcError::Io)?;
    Ok(serde_json::to_value(FsOpResult {
        op: "move".into(),
        path: dst.absolute.to_string_lossy().into_owned(),
        files_touched: files,
        bytes_written: bytes,
    })?)
}

/// Rename a single entry inside its parent directory. The new name is
/// checked for `..` / absolute components and may not collide with an
/// existing entry.
pub fn rename_path(request: serde_json::Value) -> RcResult<FsOpResult> {
    #[derive(Deserialize)]
    struct Req {
        path: String,
        new_name: String,
        roots: Vec<String>,
    }
    let req: Req = serde_json::from_value(request)?;
    let src = resolve_under_roots(&req.path, &req.roots)?;
    let parent = src
        .absolute
        .parent()
        .ok_or_else(|| RcError::Other("path has no parent".into()))?
        .to_path_buf();
    let target = safe_join(&parent, &req.new_name)?;
    if target.exists() {
        return Err(RcError::Other(format!(
            "name already taken: {}",
            target.display()
        )));
    }
    let bytes = dir_size(&src.absolute).unwrap_or(0);
    let files = count_files(&src.absolute);
    fs::rename(&src.absolute, &target).map_err(RcError::Io)?;
    Ok(FsOpResult {
        op: "rename".into(),
        path: target.to_string_lossy().into_owned(),
        files_touched: files,
        bytes_written: bytes,
    })
}

/// Delete one or more paths. Without `confirm: true`, returns an
/// [`FsOpPreview`] so the UI can render a confirmation dialog; with the
/// flag, actually performs the recursive delete.
///
/// `request` is `{ "paths": [String], "roots": [String], "confirm": Bool }`.
pub fn delete_paths(request: serde_json::Value) -> RcResult<serde_json::Value> {
    #[derive(Deserialize)]
    struct Req {
        paths: Vec<String>,
        roots: Vec<String>,
        #[serde(default)]
        confirm: bool,
    }
    let req: Req = serde_json::from_value(request)?;
    if req.paths.is_empty() {
        return Err(RcError::Other("no paths to delete".into()));
    }
    let mut resolved = Vec::with_capacity(req.paths.len());
    for p in &req.paths {
        resolved.push(resolve_under_roots(p, &req.roots)?);
    }
    if !req.confirm {
        let mut targets = Vec::with_capacity(resolved.len());
        let mut total_bytes: u64 = 0;
        let mut has_directories = false;
        for r in &resolved {
            if let Some(e) = entry_summary(&r.absolute) {
                total_bytes += e.size;
                if e.kind == "dir" {
                    has_directories = true;
                }
                targets.push(e);
            }
        }
        let preview = FsOpPreview {
            op: "delete".into(),
            targets,
            total_bytes,
            has_directories,
        };
        return Ok(serde_json::to_value(preview)?);
    }
    let mut deleted_files = 0u64;
    let mut deleted_bytes = 0u64;
    for r in &resolved {
        let f = count_files(&r.absolute);
        let b = dir_size(&r.absolute).unwrap_or(0);
        if r.absolute.is_dir() {
            fs::remove_dir_all(&r.absolute).map_err(RcError::Io)?;
        } else {
            fs::remove_file(&r.absolute).map_err(RcError::Io)?;
        }
        deleted_files += f;
        deleted_bytes += b;
    }
    Ok(serde_json::to_value(FsOpResult {
        op: "delete".into(),
        path: resolved
            .last()
            .map(|r| r.absolute.to_string_lossy().into_owned())
            .unwrap_or_default(),
        files_touched: deleted_files,
        bytes_written: deleted_bytes,
    })?)
}

/// Extract a `.zip` archive into `destination`. Only `.zip` is supported
/// because that is what FCL's mod / resource-pack / shader import
/// pipelines emit; `.tar.gz` / `.tar.xz` extraction lives in the
/// `runtime::extract` module and is exposed by the modpack import.
///
/// `request` is `{ "archive": String, "destination": String, "roots":
/// [String] }`. Returns the number of files extracted + bytes written.
pub fn extract_zip(request: serde_json::Value) -> RcResult<FsOpResult> {
    #[derive(Deserialize)]
    struct Req {
        archive: String,
        destination: String,
        roots: Vec<String>,
    }
    let req: Req = serde_json::from_value(request)?;
    let arc = resolve_under_roots(&req.archive, &req.roots)?;
    let dst = resolve_under_roots(&req.destination, &req.roots)?;
    if !arc.absolute.is_file() {
        return Err(RcError::MissingFile(
            arc.absolute.to_string_lossy().into_owned(),
        ));
    }
    let file = fs::File::open(&arc.absolute).map_err(RcError::Io)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| RcError::Other(format!("not a valid zip archive: {e}")))?;
    fs::create_dir_all(&dst.absolute).map_err(RcError::Io)?;
    let mut files = 0u64;
    let mut bytes = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| RcError::Other(format!("zip entry {i}: {e}")))?;
        // Reject absolute / traversal paths inside the archive.
        let rel = match entry.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => {
                return Err(RcError::Other(format!(
                    "archive entry escapes destination: {}",
                    entry.name()
                )))
            }
        };
        let target = dst.absolute.join(&rel);
        if target.starts_with(&dst.absolute) == false {
            return Err(RcError::Other(format!(
                "archive entry escapes destination: {}",
                rel.display()
            )));
        }
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(RcError::Io)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(RcError::Io)?;
        }
        let mut out = fs::File::create(&target).map_err(RcError::Io)?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = match entry.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => return Err(RcError::Io(e)),
            };
            out.write_all(&buf[..n]).map_err(RcError::Io)?;
            bytes += n as u64;
        }
        out.flush().map_err(RcError::Io)?;
        files += 1;
    }
    Ok(FsOpResult {
        op: "extract".into(),
        path: dst.absolute.to_string_lossy().into_owned(),
        files_touched: files,
        bytes_written: bytes,
    })
}

/// Import an opaque byte blob (typically decoded from a `content://` URI by
/// the Android side) into `destination` (must end in the desired file
/// name). The destination is rejected when it escapes an allowed root, and
/// the parent directory is created on demand.
///
/// `request` is base64-encoded JSON `{ "destination": String, "data_b64":
/// String, "roots": [String] }`.
pub fn import_bytes(request: serde_json::Value) -> RcResult<FsOpResult> {
    #[derive(Deserialize)]
    struct Req {
        destination: String,
        data_b64: String,
        roots: Vec<String>,
    }
    let req: Req = serde_json::from_value(request)?;
    let dst = resolve_under_roots(&req.destination, &req.roots)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(req.data_b64.as_bytes())
        .map_err(|e| RcError::Other(format!("invalid base64: {e}")))?;
    if let Some(parent) = dst.absolute.parent() {
        fs::create_dir_all(parent).map_err(RcError::Io)?;
    }
    let mut out = fs::File::create(&dst.absolute).map_err(RcError::Io)?;
    out.write_all(&bytes).map_err(RcError::Io)?;
    out.flush().map_err(RcError::Io)?;
    Ok(FsOpResult {
        op: "import".into(),
        path: dst.absolute.to_string_lossy().into_owned(),
        files_touched: 1,
        bytes_written: bytes.len() as u64,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Cheap directory-size estimate (`metadata.len()` summed recursively).
fn dir_size(path: &Path) -> RcResult<u64> {
    let md = fs::metadata(path).map_err(RcError::Io)?;
    if !md.is_dir() {
        return Ok(md.len());
    }
    let mut total = 0u64;
    for entry in fs::read_dir(path).map_err(RcError::Io)? {
        let entry = entry.map_err(RcError::Io)?;
        total += dir_size(&entry.path())?;
    }
    Ok(total)
}

fn count_files(path: &Path) -> u64 {
    let md = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return 0,
    };
    if !md.is_dir() {
        return 1;
    }
    let mut count = 0u64;
    if let Ok(read) = fs::read_dir(path) {
        for entry in read.flatten() {
            count += count_files(&entry.path());
        }
    }
    count
}

fn entry_summary(path: &Path) -> Option<FsEntry> {
    let md = fs::metadata(path).ok()?;
    let name = path.file_name()?.to_string_lossy().into_owned();
    let kind = if md.file_type().is_symlink() {
        "link"
    } else if md.is_dir() {
        "dir"
    } else if md.is_file() {
        "file"
    } else {
        "other"
    };
    let mtime_ms = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    Some(FsEntry {
        hidden: name.starts_with('.'),
        name,
        kind: kind.into(),
        size: md.len(),
        mtime_ms,
        path: path.to_string_lossy().into_owned(),
    })
}

fn preview_overwrite(path: &Path, op: &str) -> RcResult<FsOpPreview> {
    let entry = entry_summary(path)
        .ok_or_else(|| RcError::Other(format!("cannot preview non-existent path: {path:?}")))?;
    let total_bytes = dir_size(path).unwrap_or(0);
    let has_directories = entry.kind == "dir";
    Ok(FsOpPreview {
        op: op.into(),
        targets: vec![entry],
        total_bytes,
        has_directories,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn touch(p: &Path) {
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(p).unwrap();
        f.write_all(b"x").unwrap();
    }

    #[test]
    fn safe_join_rejects_dotdot() {
        let root = Path::new("/allowed");
        assert!(safe_join(root, "..").is_err());
        assert!(safe_join(root, "../etc").is_err());
        assert!(safe_join(root, "a/../b").is_err());
        assert!(safe_join(root, "/abs").is_err());
        assert!(safe_join(root, "").is_err());
        assert!(safe_join(root, "ok").is_ok());
    }

    #[test]
    fn list_dir_sorts_dirs_first() {
        let dir = tempdir().unwrap();
        touch(&dir.path().join("a.txt"));
        fs::create_dir(dir.path().join("zzz")).unwrap();
        touch(&dir.path().join("b.txt"));
        let req = serde_json::json!({
            "path": dir.path().to_string_lossy(),
            "roots": [dir.path().to_string_lossy()],
        });
        let listing = list_dir(req).unwrap();
        assert_eq!(listing.entries.len(), 3);
        assert_eq!(listing.entries[0].name, "zzz");
        assert_eq!(listing.entries[0].kind, "dir");
        assert_eq!(listing.entries[1].name, "a.txt");
        assert_eq!(listing.entries[2].name, "b.txt");
        assert_eq!(listing.dir_count, 1);
        assert_eq!(listing.file_count, 2);
    }

    #[test]
    fn rejects_path_outside_root() {
        let allowed = tempdir().unwrap();
        let other = tempdir().unwrap();
        touch(&other.path().join("a.txt"));
        let req = serde_json::json!({
            "path": other.path().to_string_lossy(),
            "roots": [allowed.path().to_string_lossy()],
        });
        let err = list_dir(req).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("escapes"), "unexpected error: {msg}");
    }

    #[test]
    fn delete_requires_confirm() {
        let dir = tempdir().unwrap();
        let f = dir.path().join("a.txt");
        touch(&f);
        // No confirm -> preview.
        let req = serde_json::json!({
            "paths": [f.to_string_lossy()],
            "roots": [dir.path().to_string_lossy()],
        });
        let preview = delete_paths(req.clone()).unwrap();
        assert_eq!(preview["op"], "delete");
        assert_eq!(preview["targets"].as_array().unwrap().len(), 1);
        assert!(f.exists(), "file must still exist after preview");
        // With confirm -> actually deletes.
        let mut with_confirm = req.as_object().unwrap().clone();
        with_confirm.insert("confirm".into(), serde_json::json!(true));
        let r = delete_paths(serde_json::Value::Object(with_confirm)).unwrap();
        assert_eq!(r["op"], "delete");
        assert!(!f.exists(), "file must be gone after confirm");
    }

    #[test]
    fn mkdir_and_rename() {
        let dir = tempdir().unwrap();
        let req = serde_json::json!({
            "parent": dir.path().to_string_lossy(),
            "name": "new",
            "roots": [dir.path().to_string_lossy()],
        });
        mkdir(req).unwrap();
        assert!(dir.path().join("new").is_dir());

        let req = serde_json::json!({
            "path": dir.path().join("new").to_string_lossy(),
            "new_name": "renamed",
            "roots": [dir.path().to_string_lossy()],
        });
        rename_path(req).unwrap();
        assert!(!dir.path().join("new").exists());
        assert!(dir.path().join("renamed").is_dir());
    }

    #[test]
    fn copy_recursive_counts_bytes() {
        let src_dir = tempdir().unwrap();
        let dst_dir = tempdir().unwrap();
        touch(&src_dir.path().join("a.txt"));
        touch(&src_dir.path().join("sub/b.txt"));
        let req = serde_json::json!({
            "source": src_dir.path().to_string_lossy(),
            "destination": dst_dir.path().join("copy").to_string_lossy(),
            "roots": [src_dir.path().to_string_lossy(), dst_dir.path().to_string_lossy()],
        });
        let r = copy_path(req).unwrap();
        assert_eq!(r.op, "copy");
        assert_eq!(r.files_touched, 2);
        assert_eq!(r.bytes_written, 2);
        assert!(dst_dir.path().join("copy/a.txt").is_file());
        assert!(dst_dir.path().join("copy/sub/b.txt").is_file());
    }

    #[test]
    fn move_with_overwrite_requires_confirm() {
        let dir = tempdir().unwrap();
        touch(&dir.path().join("src.txt"));
        touch(&dir.path().join("dst.txt"));
        // First call: no confirm, dst already exists -> preview.
        let req = serde_json::json!({
            "source": dir.path().join("src.txt").to_string_lossy(),
            "destination": dir.path().join("dst.txt").to_string_lossy(),
            "roots": [dir.path().to_string_lossy()],
        });
        let preview = move_path(req.clone()).unwrap();
        assert_eq!(preview["op"], "move");
        assert_eq!(preview["targets"].as_array().unwrap().len(), 1);
        assert!(dir.path().join("src.txt").exists());
        // Confirm: this is a same-parent rename, which overwrites atomically
        // on POSIX; the underlying fs::rename replaces the target.
        let mut with_confirm = req.as_object().unwrap().clone();
        with_confirm.insert("confirm".into(), serde_json::json!(true));
        let r = move_path(serde_json::Value::Object(with_confirm)).unwrap();
        assert_eq!(r["op"], "move");
        assert!(!dir.path().join("src.txt").exists());
        assert!(dir.path().join("dst.txt").exists());
    }

    #[test]
    fn import_bytes_writes_file() {
        let dir = tempdir().unwrap();
        let req = serde_json::json!({
            "destination": dir.path().join("hello.txt").to_string_lossy(),
            "data_b64": "aGVsbG8=",
            "roots": [dir.path().to_string_lossy()],
        });
        let r = import_bytes(req).unwrap();
        assert_eq!(r.op, "import");
        assert_eq!(r.bytes_written, 5);
        assert_eq!(fs::read(dir.path().join("hello.txt")).unwrap(), b"hello");
    }
}
