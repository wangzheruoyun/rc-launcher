//! **Dynamic language packs** — whole new languages loaded at runtime (task 20).
//!
//! ## Why
//!
//! [`Language`] is a compiled-in enum, so shipping 日本語 or Русский used to mean
//! editing five files and cutting a new APK (see `docs/i18n.md` §6). That is fine
//! for the three first-party catalogues but hopeless for community translations —
//! and doubly so for a launcher whose users are largely in mainland China, where
//! sideloading a rebuilt APK to get your language is a real barrier.
//!
//! A *pack* is just a `.properties` file dropped into the app's `i18n/` data
//! directory. Once loaded it is a **first-class language**: it appears in the
//! settings picker, it can be negotiated from the device locale, it has its own
//! plural rule and endonym, and the whole `t*` / `bundle` / value-formatting
//! machinery resolves through it.
//!
//! ## Relationship to the overlay
//!
//! They are deliberately different tools:
//!
//! | | [`super::catalog`] overlay | pack (this module) |
//! |---|---|---|
//! | purpose | *re-word* a language we ship | *add* a language we do not ship |
//! | addressed by | [`Language`] variant | BCP-47 tag string |
//! | appears in the picker | no | **yes** |
//! | tag may collide with a built-in | that is the point | rejected (use the overlay) |
//!
//! ## File format
//!
//! An ordinary catalogue (see [`super::catalog`]) plus optional `_meta.*` keys:
//!
//! ```properties
//! _meta.tag          = ja          # default: the file stem
//! _meta.native_name  = 日本語       # endonym shown in the picker
//! _meta.english_name = Japanese    # for logs / bug reports
//! _meta.plural       = other_only  # other_only (default) | one_other
//! _meta.rtl          = false
//! _meta.parent       = zh-CN       # where unresolved keys go next
//!
//! nav.home = ホーム
//! ```
//!
//! `_meta.*` keys are stripped from the message table, so they can never leak
//! into the UI or into [`super::all_keys`].
//!
//! ## Robustness (task 19)
//!
//! Loading arbitrary user files is an attack surface, so every failure mode is a
//! *skip with a recorded reason*, never a panic and never a partial install:
//!
//! * a file bigger than [`MAX_PACK_BYTES`] is skipped (no OOM from a huge or
//!   pathological file);
//! * at most [`MAX_PACKS`] packs are registered (a directory with 10 000 files
//!   cannot exhaust memory or make the picker unusable);
//! * an unparseable / missing `_meta.tag` **and** unparseable file stem is skipped,
//!   as are the BCP-47 non-languages `und` / `mul` / `zxx`;
//! * a tag that collides with a compiled-in language is skipped (that is what the
//!   overlay is for) — otherwise a stale `en.properties` could shadow English in a
//!   way the user cannot turn off from the picker;
//! * a pack with zero messages is skipped, so the picker never offers a language
//!   that renders as 100 % Chinese;
//! * `_meta.parent` naming an unknown language falls back to [`Language::BASE`],
//!   so the fallback chain always terminates in Chinese;
//! * unloading the *active* pack re-activates its parent, so the UI can never be
//!   left pointing at a language that no longer exists.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock, RwLockWriteGuard};

use super::catalog::Catalog;
use super::format::PluralRule;
use super::language::{Language, LanguageTag};

/// Largest pack file we will read, in bytes (~1 MiB — the shipped catalogues are
/// ~7 KiB, so this is 100× headroom while still bounding a hostile file).
pub const MAX_PACK_BYTES: u64 = 1024 * 1024;

/// Most packs we keep registered at once.
pub const MAX_PACKS: usize = 64;

/// The `_meta.` prefix that marks a pack's own metadata (never a UI message).
pub const META_PREFIX: &str = "_meta.";

/// A language loaded at runtime from a `.properties` file.
#[derive(Debug, Clone)]
pub struct LanguagePack {
    tag: String,
    native_name: String,
    english_name: String,
    plural: PluralRule,
    rtl: bool,
    parent: Language,
    catalog: Catalog,
    source: Option<PathBuf>,
}

impl LanguagePack {
    /// The canonical BCP-47 tag (`ja`, `ru`, `pt-BR`).
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// Endonym shown in the picker (never translated).
    pub fn native_name(&self) -> &str {
        &self.native_name
    }

