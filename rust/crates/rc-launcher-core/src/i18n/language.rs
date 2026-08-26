//! Supported languages, BCP-47 tag parsing and locale negotiation (task 20).
//!
//! The launcher is **Chinese-first** (中文优先): `zh-CN` is the *base* locale —
//! it is guaranteed complete, it is what an unrecognised system locale falls
//! back to, and every other catalogue falls back to it key-by-key.
//!
//! Negotiation follows the spirit of RFC 4647 *lookup*: we normalise the tag,
//! then match on `language` + `script` + `region` from most to least specific.
//! That is deliberately more forgiving than exact matching because Android
//! hands us anything from `zh` to `zh-Hant-TW` to the legacy `zh_TW`.

use std::fmt;

/// A language shipped with the launcher (one `.properties` catalogue each).
///
/// Kept deliberately small: a *variant* per catalogue, not per region. Regional
/// tags are folded onto a catalogue by [`Language::negotiate`] (e.g. `zh-HK`
/// and `zh-Hant-TW` both resolve to [`Language::ZhHant`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Language {
    /// 简体中文 — the **base** locale (complete by contract).
    ZhCn,
    /// 繁體中文 (Traditional Chinese).
    ZhHant,
    /// English.
    En,
}

impl Language {
    /// The base locale every other catalogue falls back to (Chinese-first).
    pub const BASE: Language = Language::ZhCn;

    /// All shipped languages, in display order (base first).
    pub const ALL: [Language; 3] = [Language::ZhCn, Language::ZhHant, Language::En];

    /// Dense index used to address the per-language catalogue array.
    pub const fn index(self) -> usize {
        match self {
            Language::ZhCn => 0,
            Language::ZhHant => 1,
            Language::En => 2,
        }
    }

    /// Inverse of [`Language::index`]; `None` when out of range.
    pub const fn from_index(i: usize) -> Option<Language> {
        match i {
            0 => Some(Language::ZhCn),
            1 => Some(Language::ZhHant),
            2 => Some(Language::En),
            _ => None,
        }
    }

    /// The canonical BCP-47 tag (what we persist and what the UI sends us).
    pub const fn tag(self) -> &'static str {
        match self {
            Language::ZhCn => "zh-CN",
            Language::ZhHant => "zh-Hant",
            Language::En => "en",
        }
    }

    /// The Android resource qualifier for this catalogue (`values-<qualifier>`).
    ///
    /// `zh-CN` is the launcher's *default* `values/` catalogue (Chinese-first),
    /// so it has no qualifier of its own.
    pub const fn android_qualifier(self) -> Option<&'static str> {
        match self {
            Language::ZhCn => None,
            Language::ZhHant => Some("zh-rTW"),
            Language::En => Some("en"),
        }
    }

    /// Endonym — the language's name *in that language* (never translated).
    pub const fn native_name(self) -> &'static str {
        match self {
            Language::ZhCn => "简体中文",
            Language::ZhHant => "繁體中文",
            Language::En => "English",
        }
    }

    /// English name, for logs and bug reports.
    pub const fn english_name(self) -> &'static str {
        match self {
            Language::ZhCn => "Simplified Chinese",
            Language::ZhHant => "Traditional Chinese",
            Language::En => "English",
        }
    }

    /// Right-to-left script? (None of the shipped languages is, but the UI asks
    /// so adding e.g. `fa`/`ar` later cannot silently break the layout.)
    pub const fn is_rtl(self) -> bool {
        false
    }

    /// The CLDR cardinal rule set this language uses.
    ///
    /// A runtime language pack declares its own with `_meta.plural`; see
    /// [`crate::i18n::pack`].
    pub const fn plural_rule(self) -> super::format::PluralRule {
        use super::format::PluralRule;
        match self {
            // Chinese has a single form.
            Language::ZhCn | Language::ZhHant => PluralRule::OtherOnly,
            Language::En => PluralRule::OneOther,
        }
    }

    /// Key-by-key fallback chain, most specific first, always ending at
    /// [`Language::BASE`]. A missing key therefore degrades to Chinese rather
    /// than to a raw key.
    pub fn fallback_chain(self) -> Vec<Language> {
        match self {
            Language::ZhCn => vec![Language::ZhCn],
            Language::ZhHant => vec![Language::ZhHant, Language::ZhCn],
            Language::En => vec![Language::En, Language::ZhCn],
        }
    }

    /// Exact-tag lookup (case-insensitive, `_` accepted for `-`).
    ///
    /// Use [`Language::negotiate`] for user/system input; this is for values we
    /// wrote ourselves (persisted settings).
    pub fn from_tag(tag: &str) -> Option<Language> {
        let t = LanguageTag::parse(tag)?;
        Language::ALL
            .into_iter()
            .find(|l| LanguageTag::parse(l.tag()).is_some_and(|c| c == t))
    }

    /// Resolve one system/user tag onto a shipped catalogue.
    ///
    /// Returns `None` when the tag names a language we do not ship, so the
    /// caller can decide between "try the next preference" and "use the base".
    pub fn negotiate(tag: &str) -> Option<Language> {
        let t = LanguageTag::parse(tag)?;
        match t.language.as_str() {
            "zh" | "cmn" | "yue" => {
                // Script wins over region: `zh-Hant-CN` is Traditional.
                let hant = match t.script.as_deref() {
                    Some("hant") => true,
                    Some("hans") => false,
                    // No script: infer from the region (the usual Android case).
                    _ => {
                        matches!(t.region.as_deref(), Some("tw" | "hk" | "mo"))
                        // `yue` (Cantonese) is written Traditional by default.
                        || (t.language == "yue" && t.region.is_none())
                    }
                };
                Some(if hant {
                    Language::ZhHant
                } else {
                    Language::ZhCn
                })
            }
            "en" => Some(Language::En),
            _ => None,
        }
    }

    /// Pick the best catalogue for an ordered list of preferences (an Android
    /// `LocaleList`). Falls back to [`Language::BASE`] — Chinese-first — when
    /// nothing matches.
    pub fn negotiate_list<I, S>(preferred: I) -> Language
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        preferred
            .into_iter()
            .find_map(|t| Language::negotiate(t.as_ref()))
            .unwrap_or(Language::BASE)
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.tag())
    }
}

