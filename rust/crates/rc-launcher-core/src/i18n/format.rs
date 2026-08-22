//! Message formatting: `{name}` interpolation and plural selection (task 20).
//!
//! Deliberately tiny and dependency-free (no ICU): the launcher only needs
//! named placeholders and the CLDR plural *categories* that our languages use.
//!
//! Robustness rules (a translator must never be able to crash or blank the UI):
//!
//! * an **unknown** placeholder is left verbatim (`{foo}`) so the mistake is
//!   visible in the UI and in tests instead of silently deleting text;
//! * an **unterminated** `{` is emitted verbatim;
//! * `{{` / `}}` are literal braces;
//! * extra arguments are ignored.

use std::collections::BTreeSet;

/// CLDR plural categories used by the shipped languages.
///
/// English uses `One`/`Other`; Chinese only ever uses `Other`. Adding a language
/// with more categories (ru, ar, ...) means extending this enum and
/// [`plural_category`], not the call sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluralCategory {
    One,
    Other,
}

impl PluralCategory {
    /// The `.<suffix>` appended to the base key (`download.files` -> `.one`).
    pub const fn suffix(self) -> &'static str {
        match self {
            PluralCategory::One => "one",
            PluralCategory::Other => "other",
        }
    }
}

use super::Language;

/// The plural category of `count` in `language` (CLDR cardinal rules).
pub fn plural_category(language: Language, count: i64) -> PluralCategory {
    match language {
        // zh has a single form.
        Language::ZhCn | Language::ZhHant => PluralCategory::Other,
        // en: `one` iff exactly 1 (0 and negatives are `other`).
        Language::En => {
            if count == 1 {
                PluralCategory::One
            } else {
                PluralCategory::Other
            }
        }
    }
}

/// Build the plural sub-key for `base` (`"download.files"` + 1 -> `"download.files.one"`).
pub fn plural_key(language: Language, base: &str, count: i64) -> String {
    format!("{}.{}", base, plural_category(language, count).suffix())
}

/// Substitute `{name}` placeholders in `template` from `args`.
///
/// Lookup is by exact (case-sensitive) name. See the module docs for the
/// deliberate lenience around unknown/unterminated placeholders.
pub fn interpolate(template: &str, args: &[(&str, &str)]) -> String {
    // Most messages have no placeholder at all — skip the whole machinery.
    if !template.contains('{') && !template.contains('}') {
        return template.to_string();
    }
    let mut out = String::with_capacity(template.len() + 16);
    let mut chars = template.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '{' => {
                // `{{` -> literal '{'
                if let Some(&(_, '{')) = chars.peek() {
                    chars.next();
                    out.push('{');
                    continue;
                }
                // Find the matching '}' (placeholder names never contain braces).
                let rest = &template[i + c.len_utf8()..];
                match rest.find('}') {
                    Some(end) => {
                        let name = &rest[..end];
                        // Consume the name and the closing brace.
                        for _ in 0..name.chars().count() + 1 {
                            chars.next();
                        }
                        match args.iter().find(|(k, _)| *k == name) {
                            Some((_, v)) => out.push_str(v),
                            // Unknown placeholder: keep it visible.
                            None => {
                                out.push('{');
                                out.push_str(name);
                                out.push('}');
                            }
                        }
                    }
                    // Unterminated '{': emit the rest verbatim and stop.
                    None => {
                        out.push('{');
                        out.push_str(rest);
                        break;
                    }
                }
            }
            '}' => {
                // `}}` -> literal '}'; a stray '}' is emitted as-is.
                if let Some(&(_, '}')) = chars.peek() {
                    chars.next();
                }
                out.push('}');
            }
            _ => out.push(c),
        }
    }
    out
}