    /// English name, for logs and bug reports.
    pub fn english_name(&self) -> &str {
        &self.english_name
    }

    /// The pack's CLDR cardinal rule set.
    pub fn plural_rule(&self) -> PluralRule {
        self.plural
    }

    /// Right-to-left script?
    pub fn is_rtl(&self) -> bool {
        self.rtl
    }

    /// The compiled-in language unresolved keys fall through to.
    pub fn parent(&self) -> Language {
        self.parent
    }

    /// Number of messages the pack itself provides.
    pub fn len(&self) -> usize {
        self.catalog.len()
    }

    /// True when the pack provides no messages (never registered — see module docs).
    pub fn is_empty(&self) -> bool {
        self.catalog.is_empty()
    }

    /// Where the pack was loaded from, when it came from disk.
    pub fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }

    /// Look `key` up in **this pack only** (no fallback chain).
    pub fn get(&self, key: &str) -> Option<&str> {
        self.catalog.get(key)
    }

    /// Every key the pack provides, sorted and owned.
    ///
    /// Owned because the registry lives behind an `RwLock`: handing out borrowed
    /// keys would tie the lock lifetime to the caller.
    pub fn keys_owned(&self) -> Vec<String> {
        self.catalog
            .keys()
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    /// Parse problems the catalogue parser recorded (always non-fatal).
    pub fn problems(&self) -> &[String] {
        self.catalog.problems()
    }

    /// Build a pack from a `.properties` document.
    ///
    /// `fallback_tag` is used when the document carries no `_meta.tag` (the file
    /// stem, for a directory load). Returns the reason on rejection, so the
    /// caller can report it instead of failing silently.
    pub fn parse(text: &str, fallback_tag: Option<&str>) -> Result<LanguagePack, String> {
        let raw = Catalog::parse(text);

        // Split `_meta.*` out of the message table: metadata must never be
        // reachable as a UI key.
        let mut meta: BTreeMap<String, String> = BTreeMap::new();
        let mut messages = Catalog::default();
        for key in raw.keys() {
            let value = raw.get(key).unwrap_or_default();
            match key.strip_prefix(META_PREFIX) {
                Some(name) => {
                    meta.insert(name.to_ascii_lowercase(), value.to_string());
                }
                None => messages.insert(key.to_string(), value.to_string()),
            }
        }

        let declared = meta.get("tag").map(String::as_str);
        let tag_source = declared.filter(|t| !t.trim().is_empty()).or(fallback_tag);
        let Some(tag_source) = tag_source else {
            return Err("no `_meta.tag` and no usable file name".to_string());
        };
        let Some(parsed) = LanguageTag::parse(tag_source) else {
            return Err(format!("unusable language tag {tag_source:?}"));
        };
        // `und` (undetermined), `mul` (multiple) and `zxx` (no linguistic content)
        // parse as well-formed language subtags but do not name a language a user
        // could pick, so they must not become picker rows.
        if matches!(parsed.language.as_str(), "und" | "mul" | "zxx") {
            return Err(format!("{} does not name a real language", parsed.language));
        }
        let tag = parsed.to_string();

        // A pack may not shadow a compiled-in language: use the overlay for that.
        if Language::from_tag(&tag).is_some() || Language::negotiate(&tag).is_some() {
            return Err(format!(
                "{tag} is a built-in language; use the translation overlay to re-word it"
            ));
        }
        if messages.is_empty() {
            return Err(format!("{tag} contains no messages"));
        }

        let parent = meta
            .get("parent")
            .and_then(|p| Language::from_tag(p).or_else(|| Language::negotiate(p)))
            .unwrap_or(Language::BASE);
        let plural = meta
            .get("plural")
            .map(|p| PluralRule::parse(p))
            .unwrap_or_default();
        let rtl = meta
            .get("rtl")
            .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(false);
        // A pack with no endonym still has to be findable in the picker, so fall
        // back to the tag rather than to an empty row.
        let native_name = meta
            .get("native_name")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or(&tag)
            .to_string();
        let english_name = meta
            .get("english_name")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or(&native_name)
            .to_string();

        Ok(LanguagePack {
            tag,
            native_name,
            english_name,
            plural,
            rtl,
            parent,
            catalog: messages,
            source: None,
        })
    }

    /// How many of `required` keys this pack translates itself, as a 0..=1 ratio.
    pub fn completeness<'a, I: IntoIterator<Item = &'a str>>(&self, required: I) -> f32 {
        let mut total = 0usize;
        let mut have = 0usize;
        for key in required {
            total += 1;
            if self.catalog.get(key).is_some() {
                have += 1;
            }
        }
        if total == 0 {
            1.0
        } else {
            have as f32 / total as f32
        }
    }
}