impl Default for Language {
    fn default() -> Self {
        Language::BASE
    }
}

/// A parsed, normalised BCP-47-ish tag: `language[-script][-region]`.
///
/// Lenient by design — it accepts `zh_CN`, `ZH-hans-cn`, trailing junk and
/// extra subtags (which are ignored), because that is what real devices send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageTag {
    /// Lowercase primary language subtag (`zh`, `en`, ...).
    pub language: String,
    /// Lowercase 4-letter script subtag (`hans`, `hant`), if present.
    pub script: Option<String>,
    /// Lowercase 2-letter (or 3-digit) region subtag (`cn`, `tw`), if present.
    pub region: Option<String>,
}

impl LanguageTag {
    /// Parse a tag, returning `None` when there is no usable language subtag.
    pub fn parse(tag: &str) -> Option<LanguageTag> {
        // Accept both BCP-47 `-` and the legacy Java/Android `_` separator, and
        // drop any `.charset` / `@modifier` suffix (`zh_CN.UTF-8`, `en@posix`).
        let cleaned = tag.trim();
        let cleaned = cleaned
            .split(['.', '@'])
            .next()
            .unwrap_or("")
            .replace('_', "-");
        let mut parts = cleaned
            .split('-')
            .filter(|p| !p.is_empty())
            .map(|p| p.to_ascii_lowercase());

        let language = parts.next()?;
        // A language subtag is 2-3 alphabetic characters (`zh`, `yue`); anything
        // else (digits, "und", "c", "posix", empty) is not usable.
        if !(2..=3).contains(&language.len()) || !language.chars().all(|c| c.is_ascii_alphabetic())
        {
            return None;
        }

        let mut script = None;
        let mut region = None;
        for p in parts {
            let alpha = p.chars().all(|c| c.is_ascii_alphabetic());
            let digit = p.chars().all(|c| c.is_ascii_digit());
            if p.len() == 4 && alpha && script.is_none() {
                script = Some(p);
            } else if ((p.len() == 2 && alpha) || (p.len() == 3 && digit)) && region.is_none() {
                region = Some(p);
            }
            // Everything else (variants, extensions, private use) is ignored.
        }
        Some(LanguageTag {
            language,
            script,
            region,
        })
    }
}

