//! Resource-file catalogues: a `.properties` parser, the compiled-in message
//! catalogues and an optional on-disk **overlay** (task 20).
//!
//! Why resource files rather than a table in Rust?
//!
//! * translators edit `i18n/<tag>.properties` without touching (or compiling)
//!   Rust — the same division of labour as FCL's `values-*/strings.xml`;
//! * the files are `include_str!`-embedded, so a release build has **zero**
//!   runtime I/O and lookups return `&'static str` (no allocation);
//! * the very same files can be *overlaid* at runtime
//!   ([`install_overlay_text`] / [`load_overlay_dir`]) so a community
//!   translation or a wording hot-fix ships without a new APK.
//!
//! ## Accepted syntax (a strict subset of Java `.properties`)
//!
//! ```text
//! # comment            ! also a comment
//! key = value          (the FIRST unescaped '=' separates; ':' is NOT a separator)
//! key = first line \
//!       continued      (trailing '\' continues; leading blanks are dropped)
//! escapes: \n \r \t \f \\ \= \: \uXXXX  and '\ ' for a significant space
//! ```
//!
//! Unlike Java we do **not** treat `:` or whitespace as a separator: Chinese
//! copy is full of `：` and `.properties` keys in this project never contain
//! spaces, so this rule removes a whole class of translator surprises.
//!
//! Parsing never fails. Malformed lines are skipped and recorded in
//! [`Catalog::problems`], which the unit tests assert is empty for every shipped
//! file — a broken *shipped* file breaks CI, a broken *overlay* is ignored.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::{OnceLock, RwLock};

use super::Language;

/// A parsed message catalogue: `key -> message`.
#[derive(Debug, Default, Clone)]
pub struct Catalog {
    entries: HashMap<String, String>,
    problems: Vec<String>,
}

impl Catalog {
    /// Parse a `.properties` document. Never fails; see [`Catalog::problems`].
    pub fn parse(text: &str) -> Catalog {
        let mut entries: HashMap<String, String> = HashMap::new();
        let mut problems = Vec::new();

        // Strip a UTF-8 BOM; editors on Windows love adding one.
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);

        let mut logical = String::new();
        let mut logical_start = 0usize;
        let mut pending = false;

        for (n, raw) in text.lines().enumerate() {
            // `lines()` already dropped '\n'; drop a stray '\r' from CRLF files.
            let line = raw.strip_suffix('\r').unwrap_or(raw);

            if !pending {
                let trimmed = line.trim_start();
                if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('!') {
                    continue;
                }
                logical.clear();
                logical_start = n + 1;
                logical.push_str(trimmed);
            } else {
                // Continuation lines: leading whitespace is insignificant.
                logical.push_str(line.trim_start());
            }

            // A trailing *odd* number of backslashes escapes the newline.
            let trailing = logical.chars().rev().take_while(|&c| c == '\\').count();
            if trailing % 2 == 1 {
                logical.pop(); // drop the escaping backslash
                pending = true;
                continue;
            }
            pending = false;

            match split_entry(&logical) {
                Some((k, v)) => {
                    let key = unescape(k).trim().to_string();
                    if key.is_empty() {
                        problems.push(format!("line {}: empty key", logical_start));
                        continue;
                    }
                    let value = unescape(v.trim());
                    if let Some(old) = entries.insert(key.clone(), value) {
                        problems.push(format!(
                            "line {}: duplicate key `{}` (previous value {:?} overridden)",
                            logical_start, key, old
                        ));
                    }
                }
                None => problems.push(format!(
                    "line {}: no `=` separator in {:?}",
                    logical_start, logical
                )),
            }
        }

        if pending {
            problems.push("last line ends with a dangling `\\` continuation".to_string());
            // Salvage what we can rather than dropping the entry silently.
            if let Some((k, v)) = split_entry(&logical) {
                let key = unescape(k).trim().to_string();
                if !key.is_empty() {
                    entries.insert(key, unescape(v.trim()));
                }
            }
        }

        Catalog { entries, problems }
    }

    /// Look one key up in *this* catalogue (no fallback).
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    /// Number of messages.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every key, sorted — stable output for diagnostics and tests.
    pub fn keys(&self) -> BTreeSet<&str> {
        self.entries.keys().map(String::as_str).collect()
    }

    /// Problems found while parsing (malformed lines, duplicate keys, ...).
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// Merge `other` into `self`, `other` winning. Used by the overlay.
    pub fn merge(&mut self, other: &Catalog) {
        for (k, v) in &other.entries {
            self.entries.insert(k.clone(), v.clone());
        }
    }
}