// --- Registry ---------------------------------------------------------------

/// Registered packs, keyed by canonical tag. A `BTreeMap` so the picker order is
/// deterministic (and stable across reloads) without an extra sort.
static PACKS: OnceLock<RwLock<BTreeMap<String, LanguagePack>>> = OnceLock::new();

/// The tag of the pack the `t*` helpers resolve through, when one is active.
static ACTIVE: OnceLock<RwLock<Option<String>>> = OnceLock::new();

fn packs() -> &'static RwLock<BTreeMap<String, LanguagePack>> {
    PACKS.get_or_init(|| RwLock::new(BTreeMap::new()))
}

fn active_slot() -> &'static RwLock<Option<String>> {
    ACTIVE.get_or_init(|| RwLock::new(None))
}

/// Take the write lock, tolerating a poisoned mutex.
///
/// A panic in another thread must not permanently disable the language picker
/// (task-19: degrade, never dead-end).
fn packs_mut() -> RwLockWriteGuard<'static, BTreeMap<String, LanguagePack>> {
    packs().write().unwrap_or_else(|e| e.into_inner())
}

/// Install one pack, replacing any pack with the same tag. Returns the reason on
/// rejection.
pub fn install(mut pack: LanguagePack, source: Option<PathBuf>) -> Result<String, String> {
    pack.source = source;
    let tag = pack.tag.clone();
    let mut guard = packs_mut();
    if guard.len() >= MAX_PACKS && !guard.contains_key(&tag) {
        return Err(format!("too many language packs (limit {MAX_PACKS})"));
    }
    guard.insert(tag.clone(), pack);
    Ok(tag)
}

/// Parse and install a pack from a `.properties` document.
pub fn install_text(text: &str, fallback_tag: Option<&str>) -> Result<String, String> {
    install(LanguagePack::parse(text, fallback_tag)?, None)
}

/// The outcome of scanning a directory for packs.
#[derive(Debug, Default, Clone)]
pub struct LoadReport {
    /// Tags successfully registered.
    pub loaded: Vec<String>,
    /// `file: reason` for every file that was skipped.
    pub skipped: Vec<String>,
}

/// Load every `*.properties` in `dir` as a language pack.
///
/// A missing / unreadable directory is not an error: it loads nothing (offline
/// and first-run robustness). Files are visited in sorted order so precedence is
/// deterministic.
pub fn load_dir(dir: &Path) -> LoadReport {
    let mut report = LoadReport::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return report;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("properties"))
        })
        .collect();
    files.sort();

    for path in files {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("<unnamed>")
            .to_string();
        // Check the size *before* reading, so a hostile file is never buffered.
        match std::fs::metadata(&path) {
            Ok(meta) if meta.len() > MAX_PACK_BYTES => {
                report
                    .skipped
                    .push(format!("{name}: larger than {MAX_PACK_BYTES} bytes"));
                continue;
            }
            Ok(meta) if !meta.is_file() => {
                report.skipped.push(format!("{name}: not a regular file"));
                continue;
            }
            Err(e) => {
                report.skipped.push(format!("{name}: {e}"));
                continue;
            }
            _ => {}
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            report.skipped.push(format!("{name}: not valid UTF-8"));
            continue;
        };
        let stem = path.file_stem().and_then(|s| s.to_str());
        match LanguagePack::parse(&text, stem) {
            Ok(pack) => match install(pack, Some(path.clone())) {
                Ok(tag) => report.loaded.push(tag),
                Err(reason) => report.skipped.push(format!("{name}: {reason}")),
            },
            Err(reason) => report.skipped.push(format!("{name}: {reason}")),
        }
    }
    report
}

/// Every registered pack tag, sorted.
pub fn tags() -> Vec<String> {
    packs()
        .read()
        .map(|g| g.keys().cloned().collect())
        .unwrap_or_default()
}

/// How many packs are registered.
pub fn count() -> usize {
    packs().read().map(|g| g.len()).unwrap_or(0)
}