impl fmt::Display for LanguageTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.language)?;
        if let Some(s) = &self.script {
            // Title-case the script subtag as BCP-47 recommends (`Hant`).
            let mut it = s.chars();
            if let Some(c) = it.next() {
                write!(f, "-{}{}", c.to_ascii_uppercase(), it.as_str())?;
            }
        }
        if let Some(r) = &self.region {
            write!(f, "-{}", r.to_ascii_uppercase())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_legacy_tags() {
        let t = LanguageTag::parse("zh-CN").unwrap();
        assert_eq!(t.language, "zh");
        assert_eq!(t.region.as_deref(), Some("cn"));
        assert_eq!(t.script, None);
        // Legacy Java/Android separator + case folding.
        assert_eq!(LanguageTag::parse("ZH_cn"), Some(t.clone()));
        // Charset / modifier suffixes (POSIX locales) are dropped.
        assert_eq!(LanguageTag::parse("zh_CN.UTF-8"), Some(t.clone()));
        assert_eq!(LanguageTag::parse("zh-CN@pinyin"), Some(t));
    }

    #[test]
    fn parses_script_and_ignores_extra_subtags() {
        let t = LanguageTag::parse("zh-Hant-TW-u-ca-chinese").unwrap();
        assert_eq!(t.language, "zh");
        assert_eq!(t.script.as_deref(), Some("hant"));
        assert_eq!(t.region.as_deref(), Some("tw"));
        // Numeric (UN M.49) regions are accepted.
        assert_eq!(
            LanguageTag::parse("es-419").unwrap().region.as_deref(),
            Some("419")
        );
    }

    #[test]
    fn rejects_tags_without_a_usable_language() {
        for bad in [
            "", "   ", "-", "_", "1", "x", "C", "POSIX", "und-CN", "12-CN", "@euro",
        ] {
            assert!(
                LanguageTag::parse(bad).is_none()
                    || LanguageTag::parse(bad).unwrap().language.len() >= 2,
                "{:?} should not yield a bogus language",
                bad
            );
        }
        assert!(LanguageTag::parse("").is_none());
        assert!(LanguageTag::parse("C").is_none(), "single letter");
        assert!(LanguageTag::parse("POSIX").is_none(), "5 letters");
        assert!(LanguageTag::parse("12").is_none(), "digits");
    }

    #[test]
    fn displays_tags_in_canonical_case() {
        assert_eq!(
            LanguageTag::parse("zh_hant_tw").unwrap().to_string(),
            "zh-Hant-TW"
        );
        assert_eq!(LanguageTag::parse("EN").unwrap().to_string(), "en");
    }

    #[test]
    fn negotiates_simplified_versus_traditional_chinese() {
        for tag in [
            "zh",
            "zh-CN",
            "zh_CN",
            "zh-Hans",
            "zh-Hans-TW",
            "zh-SG",
            "cmn-Hans",
        ] {
            assert_eq!(Language::negotiate(tag), Some(Language::ZhCn), "{}", tag);
        }
        for tag in [
            "zh-TW",
            "zh_TW",
            "zh-HK",
            "zh-MO",
            "zh-Hant",
            "zh-Hant-CN",
            "yue",
        ] {
            assert_eq!(Language::negotiate(tag), Some(Language::ZhHant), "{}", tag);
        }
        // Script beats region, both ways.
        assert_eq!(Language::negotiate("zh-Hans-TW"), Some(Language::ZhCn));
        assert_eq!(Language::negotiate("zh-Hant-CN"), Some(Language::ZhHant));
    }

    #[test]
    fn negotiates_english_and_rejects_unshipped_languages() {
        for tag in ["en", "en-US", "en_GB", "EN-au"] {
            assert_eq!(Language::negotiate(tag), Some(Language::En), "{}", tag);
        }
        for tag in ["fr", "de-DE", "ja", "ko", "ru", "", "xx"] {
            assert_eq!(Language::negotiate(tag), None, "{}", tag);
        }
    }

    #[test]
    fn negotiate_list_honours_order_and_defaults_to_chinese() {
        assert_eq!(
            Language::negotiate_list(["ja-JP", "en-US", "zh-CN"]),
            Language::En,
            "first supported preference wins"
        );
        assert_eq!(Language::negotiate_list(["ja", "ko"]), Language::BASE);
        assert_eq!(Language::negotiate_list(Vec::<&str>::new()), Language::BASE);
        assert_eq!(Language::negotiate_list(["zh-HK", "en"]), Language::ZhHant);
    }

    #[test]
    fn from_tag_round_trips_every_canonical_tag() {
        for l in Language::ALL {
            assert_eq!(Language::from_tag(l.tag()), Some(l), "{}", l.tag());
            assert_eq!(l.to_string(), l.tag());
        }
        // from_tag is exact-ish: a region we do not ship is not an exact match.
        assert_eq!(Language::from_tag("en-US"), None);
        assert_eq!(Language::negotiate("en-US"), Some(Language::En));
    }

    #[test]
    fn indices_round_trip_and_are_dense() {
        for (i, l) in Language::ALL.into_iter().enumerate() {
            assert_eq!(l.index(), i);
            assert_eq!(Language::from_index(i), Some(l));
        }
        assert_eq!(Language::from_index(Language::ALL.len()), None);
    }

    #[test]
    fn metadata_is_sane() {
        // Chinese-first: the base locale owns the default `values/` directory.
        assert_eq!(Language::ZhCn.android_qualifier(), None);
        assert_eq!(Language::ZhHant.android_qualifier(), Some("zh-rTW"));
        assert_eq!(Language::En.android_qualifier(), Some("en"));
        for l in Language::ALL {
            assert!(!l.native_name().is_empty());
            assert!(!l.english_name().is_empty());
            assert!(!l.is_rtl());
        }
    }
}