/// The set of placeholder names used by `template`.
///
/// Used by the catalogue parity test: a translation whose placeholder set
/// differs from the base locale would render `{path}` to the user (or lose the
/// value entirely), so CI rejects it.
pub fn placeholders(template: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let bytes: Vec<char> = template.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '{' {
            if i + 1 < bytes.len() && bytes[i + 1] == '{' {
                i += 2;
                continue;
            }
            if let Some(rel) = bytes[i + 1..].iter().position(|&c| c == '}') {
                let name: String = bytes[i + 1..i + 1 + rel].iter().collect();
                // `{}` is not a placeholder, and neither is `{ }`.
                if !name.trim().is_empty() {
                    out.insert(name);
                }
                i += rel + 2;
                continue;
            }
            break;
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_named_placeholders() {
        assert_eq!(
            interpolate(
                "hello {name}, you are {age}",
                &[("name", "Steve"), ("age", "9")]
            ),
            "hello Steve, you are 9"
        );
        // The same placeholder twice.
        assert_eq!(interpolate("{a}-{a}", &[("a", "x")]), "x-x");
    }

    #[test]
    fn passes_through_text_without_placeholders() {
        assert_eq!(
            interpolate("纯文本，无占位符", &[("a", "1")]),
            "纯文本，无占位符"
        );
        assert_eq!(interpolate("", &[]), "");
    }

    #[test]
    fn keeps_unknown_placeholders_visible() {
        // Losing the text silently would hide the bug; keep it greppable.
        assert_eq!(interpolate("v={value}", &[]), "v={value}");
        assert_eq!(interpolate("{a}/{b}", &[("a", "1")]), "1/{b}");
    }

    #[test]
    fn handles_escaped_and_stray_braces() {
        assert_eq!(interpolate("{{literal}}", &[]), "{literal}");
        assert_eq!(interpolate("{{{a}}}", &[("a", "X")]), "{X}");
        assert_eq!(interpolate("100%} done", &[]), "100%} done");
    }

    #[test]
    fn unterminated_placeholder_is_emitted_verbatim() {
        assert_eq!(interpolate("oops {name", &[("name", "x")]), "oops {name");
        assert_eq!(interpolate("{", &[]), "{");
    }

    #[test]
    fn ignores_extra_arguments() {
        assert_eq!(interpolate("{a}", &[("a", "1"), ("b", "2")]), "1");
    }

    #[test]
    fn is_multibyte_safe() {
        // Byte-index arithmetic must not split a UTF-8 sequence.
        assert_eq!(
            interpolate("路径：{path}（已损坏）", &[("path", "/存储/a.jar")]),
            "路径：/存储/a.jar（已损坏）"
        );
        assert_eq!(interpolate("表情🎮{x}🎮", &[("x", "！")]), "表情🎮！🎮");
    }

    #[test]
    fn extracts_the_placeholder_set() {
        let p = placeholders("{b} then {a} then {b}");
        assert_eq!(p.into_iter().collect::<Vec<_>>(), vec!["a", "b"]);
        assert!(placeholders("no placeholders here").is_empty());
        // Escaped braces and empty braces are not placeholders.
        assert!(placeholders("{{a}}").is_empty());
        assert!(placeholders("{}").is_empty());
        assert!(placeholders("{ }").is_empty());
        // An unterminated brace ends the scan without panicking.
        assert!(placeholders("{oops").is_empty());
        assert_eq!(
            placeholders("{count} 个文件")
                .into_iter()
                .collect::<Vec<_>>(),
            vec!["count"]
        );
    }

    #[test]
    fn plural_categories_follow_cldr() {
        use crate::i18n::Language;
        assert_eq!(plural_category(Language::En, 1), PluralCategory::One);
        for n in [0, 2, 3, 11, -1, i64::MAX] {
            assert_eq!(
                plural_category(Language::En, n),
                PluralCategory::Other,
                "en {}",
                n
            );
        }
        // Chinese has a single form for every count.
        for n in [0, 1, 2, 100, -5] {
            assert_eq!(plural_category(Language::ZhCn, n), PluralCategory::Other);
            assert_eq!(plural_category(Language::ZhHant, n), PluralCategory::Other);
        }
    }

    #[test]
    fn builds_plural_keys() {
        use crate::i18n::Language;
        assert_eq!(
            plural_key(Language::En, "download.files", 1),
            "download.files.one"
        );
        assert_eq!(
            plural_key(Language::En, "download.files", 2),
            "download.files.other"
        );
        assert_eq!(
            plural_key(Language::ZhCn, "download.files", 1),
            "download.files.other"
        );
        assert_eq!(PluralCategory::One.suffix(), "one");
        assert_eq!(PluralCategory::Other.suffix(), "other");
    }
}