/// Run `f` against the pack registered under `tag`.
///
/// A closure rather than a returned reference because the registry lives behind
/// an `RwLock` and a pack can be removed at any time.
pub fn with<T>(tag: &str, f: impl FnOnce(&LanguagePack) -> T) -> Option<T> {
    let canonical = canonical_tag(tag)?;
    let guard = packs().read().ok()?;
    guard.get(&canonical).map(f)
}

/// True when `tag` names a registered pack.
pub fn contains(tag: &str) -> bool {
    with(tag, |_| ()).is_some()
}

/// Normalise `tag` the way [`LanguagePack::parse`] did when registering.
pub fn canonical_tag(tag: &str) -> Option<String> {
    LanguageTag::parse(tag).map(|t| t.to_string())
}

/// Remove one pack. Returns true when something was removed.
///
/// If the removed pack was [`active`], the active selection is dropped so the
/// caller's next lookup resolves through the built-in language instead of a
/// dangling tag.
pub fn remove(tag: &str) -> bool {
    let Some(canonical) = canonical_tag(tag) else {
        return false;
    };
    let removed = packs_mut().remove(&canonical).is_some();
    if removed && active().as_deref() == Some(canonical.as_str()) {
        set_active(None);
    }
    removed
}

/// Drop every pack (and any active selection).
pub fn clear() {
    packs_mut().clear();
    set_active(None);
}

/// The active pack tag, when a dynamically loaded language is selected.
pub fn active() -> Option<String> {
    active_slot()
        .read()
        .ok()
        .and_then(|g| g.clone())
        .filter(|t| contains(t))
}

/// Select (or, with `None`, deselect) the active pack.
///
/// Selecting an unregistered tag deselects instead, so the active slot can never
/// point at something that does not exist.
pub fn set_active(tag: Option<&str>) -> Option<String> {
    let resolved = tag.and_then(canonical_tag).filter(|t| contains(t));
    if let Ok(mut guard) = active_slot().write() {
        *guard = resolved.clone();
    }
    resolved
}

/// Negotiate `tag` against the registered packs.
///
/// Compiled-in languages are matched **first** by the caller
/// ([`super::set_language_from_preferences`]): a pack only ever serves a language
/// the launcher does not already ship, which is why [`LanguagePack::parse`]
/// rejects colliding tags.
pub fn negotiate(tag: &str) -> Option<String> {
    let wanted = LanguageTag::parse(tag)?;
    let guard = packs().read().ok()?;
    // Exact tag (language+script+region) beats a language-only match, so a
    // `pt-BR` pack is preferred over `pt` for a pt-BR device.
    let mut language_only: Option<String> = None;
    for registered in guard.keys() {
        let Some(candidate) = LanguageTag::parse(registered) else {
            continue;
        };
        if candidate == wanted {
            return Some(registered.clone());
        }
        if candidate.language == wanted.language && language_only.is_none() {
            language_only = Some(registered.clone());
        }
    }
    language_only
}

/// Look `key` up in the pack `tag` only (no fallback chain).
pub fn lookup_exact(tag: &str, key: &str) -> Option<String> {
    with(tag, |p| p.get(key).map(str::to_string))?
}

/// Look `key` up in the **active** pack only (no fallback chain).
pub fn lookup_active(key: &str) -> Option<String> {
    lookup_exact(&active()?, key)
}

