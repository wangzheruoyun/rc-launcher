//! Internationalisation (task 20) — a resource-file i18n framework.
//!
//! Design goals, in priority order:
//!
//! 1. **中文优先 (Chinese-first).** `zh-CN` is the *base* locale: it is complete
//!    by contract (a unit test enforces it), it is the default when the device
//!    locale is unknown, and every other catalogue falls back to it key-by-key.
//!    An untranslated string therefore shows Chinese copy, never a raw key.
//! 2. **Resource files, not code.** Messages live in
//!    `rust/crates/rc-launcher-core/i18n/<tag>.properties` — the same division
//!    of labour as FCL's `values-*/strings.xml` and MCTier's `src/i18n/*.ts`.
//!    They are `include_str!`-embedded, so lookups are allocation-free and the
//!    release build performs no I/O.
//! 3. **Switchable at runtime.** [`set_language`] is a single atomic store; the
//!    Compose layer re-reads the strings and recomposes. No process restart, no
//!    Activity recreation.
//! 4. **Single source of truth across the FFI.** [`bundle`] hands Kotlin the
//!    whole resolved catalogue, so the launcher core and the UI can never
//!    disagree about a crash advice or an error message.
//!
//! ```no_run
//! use rc_launcher::i18n::{self, Language};
//!
//! i18n::set_language(Language::negotiate_list(["zh-Hant-TW", "en-US"]));
//! assert_eq!(i18n::current_language(), Language::ZhHant);
//! let msg = i18n::t_args("error.checksum", &[("path", "/sdcard/x.jar")]);
//! ```
//!
//! Sub-modules: [`language`] (tags + negotiation), [`catalog`] (`.properties`
//! parsing, embedded catalogues, runtime overlay), [`format`] (`{name}`
//! interpolation + plural rules).

pub mod catalog;
pub mod format;
pub mod language;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{OnceLock, RwLock};

pub use catalog::Catalog;
pub use format::PluralCategory;
pub use language::{Language, LanguageTag};

/// The process-wide UI language, as a [`Language::index`].
///
/// An atomic (not a mutex) so `t()` on a hot path — e.g. a per-frame status
/// label — never blocks and can never deadlock against a translation write.
static CURRENT: AtomicUsize = AtomicUsize::new(0); // 0 == Language::BASE (zh-CN)

/// Keys that were requested but not found, for [`diagnostics`]. Bounded so a
/// buggy caller in a loop cannot grow it without limit.
static MISSING: OnceLock<RwLock<BTreeSet<String>>> = OnceLock::new();
const MISSING_CAP: usize = 256;

fn missing() -> &'static RwLock<BTreeSet<String>> {
    MISSING.get_or_init(|| RwLock::new(BTreeSet::new()))
}

fn record_missing(key: &str) {
    if let Ok(mut g) = missing().write() {
        if g.len() < MISSING_CAP {
            g.insert(key.to_string());
        }
    }
}

/// The keys `t*` failed to resolve so far (sorted). Empty in a healthy build.
pub fn missing_keys() -> Vec<String> {
    missing()
        .read()
        .map(|g| g.iter().cloned().collect())
        .unwrap_or_default()
}

/// Forget the recorded missing keys (tests / "clear diagnostics").
pub fn reset_missing_keys() {
    if let Ok(mut g) = missing().write() {
        g.clear();
    }
}

// --- Current language ------------------------------------------------------

/// The active UI language.
pub fn current_language() -> Language {
    Language::from_index(CURRENT.load(Ordering::Relaxed)).unwrap_or(Language::BASE)
}

/// Switch the active UI language. Cheap enough to call from a click handler.
pub fn set_language(language: Language) {
    CURRENT.store(language.index(), Ordering::Relaxed);
}

/// Switch by tag, negotiating the closest shipped catalogue.
///
/// An empty / unknown / `"system"` tag resolves to the base locale, so the UI
/// can pass the raw persisted value (or the device locale) straight through.
/// Returns the language actually selected.
pub fn set_language_tag(tag: &str) -> Language {
    let chosen = Language::from_tag(tag)
        .or_else(|| Language::negotiate(tag))
        .unwrap_or(Language::BASE);
    set_language(chosen);
    chosen
}

/// Negotiate + apply an ordered preference list (an Android `LocaleList`).
pub fn set_language_from_preferences<I, S>(preferred: I) -> Language
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let chosen = Language::negotiate_list(preferred);
    set_language(chosen);
    chosen
}