/// Split a logical line at the first **unescaped** `=`.
fn split_entry(line: &str) -> Option<(&str, &str)> {
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '=' => return Some((&line[..i], &line[i + 1..])),
            _ => {}
        }
    }
    None
}

/// Resolve `.properties` escape sequences.
///
/// An unknown escape yields the escaped character itself (`\q` -> `q`), which is
/// exactly what `java.util.Properties` does.
fn unescape(s: &str) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('f') => out.push('\u{c}'),
            Some('u') => {
                // \uXXXX — keep the sequence verbatim if it is malformed.
                let hex: String = it.clone().take(4).collect();
                if hex.len() == 4 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                    for _ in 0..4 {
                        it.next();
                    }
                    match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        Some(ch) => out.push(ch),
                        None => {
                            out.push_str("\\u");
                            out.push_str(&hex);
                        }
                    }
                } else {
                    out.push_str("\\u");
                }
            }
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

// --- Compiled-in catalogues -------------------------------------------------

/// The shipped resource files, embedded at compile time (base locale first).
const EMBEDDED: [(Language, &str); 3] = [
    (Language::ZhCn, include_str!("../../i18n/zh-CN.properties")),
    (
        Language::ZhHant,
        include_str!("../../i18n/zh-Hant.properties"),
    ),
    (Language::En, include_str!("../../i18n/en.properties")),
];

static CATALOGS: OnceLock<[Catalog; 3]> = OnceLock::new();

/// The parsed, compiled-in catalogues (parsed once, then shared).
pub fn embedded() -> &'static [Catalog; 3] {
    CATALOGS.get_or_init(|| {
        let mut out: [Catalog; 3] = Default::default();
        for (lang, text) in EMBEDDED {
            out[lang.index()] = Catalog::parse(text);
        }
        out
    })
}

/// The compiled-in catalogue of one language.
pub fn embedded_for(language: Language) -> &'static Catalog {
    &embedded()[language.index()]
}

/// Look `key` up in the compiled-in catalogue of `language` only (no fallback,
/// no overlay). Returns a `&'static str`, so hot paths never allocate.
pub fn lookup_exact(language: Language, key: &str) -> Option<&'static str> {
    embedded_for(language).get(key)
}

/// Look `key` up following `language`'s fallback chain (compiled-in only).
pub fn lookup_static(language: Language, key: &str) -> Option<&'static str> {
    language
        .fallback_chain()
        .into_iter()
        .find_map(|l| lookup_exact(l, key))
}

// --- Runtime overlay -------------------------------------------------------

static OVERLAY: OnceLock<RwLock<[Catalog; 3]>> = OnceLock::new();

fn overlay() -> &'static RwLock<[Catalog; 3]> {
    OVERLAY.get_or_init(|| RwLock::new(Default::default()))
}

/// Merge `text` (a `.properties` document) into the runtime overlay of
/// `language`, shadowing the compiled-in catalogue. Returns the number of
/// messages installed.
///
/// Overlay entries take priority over compiled-in ones, which is what makes a
/// wording hot-fix or a community translation possible without a new APK.
/// Malformed lines are skipped (never fatal).
pub fn install_overlay_text(language: Language, text: &str) -> usize {
    let parsed = Catalog::parse(text);
    let n = parsed.len();
    if let Ok(mut guard) = overlay().write() {
        guard[language.index()].merge(&parsed);
    }
    n
}

/// Load every `<tag>.properties` in `dir` into the overlay.
///
/// The file *stem* is negotiated with [`Language::negotiate`], so `zh-TW.properties`
/// lands on [`Language::ZhHant`] and unknown languages are skipped. Returns the
/// number of messages installed. A missing directory is not an error — it just
/// installs nothing (offline/robustness contract, task 19).
pub fn load_overlay_dir(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    // Sort for deterministic precedence when two files map to one language.
    let mut files: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "properties"))
        .collect();
    files.sort();

    let mut installed = 0;
    for path in files {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Some(language) = Language::from_tag(stem).or_else(|| Language::negotiate(stem)) else {
            continue;
        };
        if let Ok(text) = std::fs::read_to_string(&path) {
            installed += install_overlay_text(language, &text);
        }
    }
    installed
}