/// A machine-readable description of one pack, for the picker and diagnostics.
pub fn describe(tag: &str, required: &[String]) -> Option<serde_json::Value> {
    let refs: Vec<&str> = required.iter().map(String::as_str).collect();
    with(tag, |p| {
        serde_json::json!({
            "tag": p.tag(),
            "native_name": p.native_name(),
            "english_name": p.english_name(),
            "android_qualifier": serde_json::Value::Null,
            "completeness": p.completeness(refs.iter().copied()),
            "messages": p.len(),
            "base": false,
            "dynamic": true,
            "rtl": p.is_rtl(),
            "plural": p.plural_rule().id(),
            "parent": p.parent().tag(),
            "source": p.source().map(|s| s.display().to_string()),
            "problems": p.problems(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{self, Language, Scope};

    /// Serialises against every other test touching process-wide i18n state.
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        crate::i18n::GLOBAL_I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// A minimal but realistic Japanese pack.
    fn ja() -> &'static str {
        "_meta.tag = ja\n\
         _meta.native_name = 日本語\n\
         _meta.english_name = Japanese\n\
         _meta.plural = other_only\n\
         nav.home = ホーム\n\
         nav.settings = 設定\n\
         unit.mib = MB\n\
         format.rate = {value} {unit}/秒\n\
         duration.minute.other = {count}分\n\
         duration.second.other = {count}秒\n"
    }

    #[test]
    fn parses_metadata_and_strips_it_from_the_messages() {
        let p = LanguagePack::parse(ja(), None).expect("valid pack");
        assert_eq!(p.tag(), "ja");
        assert_eq!(p.native_name(), "日本語");
        assert_eq!(p.english_name(), "Japanese");
        assert_eq!(p.plural_rule(), PluralRule::OtherOnly);
        assert_eq!(p.parent(), Language::BASE);
        assert!(!p.is_rtl());
        assert_eq!(p.get("nav.home"), Some("ホーム"));
        // `_meta.*` must never be reachable as a UI key.
        for meta in ["_meta.tag", "_meta.native_name", "_meta.plural"] {
            assert_eq!(p.get(meta), None, "{meta} leaked into the message table");
        }
        assert!(!p.keys_owned().iter().any(|k| k.starts_with(META_PREFIX)));
    }

    #[test]
    fn falls_back_to_the_file_stem_and_the_tag_for_missing_metadata() {
        let p = LanguagePack::parse("nav.home = Etusivu\n", Some("fi")).expect("valid");
        assert_eq!(p.tag(), "fi");
        // No endonym declared: the row must still be findable, so it shows the tag.
        assert_eq!(p.native_name(), "fi");
        assert_eq!(p.english_name(), "fi");
        assert_eq!(p.plural_rule(), PluralRule::OtherOnly);
    }

    #[test]
    fn canonicalises_the_tag() {
        let p = LanguagePack::parse("nav.home = Início\n", Some("pt_br")).unwrap();
        assert_eq!(p.tag(), "pt-BR");
        let p = LanguagePack::parse("_meta.tag = RU\nnav.home = Дом\n", None).unwrap();
        assert_eq!(p.tag(), "ru");
    }

    #[test]
    fn rejects_packs_that_would_confuse_the_user_or_exhaust_us() {
        // A built-in language: that is what the overlay is for.
        for tag in ["en", "zh-CN", "zh_TW", "zh-Hant", "en-GB"] {
            let err = LanguagePack::parse("nav.home = x\n", Some(tag)).unwrap_err();
            assert!(err.contains("built-in"), "{tag}: {err}");
        }
        // No messages at all would offer a language that renders as Chinese.
        let err = LanguagePack::parse("_meta.tag = ja\n", None).unwrap_err();
        assert!(err.contains("no messages"), "{err}");
        // Unusable tags.
        for bad in ["", "  ", "123", "und", "x"] {
            assert!(
                LanguagePack::parse("nav.home = x\n", Some(bad)).is_err(),
                "{bad:?}"
            );
        }
        // Neither metadata nor a file name.
        assert!(LanguagePack::parse("nav.home = x\n", None).is_err());
    }

    #[test]
    fn unknown_parent_and_plural_degrade_to_safe_defaults() {
        let p = LanguagePack::parse(
            "_meta.tag = ja\n_meta.parent = klingon\n_meta.plural = wat\nnav.home = ホーム\n",
            None,
        )
        .unwrap();
        // The chain must always terminate in Chinese.
        assert_eq!(p.parent(), Language::BASE);
        // An unknown rule must still render: `.other` alone always exists.
        assert_eq!(p.plural_rule(), PluralRule::OtherOnly);
    }

    #[test]
    fn parent_and_plural_are_honoured_when_valid() {
        let p = LanguagePack::parse(
            "_meta.tag = ru\n_meta.parent = en\n_meta.plural = one_other\n_meta.rtl = true\n\
             nav.home = Дом\n",
            None,
        )
        .unwrap();
        assert_eq!(p.parent(), Language::En);
        assert_eq!(p.plural_rule(), PluralRule::OneOther);
        assert!(p.is_rtl());
    }

    #[test]
    fn registry_installs_lists_and_removes() {
        let _g = lock();
        clear();
        assert_eq!(count(), 0);
        assert_eq!(install_text(ja(), None).unwrap(), "ja");
        assert_eq!(tags(), vec!["ja".to_string()]);
        assert!(contains("ja"));
        // Tag normalisation on lookup too.
        assert!(contains("JA"));
        assert!(!contains("ko"));
        assert_eq!(lookup_exact("ja", "nav.home").as_deref(), Some("ホーム"));
        assert_eq!(
            lookup_exact("ja", "nav.instances"),
            None,
            "no fallback here"
        );
        assert!(remove("ja"));
        assert!(!remove("ja"));
        assert_eq!(count(), 0);
        clear();
    }

    #[test]
    fn reinstalling_a_tag_replaces_it() {
        let _g = lock();
        clear();
        install_text(ja(), None).unwrap();
        install_text("_meta.tag = ja\nnav.home = ホーム2\n", None).unwrap();
        assert_eq!(count(), 1, "must replace, not duplicate");
        assert_eq!(lookup_exact("ja", "nav.home").as_deref(), Some("ホーム2"));
        // The replacement dropped the other keys, as a full reload should.
        assert_eq!(lookup_exact("ja", "nav.settings"), None);
        clear();
    }

    #[test]
    fn the_registry_is_bounded() {
        let _g = lock();
        clear();
        // Fill to the cap with distinct, valid, non-built-in tags.
        let mut installed = 0;
        for a in b'a'..=b'z' {
            for b in b'a'..=b'z' {
                let tag = format!("{}{}", a as char, b as char);
                if Language::negotiate(&tag).is_some() {
                    continue;
                }
                if install_text(&format!("_meta.tag = {tag}\nnav.home = x\n"), None).is_ok() {
                    installed += 1;
                } else {
                    break;
                }
                if installed > MAX_PACKS + 5 {
                    break;
                }
            }
            if installed > MAX_PACKS + 5 {
                break;
            }
        }
        assert_eq!(count(), MAX_PACKS, "registry must stop at MAX_PACKS");
        // An existing tag can still be *updated* once full (no lock-out).
        let existing = tags()[0].clone();
        assert!(
            install_text(&format!("_meta.tag = {existing}\nnav.home = y\n"), None).is_ok(),
            "updating an existing pack must work even at the cap"
        );
        clear();
    }

    #[test]
    fn active_selection_tracks_removal() {
        let _g = lock();
        clear();
        install_text(ja(), None).unwrap();
        assert_eq!(set_active(Some("ja")).as_deref(), Some("ja"));
        assert_eq!(active().as_deref(), Some("ja"));
        assert_eq!(lookup_active("nav.home").as_deref(), Some("ホーム"));
        // Removing the active pack must not leave a dangling selection.
        assert!(remove("ja"));
        assert_eq!(active(), None);
        assert_eq!(lookup_active("nav.home"), None);
        // Selecting something unregistered deselects rather than dangling.
        install_text(ja(), None).unwrap();
        set_active(Some("ja"));
        assert_eq!(set_active(Some("ko")), None);
        assert_eq!(active(), None);
        clear();
    }

    #[test]
    fn clear_drops_packs_and_the_selection() {
        let _g = lock();
        clear();
        install_text(ja(), None).unwrap();
        set_active(Some("ja"));
        clear();
        assert_eq!(count(), 0);
        assert_eq!(active(), None);
    }

    #[test]
    fn negotiation_prefers_an_exact_regional_match() {
        let _g = lock();
        clear();
        install_text("_meta.tag = pt\nnav.home = Início\n", None).unwrap();
        install_text("_meta.tag = pt-BR\nnav.home = Início BR\n", None).unwrap();
        assert_eq!(negotiate("pt-BR").as_deref(), Some("pt-BR"));
        assert_eq!(negotiate("pt_br").as_deref(), Some("pt-BR"));
        // Language-only match when the region is different / absent.
        assert_eq!(negotiate("pt").as_deref(), Some("pt"));
        assert_eq!(negotiate("ko"), None);
        assert_eq!(negotiate("junk"), None);
        clear();
    }

    #[test]
    fn loading_a_directory_reports_what_it_skipped() {
        let _g = lock();
        clear();
        let dir = std::env::temp_dir().join(format!("rc-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ja.properties"), ja()).unwrap();
        // A built-in tag: rejected with a reason (use the overlay).
        std::fs::write(dir.join("en.properties"), "nav.home = Home\n").unwrap();
        // Empty: rejected.
        std::fs::write(dir.join("ko.properties"), "# only a comment\n").unwrap();
        // Not a catalogue at all: ignored by extension.
        std::fs::write(dir.join("README.md"), "hello").unwrap();
        // Oversized: rejected before being read.
        std::fs::write(
            dir.join("hu.properties"),
            "x".repeat(MAX_PACK_BYTES as usize + 1),
        )
        .unwrap();

        let report = load_dir(&dir);
        assert_eq!(report.loaded, vec!["ja".to_string()], "{report:?}");
        assert_eq!(report.skipped.len(), 3, "{report:?}");
        assert!(report
            .skipped
            .iter()
            .any(|s| s.contains("en.properties") && s.contains("built-in")));
        assert!(report
            .skipped
            .iter()
            .any(|s| s.contains("ko.properties") && s.contains("no messages")));
        assert!(report
            .skipped
            .iter()
            .any(|s| s.contains("hu.properties") && s.contains("larger than")));
        // Provenance is recorded for the one that loaded.
        assert!(with("ja", |p| p.source().is_some()).unwrap());

        clear();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_directory_is_not_an_error() {
        let _g = lock();
        clear();
        let report = load_dir(Path::new("/definitely/not/here/rc-i18n"));
        assert!(report.loaded.is_empty());
        assert!(report.skipped.is_empty());
        assert_eq!(count(), 0);
    }

    #[test]
    fn a_pack_resolves_through_the_scope_and_falls_back_to_chinese() {
        let _g = lock();
        clear();
        let restore = i18n::current_language();
        install_text(ja(), None).unwrap();

        let scope = Scope::for_tag("ja");
        assert!(scope.is_dynamic());
        assert_eq!(scope.tag(), "ja");
        assert_eq!(scope.language(), Language::BASE);
        // Translated by the pack.
        assert_eq!(i18n::t_scoped(&scope, "nav.home"), "ホーム");
        // Not in the pack: falls back to the base locale (Chinese), never a key.
        assert_eq!(i18n::t_scoped(&scope, "nav.accounts"), "账户");

        // Selecting it makes the plain `t()` helpers speak Japanese.
        assert_eq!(i18n::set_language_tag("ja"), Language::BASE);
        assert_eq!(i18n::current_language_tag(), "ja");
        assert_eq!(i18n::t("nav.home"), "ホーム");
        assert_eq!(i18n::t("nav.accounts"), "账户");
        // ... and the value formatters, because the pack ships those keys too.
        assert_eq!(i18n::rate(1_258_291), "1.2 MB/秒");
        assert_eq!(i18n::duration(200), "3分 20秒");

        // A built-in choice deselects the pack.
        i18n::set_language(Language::En);
        assert_eq!(i18n::current_language_tag(), "en");
        assert_eq!(i18n::t("nav.home"), "Home");

        clear();
        i18n::set_language(restore);
    }

    #[test]
    fn a_pack_can_use_its_own_plural_rule() {
        let _g = lock();
        clear();
        let restore = i18n::current_language();
        install_text(
            "_meta.tag = ru\n_meta.plural = one_other\n\
             download.files.one = {count} файл\n\
             download.files.other = {count} файлов\n",
            None,
        )
        .unwrap();
        let scope = Scope::for_tag("ru");
        assert_eq!(
            scope.plural_rule(),
            crate::i18n::format::PluralRule::OneOther
        );
        assert_eq!(i18n::t_plural_scoped(&scope, "download.files", 1), "1 файл");
        assert_eq!(
            i18n::t_plural_scoped(&scope, "download.files", 5),
            "5 файлов"
        );
        clear();
        i18n::set_language(restore);
    }

    #[test]
    fn the_picker_lists_packs_and_reports_completeness_honestly() {
        let _g = lock();
        clear();
        let before = i18n::available_languages().len();
        install_text(ja(), None).unwrap();
        let langs = i18n::available_languages();
        assert_eq!(langs.len(), before + 1);
        let ja_info = langs.iter().find(|l| l.tag == "ja").expect("listed");
        assert!(ja_info.dynamic);
        assert!(!ja_info.base);
        assert_eq!(ja_info.native_name, "日本語");
        assert_eq!(ja_info.plural, "other_only");
        assert_eq!(ja_info.parent, "zh-CN");
        // A handful of keys out of 128: honest, and far below 1.0.
        assert!(
            ja_info.completeness > 0.0 && ja_info.completeness < 0.2,
            "{}",
            ja_info.completeness
        );
        assert!(langs
            .iter()
            .filter(|l| !l.dynamic)
            .all(|l| l.completeness == 1.0));
        clear();
    }

    #[test]
    fn a_pack_bundle_is_complete_for_the_ui() {
        let _g = lock();
        clear();
        install_text(ja(), None).unwrap();
        let bundle = i18n::bundle_for_tag("ja");
        let base = i18n::bundle(Language::BASE);
        // Same key set as a built-in bundle: the UI must never see a hole.
        assert_eq!(bundle.len(), base.len());
        assert_eq!(bundle.get("nav.home").map(String::as_str), Some("ホーム"));
        // Untranslated keys carry Chinese, not the key.
        assert_eq!(bundle.get("nav.accounts"), base.get("nav.accounts"));
        assert!(bundle.values().all(|v| !v.is_empty()));
        clear();
    }

    #[test]
    fn device_preferences_can_select_a_pack() {
        let _g = lock();
        clear();
        let restore = i18n::current_language();
        install_text(ja(), None).unwrap();

        // A shipped language always wins when it appears first.
        i18n::set_language_from_preferences(["en-US", "ja"]);
        assert_eq!(i18n::current_language_tag(), "en");
        // The pack wins over a later shipped preference.
        i18n::set_language_from_preferences(["ja-JP", "en-US"]);
        assert_eq!(i18n::current_language_tag(), "ja");
        // Nothing matches: Chinese-first.
        i18n::set_language_from_preferences(["de-DE", "fr"]);
        assert_eq!(i18n::current_language_tag(), "zh-CN");

        clear();
        i18n::set_language(restore);
    }

    #[test]
    fn removing_the_active_pack_reverts_the_ui_to_its_parent() {
        let _g = lock();
        clear();
        let restore = i18n::current_language();
        install_text(
            "_meta.tag = ja\n_meta.parent = en\nnav.home = ホーム\n",
            None,
        )
        .unwrap();
        i18n::set_language_tag("ja");
        assert_eq!(i18n::t("nav.home"), "ホーム");
        // Uninstall while it is on screen: must degrade to the parent, not to keys.
        remove("ja");
        assert_eq!(i18n::current_language_tag(), "en");
        assert_eq!(i18n::t("nav.home"), "Home");
        clear();
        i18n::set_language(restore);
    }

    #[test]
    fn diagnostics_surface_pack_health() {
        let _g = lock();
        clear();
        install_text(
            "_meta.tag = ja\nnav.home = ホーム\nnot.a.real.key = x\n\
             settings.language.applied = 変更済み\n",
            None,
        )
        .unwrap();
        let d = i18n::diagnostics();
        assert_eq!(d["pack_count"], 1);
        let packs = d["packs"].as_array().unwrap();
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0]["tag"], "ja");
        assert_eq!(packs[0]["dynamic"], true);
        // An orphan key (a typo in the pack) is reported rather than ignored.
        assert!(packs[0]["orphan_keys"]
            .as_array()
            .unwrap()
            .iter()
            .any(|k| k == "not.a.real.key"));
        // So is placeholder drift: the base has `{language}` here, the pack lost it.
        assert!(!packs[0]["placeholder_mismatch"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(serde_json::to_string(&d).is_ok());
        clear();
    }

    #[test]
    fn concurrent_readers_and_writers_do_not_deadlock_or_tear() {
        let _g = lock();
        clear();
        install_text(ja(), None).unwrap();
        let mut handles = Vec::new();
        for _ in 0..4 {
            handles.push(std::thread::spawn(|| {
                for _ in 0..200 {
                    // Every read must yield either the pack value or the fallback
                    // — never an empty string and never a panic.
                    let v = i18n::t_scoped(&Scope::for_tag("ja"), "nav.home");
                    assert!(!v.is_empty());
                    let _ = tags();
                    let _ = count();
                }
            }));
        }
        for _ in 0..50 {
            let _ = install_text(ja(), None);
            let _ = describe("ja", &["nav.home".to_string()]);
        }
        for h in handles {
            h.join().expect("no panic in a reader");
        }
        clear();
    }
}