// --- Translation -----------------------------------------------------------

/// Translate `key` in `language`, or `None` when no catalogue in the fallback
/// chain has it.
pub fn lookup(language: Language, key: &str) -> Option<String> {
    catalog::lookup(language, key)
}

/// Translate `key` in `language`; unknown keys yield the key itself.
///
/// Returning the key (rather than an empty string or a panic) keeps the UI
/// readable *and* makes the mistake obvious and greppable. The key is also
/// recorded for [`diagnostics`].
pub fn t_in(language: Language, key: &str) -> String {
    match catalog::lookup(language, key) {
        Some(v) => v,
        None => {
            record_missing(key);
            key.to_string()
        }
    }
}

/// Translate `key` in the [`current_language`].
pub fn t(key: &str) -> String {
    t_in(current_language(), key)
}

/// Translate `key` in `language` and substitute `{name}` placeholders.
pub fn t_args_in(language: Language, key: &str, args: &[(&str, &str)]) -> String {
    format::interpolate(&t_in(language, key), args)
}

/// Translate `key` in the current language and substitute placeholders.
pub fn t_args(key: &str, args: &[(&str, &str)]) -> String {
    t_args_in(current_language(), key, args)
}

/// Plural-aware translation: picks `<base>.one` / `<base>.other` per the
/// language's CLDR rules and exposes `count` as the `{count}` placeholder.
pub fn t_plural_in(language: Language, base: &str, count: i64) -> String {
    let key = format::plural_key(language, base, count);
    let n = count.to_string();
    t_args_in(language, &key, &[("count", n.as_str())])
}

/// Plural-aware translation in the current language.
pub fn t_plural(base: &str, count: i64) -> String {
    t_plural_in(current_language(), base, count)
}

/// Whether `key` resolves in `language` (including via fallback).
pub fn has_key(language: Language, key: &str) -> bool {
    catalog::lookup(language, key).is_some()
}

// --- Bundles (for the UI / FFI) -------------------------------------------

/// Every key known to the launcher, sorted. This is the base-locale key set
/// (the contract), unioned with anything an overlay added.
pub fn all_keys() -> BTreeSet<String> {
    let mut keys: BTreeSet<String> = catalog::embedded_for(Language::BASE)
        .keys()
        .into_iter()
        .map(str::to_string)
        .collect();
    for l in Language::ALL {
        keys.extend(
            catalog::embedded_for(l)
                .keys()
                .into_iter()
                .map(str::to_string),
        );
        // An overlay may introduce keys no compiled-in catalogue has (a new
        // string shipped ahead of the app), so the bundle must see them too.
        keys.extend(catalog::overlay_keys(l));
    }
    keys
}

/// The whole catalogue of `language`, fully resolved through the fallback chain.
///
/// Handed to Kotlin by `RustBridge.i18nBundle(tag)` so the Compose layer renders
/// the *same* strings as the core (crash advice, error text, ...).
pub fn bundle(language: Language) -> BTreeMap<String, String> {
    all_keys()
        .into_iter()
        .filter_map(|k| catalog::lookup(language, &k).map(|v| (k, v)))
        .collect()
}

/// [`bundle`] as JSON (`{ "<key>": "<message>", ... }`).
pub fn bundle_json(language: Language) -> serde_json::Value {
    serde_json::Value::Object(
        bundle(language)
            .into_iter()
            .map(|(k, v)| (k, serde_json::Value::String(v)))
            .collect(),
    )
}

/// How many of the base-locale keys `language` actually translates itself.
///
/// The base locale is 100% by definition; a partially translated catalogue
/// reports honestly and still renders (via fallback).
pub fn completeness(language: Language) -> f32 {
    let total = catalog::embedded_for(Language::BASE).len();
    if total == 0 {
        return 1.0;
    }
    let own = catalog::embedded_for(Language::BASE)
        .keys()
        .into_iter()
        .filter(|k| catalog::lookup_exact(language, k).is_some())
        .count();
    own as f32 / total as f32
}

/// UI-facing description of one shipped language.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LanguageInfo {
    /// Canonical BCP-47 tag, e.g. `zh-CN` (what the UI persists).
    pub tag: String,
    /// The language's own name — never translated, so it is always readable.
    pub native_name: String,
    /// English name, for logs / bug reports.
    pub english_name: String,
    /// Android resource qualifier (`values-<qualifier>`), `null` for the default.
    pub android_qualifier: Option<String>,
    /// Fraction of base-locale keys translated in this catalogue (0.0 - 1.0).
    pub completeness: f32,
    /// Number of messages in this catalogue.
    pub messages: usize,
    /// True for the base (fallback) locale.
    pub base: bool,
    /// Right-to-left script.
    pub rtl: bool,
}