/// Drop every overlay entry (used by tests and by "reset to built-in copy").
pub fn clear_overlay() {
    if let Ok(mut guard) = overlay().write() {
        *guard = Default::default();
    }
}

/// True when any overlay entry is installed.
pub fn has_overlay() -> bool {
    overlay()
        .read()
        .map(|g| g.iter().any(|c| !c.is_empty()))
        .unwrap_or(false)
}

/// The keys `language`'s overlay contributes (empty when no overlay is loaded).
///
/// Owned `String`s because the overlay lives behind an `RwLock` and may be
/// replaced at any time, unlike the `&'static` compiled-in catalogues.
pub fn overlay_keys(language: Language) -> BTreeSet<String> {
    overlay()
        .read()
        .ok()
        .and_then(|g| {
            g.get(language.index())
                .map(|c| c.keys().into_iter().map(str::to_string).collect())
        })
        .unwrap_or_default()
}

/// Overlay-aware lookup for one language (no fallback chain).
fn overlay_exact(language: Language, key: &str) -> Option<String> {
    overlay()
        .read()
        .ok()?
        .get(language.index())?
        .get(key)
        .map(str::to_string)
}

/// Look `key` up: overlay first, then the compiled-in catalogue, walking
/// `language`'s fallback chain. Returns `None` when no catalogue has the key.
pub fn lookup(language: Language, key: &str) -> Option<String> {
    for l in language.fallback_chain() {
        if let Some(v) = overlay_exact(l, key) {
            return Some(v);
        }
        if let Some(v) = lookup_exact(l, key) {
            return Some(v.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_entries_and_trims_around_the_separator() {
        let c = Catalog::parse("a=1\n b = 2 \nc\t=\t3\n");
        assert_eq!(c.get("a"), Some("1"));
        assert_eq!(c.get("b"), Some("2"));
        assert_eq!(c.get("c"), Some("3"));
        assert_eq!(c.len(), 3);
        assert!(c.problems().is_empty(), "{:?}", c.problems());
    }

    #[test]
    fn skips_comments_and_blank_lines() {
        let c =
            Catalog::parse("# comment = not an entry\n\n! also a comment\n   # indented\nk=v\n");
        assert_eq!(c.len(), 1);
        assert_eq!(c.get("k"), Some("v"));
        assert!(c.problems().is_empty());
    }

    #[test]
    fn only_the_first_equals_separates_so_values_may_contain_equals_and_colons() {
        let c = Catalog::parse("k = Java 8 for <= 1.16, note: fine = ok\n");
        assert_eq!(c.get("k"), Some("Java 8 for <= 1.16, note: fine = ok"));
        // A colon alone never separates (Chinese copy is full of `：`).
        let c = Catalog::parse("err.io = 读取失败：文件不存在\n");
        assert_eq!(c.get("err.io"), Some("读取失败：文件不存在"));
    }

    #[test]
    fn handles_bom_and_crlf() {
        let c = Catalog::parse("\u{feff}a=1\r\nb=2\r\n");
        assert_eq!(c.get("a"), Some("1"));
        assert_eq!(c.get("b"), Some("2"));
        assert!(c.problems().is_empty());
    }

    #[test]
    fn joins_continuation_lines_dropping_leading_blanks() {
        let c = Catalog::parse("msg = first \\\n      second \\\n      third\nnext = x\n");
        assert_eq!(c.get("msg"), Some("first second third"));
        assert_eq!(c.get("next"), Some("x"));
        assert!(c.problems().is_empty());
    }

    #[test]
    fn an_even_run_of_backslashes_does_not_continue_the_line() {
        // `\\` is an escaped backslash, so the line ends there.
        let c = Catalog::parse("path = C:\\\\\nnext = 1\n");
        assert_eq!(c.get("path"), Some("C:\\"));
        assert_eq!(c.get("next"), Some("1"));
        assert!(c.problems().is_empty());
    }

    #[test]
    fn resolves_escape_sequences() {
        let c = Catalog::parse(
            "nl = a\\nb\ntab = a\\tb\nesc = 1\\=2\\:3\nuni = \\u4F60\\u597D\nunknown = \\q\n",
        );
        assert_eq!(c.get("nl"), Some("a\nb"));
        assert_eq!(c.get("tab"), Some("a\tb"));
        assert_eq!(c.get("esc"), Some("1=2:3"));
        assert_eq!(c.get("uni"), Some("你好"));
        assert_eq!(
            c.get("unknown"),
            Some("q"),
            "unknown escapes drop the backslash"
        );
    }

    #[test]
    fn a_malformed_unicode_escape_is_kept_verbatim() {
        let c = Catalog::parse("k = \\u12 and \\uZZZZ\n");
        assert_eq!(c.get("k"), Some("\\u12 and \\uZZZZ"));
    }

    #[test]
    fn an_escaped_equals_can_appear_in_a_key() {
        let c = Catalog::parse("a\\=b = v\n");
        assert_eq!(c.get("a=b"), Some("v"));
    }

    #[test]
    fn a_significant_trailing_space_needs_an_escape() {
        let c = Catalog::parse("k = value\\u0020\n");
        assert_eq!(c.get("k"), Some("value "));
    }

    #[test]
    fn records_lines_without_a_separator() {
        let c = Catalog::parse("good = 1\nthis is not an entry\nalso.good = 2\n");
        assert_eq!(c.len(), 2, "the good lines still load");
        assert_eq!(c.problems().len(), 1);
        assert!(c.problems()[0].contains("line 2"), "{:?}", c.problems());
        assert!(c.problems()[0].contains("no `=` separator"));
    }

    #[test]
    fn records_duplicate_and_empty_keys() {
        let c = Catalog::parse("k = first\nk = second\n = orphan\n");
        assert_eq!(c.get("k"), Some("second"), "last one wins");
        assert_eq!(c.problems().len(), 2);
        assert!(c.problems().iter().any(|p| p.contains("duplicate key")));
        assert!(c.problems().iter().any(|p| p.contains("empty key")));
    }

    #[test]
    fn salvages_a_dangling_continuation_at_eof() {
        let c = Catalog::parse("k = value \\");
        assert_eq!(c.get("k"), Some("value"));
        assert_eq!(c.problems().len(), 1);
        assert!(c.problems()[0].contains("dangling"));
    }

    #[test]
    fn empty_and_whitespace_documents_are_fine() {
        for doc in ["", "\n\n", "   \n\t\n", "# only a comment\n"] {
            let c = Catalog::parse(doc);
            assert!(c.is_empty());
            assert!(c.problems().is_empty(), "{:?}", c.problems());
        }
    }

    #[test]
    fn an_empty_value_is_allowed_but_visible_to_the_parity_test() {
        let c = Catalog::parse("k =\n");
        assert_eq!(c.get("k"), Some(""));
        assert!(c.problems().is_empty());
    }

    #[test]
    fn merge_lets_the_other_catalogue_win() {
        let mut a = Catalog::parse("x = 1\ny = 2\n");
        let b = Catalog::parse("y = 22\nz = 3\n");
        a.merge(&b);
        assert_eq!(a.get("x"), Some("1"));
        assert_eq!(a.get("y"), Some("22"));
        assert_eq!(a.get("z"), Some("3"));
        assert_eq!(a.len(), 3);
    }

    #[test]
    fn embedded_catalogues_are_parsed_once_and_shared() {
        let a = embedded_for(Language::ZhCn) as *const Catalog;
        let b = embedded_for(Language::ZhCn) as *const Catalog;
        assert_eq!(a, b, "the OnceLock must hand out the same instance");
        assert!(embedded_for(Language::ZhCn).len() >= 90);
    }

    #[test]
    fn lookup_exact_does_not_fall_back_but_lookup_static_does() {
        assert!(lookup_exact(Language::En, "nav.home").is_some());
        assert_eq!(lookup_exact(Language::En, "totally.absent"), None);
        assert_eq!(lookup_static(Language::En, "totally.absent"), None);
        // Every base key resolves in every language through the chain.
        for k in embedded_for(Language::BASE).keys() {
            for l in Language::ALL {
                assert!(lookup_static(l, k).is_some(), "{} / {}", l.tag(), k);
            }
        }
    }

    #[test]
    fn keys_are_sorted_and_deduplicated() {
        let c = Catalog::parse("b = 1\na = 2\nb = 3\n");
        assert_eq!(c.keys().into_iter().collect::<Vec<_>>(), vec!["a", "b"]);
    }
}