/// Every shipped language, base first — drives the settings picker.
pub fn available_languages() -> Vec<LanguageInfo> {
    Language::ALL
        .into_iter()
        .map(|l| LanguageInfo {
            tag: l.tag().to_string(),
            native_name: l.native_name().to_string(),
            english_name: l.english_name().to_string(),
            android_qualifier: l.android_qualifier().map(str::to_string),
            completeness: completeness(l),
            messages: catalog::embedded_for(l).len(),
            base: l == Language::BASE,
            rtl: l.is_rtl(),
        })
        .collect()
}

/// A machine-readable health report of the catalogues.
///
/// Exposed over the FFI (`RustBridge.i18nDiagnostics()`) and asserted by the
/// unit tests, so a translator's mistake is caught in CI and visible in-app.
pub fn diagnostics() -> serde_json::Value {
    let base = catalog::embedded_for(Language::BASE);
    let base_keys = base.keys();
    let mut per_language = Vec::new();

    for l in Language::ALL {
        let cat = catalog::embedded_for(l);
        let keys = cat.keys();
        let missing: Vec<&str> = base_keys.difference(&keys).copied().collect();
        let orphan: Vec<&str> = keys.difference(&base_keys).copied().collect();
        // Placeholder drift: the translation must expose the same {names}.
        let mut placeholder_mismatch = Vec::new();
        for k in keys.intersection(&base_keys) {
            let want = format::placeholders(base.get(k).unwrap_or(""));
            let got = format::placeholders(cat.get(k).unwrap_or(""));
            if want != got {
                placeholder_mismatch.push(serde_json::json!({
                    "key": k,
                    "expected": want.into_iter().collect::<Vec<_>>(),
                    "actual": got.into_iter().collect::<Vec<_>>(),
                }));
            }
        }
        let empty: Vec<&str> = keys
            .iter()
            .copied()
            .filter(|k| cat.get(k).is_some_and(|v| v.trim().is_empty()))
            .collect();

        per_language.push(serde_json::json!({
            "tag": l.tag(),
            "messages": cat.len(),
            "completeness": completeness(l),
            "missing_keys": missing,
            "orphan_keys": orphan,
            "empty_values": empty,
            "placeholder_mismatch": placeholder_mismatch,
            "parse_problems": cat.problems(),
        }));
    }

    serde_json::json!({
        "current": current_language().tag(),
        "base": Language::BASE.tag(),
        "total_keys": base.len(),
        "overlay_active": catalog::has_overlay(),
        "missing_at_runtime": missing_keys(),
        "languages": per_language,
    })
}

// --- Crash diagnosis integration (task 7 / 19) ----------------------------

/// The i18n key of a crash category's one-line summary.
///
/// Keys are derived from [`crate::launch::crash::CrashCategory::id`] so the
/// catalogue and the classifier can never drift apart.
pub fn crash_summary_key(id: &str) -> String {
    format!("crash.{}.summary", id)
}

/// The i18n key of a crash category's actionable advice.
pub fn crash_advice_key(id: &str) -> String {
    format!("crash.{}.advice", id)
}

// --- Overlay (community translations / wording hot-fixes) ------------------

/// Merge a `.properties` document into `language`'s runtime overlay.
pub fn install_overlay_text(language: Language, text: &str) -> usize {
    catalog::install_overlay_text(language, text)
}

/// Load `<tag>.properties` files from `dir` into the overlay.
pub fn load_overlay_dir<P: AsRef<std::path::Path>>(dir: P) -> usize {
    catalog::load_overlay_dir(dir.as_ref())
}

/// Drop every overlay entry, restoring the compiled-in copy.
pub fn clear_overlay() {
    catalog::clear_overlay();
}

/// Serialises tests that touch the *process-wide* i18n state (the current
/// language and the runtime overlay).
///
/// A single lock shared by every test module (`i18n`, `ffi`, `capi`, `error`) —
/// the same discipline as [`crate::event::GLOBAL_BUS_TEST_LOCK`]. Without it a
/// reader in one module can observe an overlay another module installed, because
/// `cargo test` runs the whole binary's tests in parallel threads.
#[cfg(test)]
pub static GLOBAL_I18N_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch::crash::CrashCategory;

    /// Serialises against every other test that touches the global i18n state.
    fn global_lock() -> std::sync::MutexGuard<'static, ()> {
        GLOBAL_I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    // --- Shipped catalogues: the CI gate ---------------------------------

    #[test]
    fn every_shipped_catalogue_parses_without_problems() {
        for l in Language::ALL {
            let cat = catalog::embedded_for(l);
            assert!(
                cat.problems().is_empty(),
                "{} has parse problems: {:?}",
                l.tag(),
                cat.problems()
            );
            assert!(!cat.is_empty(), "{} is empty", l.tag());
        }
    }

    #[test]
    fn base_locale_is_chinese_and_first() {
        assert_eq!(Language::BASE, Language::ZhCn, "中文优先");
        assert_eq!(Language::ALL[0], Language::BASE);
        assert_eq!(Language::default(), Language::ZhCn);
        // The default of the atomic must be the base locale.
        assert_eq!(Language::from_index(0), Some(Language::BASE));
    }

    #[test]
    fn all_catalogues_have_exactly_the_base_key_set() {
        let base = catalog::embedded_for(Language::BASE).keys();
        for l in Language::ALL {
            let keys = catalog::embedded_for(l).keys();
            let missing: Vec<_> = base.difference(&keys).collect();
            let orphan: Vec<_> = keys.difference(&base).collect();
            assert!(missing.is_empty(), "{} misses {:?}", l.tag(), missing);
            assert!(
                orphan.is_empty(),
                "{} has keys the base locale lacks (typo?): {:?}",
                l.tag(),
                orphan
            );
            assert_eq!(completeness(l), 1.0, "{} incomplete", l.tag());
        }
    }

    #[test]
    fn no_message_is_empty_or_still_a_key() {
        for l in Language::ALL {
            let cat = catalog::embedded_for(l);
            for k in cat.keys() {
                let v = cat.get(k).unwrap();
                assert!(!v.trim().is_empty(), "{}: {} is empty", l.tag(), k);
                assert_ne!(v, k, "{}: {} was left as its own key", l.tag(), k);
            }
        }
    }

    #[test]
    fn placeholders_match_the_base_locale_everywhere() {
        let base = catalog::embedded_for(Language::BASE);
        for l in Language::ALL {
            let cat = catalog::embedded_for(l);
            for k in base.keys() {
                let want = format::placeholders(base.get(k).unwrap());
                if let Some(v) = cat.get(k) {
                    assert_eq!(want, format::placeholders(v), "{} / {}", l.tag(), k);
                }
            }
        }
    }

    #[test]
    fn diagnostics_report_a_clean_bill_of_health() {
        let _g = global_lock();
        let d = diagnostics();
        assert_eq!(d["base"], "zh-CN");
        assert!(d["total_keys"].as_u64().unwrap() >= 90);
        for lang in d["languages"].as_array().unwrap() {
            assert!(lang["missing_keys"].as_array().unwrap().is_empty());
            assert!(lang["orphan_keys"].as_array().unwrap().is_empty());
            assert!(lang["empty_values"].as_array().unwrap().is_empty());
            assert!(lang["placeholder_mismatch"].as_array().unwrap().is_empty());
            assert!(lang["parse_problems"].as_array().unwrap().is_empty());
            assert_eq!(lang["completeness"], 1.0);
        }
    }

    // --- Translation ----------------------------------------------------

    #[test]
    fn translates_per_language() {
        let _g = global_lock();
        assert_eq!(t_in(Language::ZhCn, "nav.home"), "主页");
        assert_eq!(t_in(Language::ZhHant, "nav.home"), "主頁");
        assert_eq!(t_in(Language::En, "nav.home"), "Home");
    }

    #[test]
    fn unknown_key_returns_the_key_and_is_recorded() {
        let _g = global_lock();
        reset_missing_keys();
        assert_eq!(t_in(Language::En, "nope.not.here"), "nope.not.here");
        assert!(missing_keys().contains(&"nope.not.here".to_string()));
        reset_missing_keys();
        assert!(missing_keys().is_empty());
    }

    #[test]
    fn missing_key_recording_is_bounded() {
        let _g = global_lock();
        reset_missing_keys();
        for i in 0..(MISSING_CAP + 50) {
            let _ = t_in(Language::En, &format!("absent.key.{}", i));
        }
        assert_eq!(missing_keys().len(), MISSING_CAP);
        reset_missing_keys();
    }

    #[test]
    fn interpolates_named_arguments() {
        let _g = global_lock();
        let s = t_args_in(
            Language::En,
            "error.checksum",
            &[("path", "/sdcard/mods/a.jar")],
        );
        assert_eq!(s, "Checksum verification failed: /sdcard/mods/a.jar");
        assert!(!s.contains('{'), "placeholder left unresolved: {}", s);
    }

    #[test]
    fn interpolates_multiple_arguments_in_every_language() {
        let _g = global_lock();
        for l in Language::ALL {
            let s = t_args_in(
                l,
                "error.retry_scheduled",
                &[("attempt", "3"), ("delay_secs", "8")],
            );
            assert!(s.contains('3') && s.contains('8'), "{}: {}", l.tag(), s);
            assert!(!s.contains('{'), "{}: {}", l.tag(), s);
        }
    }

    #[test]
    fn plural_rules_follow_the_language() {
        let _g = global_lock();
        // English distinguishes one/other ...
        assert_eq!(t_plural_in(Language::En, "download.files", 1), "1 file");
        assert_eq!(t_plural_in(Language::En, "download.files", 3), "3 files");
        assert_eq!(t_plural_in(Language::En, "download.files", 0), "0 files");
        // ... Chinese does not.
        assert_eq!(
            t_plural_in(Language::ZhCn, "download.files", 1),
            "共 1 个文件"
        );
        assert_eq!(
            t_plural_in(Language::ZhCn, "download.files", 7),
            "共 7 个文件"
        );
    }

    #[test]
    fn current_language_is_switchable_at_runtime() {
        let _g = global_lock();
        let restore = current_language();
        set_language(Language::En);
        assert_eq!(current_language(), Language::En);
        assert_eq!(t("nav.settings"), "Settings");
        set_language(Language::ZhCn);
        assert_eq!(t("nav.settings"), "设置");
        assert_eq!(t_plural("download.files", 2), "共 2 个文件");
        set_language(restore);
    }

    #[test]
    fn set_language_tag_negotiates_and_defaults_to_chinese() {
        let _g = global_lock();
        let restore = current_language();
        assert_eq!(set_language_tag("zh-Hant"), Language::ZhHant);
        assert_eq!(set_language_tag("zh_TW"), Language::ZhHant);
        assert_eq!(set_language_tag("en-US"), Language::En);
        assert_eq!(set_language_tag("zh-CN"), Language::ZhCn);
        // Unknown / empty / "system" -> base locale (Chinese-first).
        assert_eq!(set_language_tag("fr-FR"), Language::ZhCn);
        assert_eq!(set_language_tag(""), Language::ZhCn);
        assert_eq!(set_language_tag("system"), Language::ZhCn);
        assert_eq!(
            set_language_from_preferences(["xx", "de", "en-GB"]),
            Language::En
        );
        assert_eq!(
            set_language_from_preferences(Vec::<String>::new()),
            Language::ZhCn
        );
        set_language(restore);
    }

    // --- Bundles --------------------------------------------------------

    #[test]
    fn bundle_is_complete_for_every_language() {
        let _g = global_lock();
        let keys = all_keys();
        for l in Language::ALL {
            let b = bundle(l);
            assert_eq!(b.len(), keys.len(), "{} bundle incomplete", l.tag());
            for k in &keys {
                let v = b
                    .get(k)
                    .unwrap_or_else(|| panic!("{} lacks {}", l.tag(), k));
                assert!(!v.trim().is_empty());
            }
        }
    }

    #[test]
    fn bundle_json_is_a_flat_string_map() {
        let _g = global_lock();
        let j = bundle_json(Language::En);
        assert!(j.is_object());
        assert_eq!(j["nav.home"], "Home");
        assert!(j.as_object().unwrap().values().all(|v| v.is_string()));
    }

    #[test]
    fn available_languages_describe_the_picker() {
        let langs = available_languages();
        assert_eq!(langs.len(), 3);
        assert!(langs[0].base, "the base locale must come first");
        assert_eq!(langs[0].tag, "zh-CN");
        assert_eq!(langs[0].native_name, "简体中文");
        assert_eq!(langs[0].android_qualifier, None, "zh-CN is values/");
        assert!(langs.iter().all(|l| l.completeness == 1.0));
        assert!(langs.iter().all(|l| l.messages >= 90));
        assert!(langs.iter().all(|l| !l.rtl));
        // Endonyms are never translated, so the picker is always readable.
        let names: Vec<_> = langs.iter().map(|l| l.native_name.as_str()).collect();
        assert_eq!(names, vec!["简体中文", "繁體中文", "English"]);
        // Android qualifiers are unique.
        let quals: BTreeSet<_> = langs
            .iter()
            .filter_map(|l| l.android_qualifier.clone())
            .collect();
        assert_eq!(quals.len(), 2);
    }

    // --- Crash / error integration --------------------------------------

    #[test]
    fn every_crash_category_is_translated_in_every_language() {
        let _g = global_lock();
        let cats = [
            CrashCategory::CleanExit,
            CrashCategory::UserTerminated,
            CrashCategory::KilledBySystem,
            CrashCategory::OutOfMemory,
            CrashCategory::UnsupportedJavaVersion,
            CrashCategory::MissingNativeLibrary,
            CrashCategory::GraphicsFailure,
            CrashCategory::NativeCrash,
            CrashCategory::CorruptedFile,
            CrashCategory::MissingMainClass,
            CrashCategory::AuthenticationFailure,
            CrashCategory::DiskFull,
            CrashCategory::PermissionDenied,
            CrashCategory::ModLoaderFailure,
            CrashCategory::GameError,
            CrashCategory::Unknown,
        ];
        for c in cats {
            for l in Language::ALL {
                for key in [crash_summary_key(c.id()), crash_advice_key(c.id())] {
                    let v = catalog::lookup_exact(l, &key)
                        .unwrap_or_else(|| panic!("{} lacks {}", l.tag(), key));
                    assert!(!v.trim().is_empty());
                }
            }
        }
    }

    #[test]
    fn crash_helpers_agree_with_the_catalogue() {
        let _g = global_lock();
        // The accessors on CrashCategory are thin views over these catalogues,
        // so a wording change in a .properties file reaches the UI *and* the
        // core with no code change.
        let c = CrashCategory::OutOfMemory;
        assert_eq!(
            c.advice(),
            catalog::lookup_exact(Language::En, &crash_advice_key(c.id())).unwrap()
        );
        assert_eq!(
            c.advice_zh(),
            catalog::lookup_exact(Language::ZhCn, &crash_advice_key(c.id())).unwrap()
        );
        assert_eq!(
            c.summary(),
            catalog::lookup_exact(Language::En, &crash_summary_key(c.id())).unwrap()
        );
        // The localised accessors read the same catalogues ...
        assert_eq!(c.localized_advice(Language::En), c.advice());
        assert_eq!(c.localized_advice(Language::ZhCn), c.advice_zh());
        assert_eq!(c.localized_summary(Language::En), c.summary());
        // ... and Traditional Chinese is a real translation, not a fallback.
        let hant = c.localized_advice(Language::ZhHant);
        assert!(!hant.trim().is_empty());
        assert_ne!(hant, c.advice_zh(), "zh-Hant must not fall back here");
        assert!(hant.contains("記憶體"), "unexpected zh-Hant copy: {}", hant);
    }

    // --- Overlay --------------------------------------------------------

    #[test]
    fn overlay_shadows_the_compiled_in_catalogue_and_can_be_cleared() {
        let _g = global_lock();
        clear_overlay();
        assert!(!catalog::has_overlay());
        assert_eq!(t_in(Language::En, "nav.home"), "Home");

        let n = install_overlay_text(Language::En, "nav.home = Dashboard\nnav.new = Brand new\n");
        assert_eq!(n, 2);
        assert!(catalog::has_overlay());
        assert_eq!(t_in(Language::En, "nav.home"), "Dashboard");
        // A key only the overlay knows still resolves ...
        assert_eq!(t_in(Language::En, "nav.new"), "Brand new");
        // ... and other languages are untouched.
        assert_eq!(t_in(Language::ZhCn, "nav.home"), "主页");
        // The bundle picks the overlay up too (the UI sees the hot-fix).
        assert_eq!(bundle(Language::En)["nav.home"], "Dashboard");

        clear_overlay();
        assert!(!catalog::has_overlay());
        assert_eq!(t_in(Language::En, "nav.home"), "Home");
    }

    #[test]
    fn overlay_directory_is_loaded_by_tag_and_ignores_junk() {
        let _g = global_lock();
        clear_overlay();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("en.properties"), "nav.home = Base camp\n").unwrap();
        // A regional tag is negotiated onto its catalogue ...
        std::fs::write(dir.path().join("zh-TW.properties"), "nav.home = 首頁\n").unwrap();
        // ... an unsupported language and non-.properties files are skipped.
        std::fs::write(dir.path().join("fr.properties"), "nav.home = Accueil\n").unwrap();
        std::fs::write(dir.path().join("readme.txt"), "nav.home = nope\n").unwrap();

        let n = load_overlay_dir(dir.path());
        assert_eq!(n, 2, "only en + zh-TW should load");
        assert_eq!(t_in(Language::En, "nav.home"), "Base camp");
        assert_eq!(t_in(Language::ZhHant, "nav.home"), "首頁");
        assert_eq!(t_in(Language::ZhCn, "nav.home"), "主页");

        clear_overlay();
    }

    #[test]
    fn missing_overlay_directory_is_not_an_error() {
        let _g = global_lock();
        clear_overlay();
        assert_eq!(load_overlay_dir("/nonexistent/rc/i18n"), 0);
        assert!(!catalog::has_overlay());
    }

    #[test]
    fn overlay_survives_a_malformed_document() {
        let _g = global_lock();
        clear_overlay();
        // Line 2 has no separator and must not prevent lines 1 and 3 loading.
        let n = install_overlay_text(
            Language::En,
            "nav.home = Fine\nthis line is broken\nnav.settings = Also fine\n",
        );
        assert_eq!(n, 2);
        assert_eq!(t_in(Language::En, "nav.home"), "Fine");
        assert_eq!(t_in(Language::En, "nav.settings"), "Also fine");
        clear_overlay();
    }

    #[test]
    fn fallback_chain_reaches_chinese_for_a_partial_translation() {
        let _g = global_lock();
        clear_overlay();
        // Simulate a brand-new key that only the base locale has.
        install_overlay_text(Language::ZhCn, "brand.new.key = 全新文案\n");
        assert_eq!(t_in(Language::ZhCn, "brand.new.key"), "全新文案");
        // Untranslated in en / zh-Hant -> Chinese copy, never a raw key.
        assert_eq!(t_in(Language::En, "brand.new.key"), "全新文案");
        assert_eq!(t_in(Language::ZhHant, "brand.new.key"), "全新文案");
        clear_overlay();
    }

    #[test]
    fn an_overlay_introduced_key_reaches_the_bundle() {
        let _g = global_lock();
        clear_overlay();
        // A string shipped ahead of the app (new key, English only).
        install_overlay_text(Language::En, "brand.new.button = Ship it\n");
        assert!(all_keys().contains("brand.new.button"));
        let en = bundle(Language::En);
        assert_eq!(en["brand.new.button"], "Ship it");
        // Languages that cannot resolve it are simply not given the key (rather
        // than being handed an English string or a raw key).
        assert!(!bundle(Language::ZhCn).contains_key("brand.new.button"));
        clear_overlay();
        assert!(!all_keys().contains("brand.new.button"));
    }

    #[test]
    fn translating_is_thread_safe_while_the_language_changes() {
        let _g = global_lock();
        clear_overlay();
        let restore = current_language();
        // `t()` claims to be safe on a hot path (an atomic load, no lock), so
        // hammer it from several threads while another flips the language.
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let readers: Vec<_> = (0..4)
            .map(|_| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    let mut n = 0usize;
                    while !stop.load(Ordering::Relaxed) {
                        // Whatever the language, a shipped key always resolves
                        // to real copy — never to the key itself.
                        let v = t("nav.home");
                        assert_ne!(v, "nav.home");
                        assert!(!v.is_empty());
                        n += 1;
                    }
                    n
                })
            })
            .collect();

        for _ in 0..200 {
            for l in Language::ALL {
                set_language(l);
            }
        }
        stop.store(true, Ordering::Relaxed);
        let total: usize = readers.into_iter().map(|h| h.join().unwrap()).sum();
        assert!(total > 0, "readers should have made progress");
        set_language(restore);
    }

    #[test]
    fn language_chains_all_end_at_the_base_locale() {
        for l in Language::ALL {
            let chain = l.fallback_chain();
            assert_eq!(chain[0], l, "a language must try itself first");
            assert_eq!(*chain.last().unwrap(), Language::BASE);
        }
    }
}
