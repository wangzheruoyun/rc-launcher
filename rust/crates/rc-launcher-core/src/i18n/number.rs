//! Locale-aware value formatting: numbers, byte sizes, rates, percentages,
//! durations and relative time (task 20).
//!
//! ## Why this lives in the core
//!
//! A launcher's most visible copy is *derived*: `1.4 GB`, `1.2 MB/秒`,
//! `剩余 3 分 20 秒`, `42.5%`. Before this module each side of the FFI grew its
//! own private formatter — the Compose layer shipped an English
//! `arrayOf("B", "KB", "MB", …)` while the core spoke Chinese — which quietly
//! broke the module contract that the catalogue is the *single source of truth*
//! (see [`super`]). Everything here is therefore assembled from **catalogue
//! keys**, so a translator owns the unit names, the separators *and* the
//! assembly templates:
//!
//! | catalogue key | role |
//! |---|---|
//! | `format.group_separator` / `format.decimal_separator` | digit grouping |
//! | `format.size` / `format.rate` / `format.percent` | `{value}`/`{unit}` skeletons |
//! | `format.progress_of` / `format.duration_join` / `format.fps` | assembly |
//! | `unit.byte` … `unit.pib` | 1024-based byte units |
//! | `duration.zero`, `duration.{day,hour,minute,second}.{one,other}` | duration pieces |
//! | `relative.now` / `relative.past` / `relative.future` | relative time |
//! | `download.eta` | remaining-time phrasing |
//!
//! ## Robustness rules (task 19 discipline)
//!
//! No input can panic, and nothing developer-facing can leak into the UI:
//!
//! * a non-finite `f64` renders as `format.invalid_number` (`—`), never `NaN`
//!   or `inf`;
//! * a missing / empty catalogue key falls back to a compiled-in ASCII default,
//!   so even a catastrophically broken runtime overlay still renders digits;
//! * requested precision is clamped to [`MAX_FRACTION_DIGITS`], so a bogus
//!   `fraction_digits` cannot allocate an enormous string;
//! * `-0.0` after rounding is printed as `0`, not `-0`;
//! * rounding is applied *before* the byte unit is chosen, so 1023.97 KiB is
//!   promoted to `1.0 MB` instead of the nonsensical `1024.0 KB`;
//! * every integer conversion saturates ([`i64::MIN`] included).
//!
//! ```
//! use rc_launcher::i18n::{number, Language};
//!
//! assert_eq!(number::format_bytes(Language::En, 1_536), "1.5 KB");
//! assert_eq!(number::format_int(Language::En, 1_234_567), "1,234,567");
//! assert_eq!(number::format_duration(Language::ZhCn, 200), "3 分 20 秒");
//! ```

use super::{format, Scope};

/// Compiled-in separator used when `format.group_separator` is unavailable.
const DEFAULT_GROUP_SEPARATOR: &str = ",";
/// Compiled-in separator used when `format.decimal_separator` is unavailable.
const DEFAULT_DECIMAL_SEPARATOR: &str = ".";
/// Digits per group (all shipped languages use Western 3-digit grouping).
const GROUP_SIZE: usize = 3;

/// Upper bound on `fraction_digits`, so a bogus caller cannot make us allocate
/// a megabyte-long number.
pub const MAX_FRACTION_DIGITS: usize = 9;

/// Byte-unit catalogue keys, smallest first (1024-based).
pub const BYTE_UNIT_KEYS: [&str; 6] = [
    "unit.byte",
    "unit.kib",
    "unit.mib",
    "unit.gib",
    "unit.tib",
    "unit.pib",
];

/// Last-resort unit names, index-aligned with [`BYTE_UNIT_KEYS`].
const BYTE_UNIT_FALLBACK: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];

/// Duration units, largest first:
/// `(seconds, catalogue plural base key, compiled-in fallback template)`.
///
/// The fallback is what renders when the catalogue cannot supply the piece. It
/// matters more than it looks: [`super::t_plural_in`] echoes the *key* on a miss,
/// which is right for prose but disastrous inside an assembled label — a progress
/// row reading `duration.minute.other duration.second.other` is worse than a
/// terse `3 min 20 s`. The Kotlin port carries the same table.
pub const DURATION_UNITS: [(i64, &str, &str); 4] = [
    (86_400, "duration.day", "{count} d"),
    (3_600, "duration.hour", "{count} h"),
    (60, "duration.minute", "{count} min"),
    (1, "duration.second", "{count} s"),
];

/// How many duration pieces [`format_duration`] shows (`3 分 20 秒`).
const DEFAULT_DURATION_PARTS: usize = 2;

/// Below this many seconds a relative timestamp reads `relative.now`.
const RELATIVE_NOW_THRESHOLD: i64 = 60;

/// A catalogue lookup that can never yield an empty string.
///
/// Unlike [`super::t_in`] a miss is *not* recorded as a diagnostic and never
/// echoes the key: a half-formatted `unit.mib` in the middle of a progress
/// label would be worse than a plain `MB`.
fn skeleton(scope: &Scope, key: &str, fallback: &str) -> String {
    match super::lookup_scoped(scope, key) {
        Some(value) if !value.is_empty() => value,
        _ => fallback.to_string(),
    }
}

/// A lookup where an **explicitly empty** value is meaningful.
///
/// [`skeleton`] treats blank as missing, which is right for a unit name or a
/// template (an empty `format.size` would blank the whole label). The *grouping*
/// separator is the one exception: `format.group_separator =` is how a catalogue
/// says "do not group digits at all", so an entry that exists and is empty is
/// honoured rather than silently replaced by `,`. Only an *absent* key falls back.
///
/// `RcValueFormat.groupSeparator` mirrors this exactly.
fn group_separator(scope: &Scope) -> String {
    match super::lookup_scoped(scope, "format.group_separator") {
        Some(v) => v,
        None => DEFAULT_GROUP_SEPARATOR.to_string(),
    }
}

/// Insert `separator` every [`GROUP_SIZE`] digits, counting from the right.
///
/// `digits` must be ASCII decimal digits (it always is: we produce it), which
/// is what makes the byte slicing below sound.
fn group_digits(digits: &str, separator: &str) -> String {
    if separator.is_empty() || digits.len() <= GROUP_SIZE {
        return digits.to_string();
    }
    let mut out = String::with_capacity(digits.len() + digits.len() / GROUP_SIZE * separator.len());
    // The first group is the remainder, so `1234567` splits as `1|234|567`.
    let head = match digits.len() % GROUP_SIZE {
        0 => GROUP_SIZE,
        n => n,
    };
    out.push_str(&digits[..head]);
    let mut i = head;
    while i < digits.len() {
        out.push_str(separator);
        out.push_str(&digits[i..i + GROUP_SIZE]);
        i += GROUP_SIZE;
    }
    out
}

/// Group an integer with `language`'s separator (`1,234,567`).
pub fn format_int(language: impl Into<Scope>, value: i64) -> String {
    let scope = language.into();
    let separator = group_separator(&scope);
    // `unsigned_abs` so `i64::MIN` cannot overflow.
    let grouped = group_digits(&value.unsigned_abs().to_string(), &separator);
    if value < 0 {
        format!("-{grouped}")
    } else {
        grouped
    }
}

/// Group an unsigned integer with `language`'s separator.
pub fn format_uint(language: impl Into<Scope>, value: u64) -> String {
    let scope = language.into();
    let separator = group_separator(&scope);
    group_digits(&value.to_string(), &separator)
}

/// Round half **away from zero** at `digits` decimals.
///
/// Rust's `{:.N}` uses round-half-to-*even* (`1.25` -> `"1.2"`), while Java and
/// Kotlin's `String.format` use half-up (`1.25` -> `"1.3"`). The Compose layer
/// mirrors this module byte for byte, so the two sides of the FFI would
/// otherwise disagree about a download size whenever a value landed exactly on
/// a half. We adopt the Java rule because it is also what users expect.
fn round_half_away_from_zero(value: f64, digits: usize) -> f64 {
    let factor = 10f64.powi(digits as i32);
    let scaled = value * factor;
    // Past 2^53 an f64 has no fractional part left to round, and the
    // multiply/divide round-trip would only lose precision.
    if !scaled.is_finite() || scaled.abs() >= 9_007_199_254_740_992.0 {
        return value;
    }
    // `f64::round` is already half-away-from-zero.
    scaled.round() / factor
}

/// Render `value` with exactly `fraction_digits` decimals, grouped and using
/// `language`'s decimal separator.
///
/// Non-finite input yields `format.invalid_number`.
pub fn format_decimal(language: impl Into<Scope>, value: f64, fraction_digits: usize) -> String {
    let scope = language.into();
    if !value.is_finite() {
        return skeleton(&scope, "format.invalid_number", "—");
    }
    let digits = fraction_digits.min(MAX_FRACTION_DIGITS);
    let rendered = format!(
        "{:.*}",
        digits,
        round_half_away_from_zero(value.abs(), digits)
    );
    let (int_part, frac_part) = match rendered.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (rendered.as_str(), None),
    };

    let group = group_separator(&scope);
    // A blank *decimal* separator would fuse "1" and "5" into "15", so unlike the
    // grouping separator it always falls back.
    let decimal = skeleton(
        &scope,
        "format.decimal_separator",
        DEFAULT_DECIMAL_SEPARATOR,
    );

    // Only sign a value that survived rounding: -0.04 at 1 decimal is "0.0",
    // and "-0.0" in a UI looks like a bug.
    let significant = rendered.bytes().any(|b| b.is_ascii_digit() && b != b'0');
    let mut out = String::with_capacity(rendered.len() + 8);
    if value < 0.0 && significant {
        out.push('-');
    }
    out.push_str(&group_digits(int_part, &group));
    if let Some(frac) = frac_part {
        if !frac.is_empty() {
            out.push_str(&decimal);
            out.push_str(frac);
        }
    }
    out
}

/// Fraction digits used for a byte value at unit index `idx`.
///
/// Whole bytes are never fractional (`512 B`, not `512.0 B`); every larger unit
/// gets one decimal, which is what FCL and Android's own UIs show.
const fn byte_fraction_digits(idx: usize) -> usize {
    if idx == 0 {
        0
    } else {
        1
    }
}

/// Scale `bytes` into `(value, index into BYTE_UNIT_KEYS)` using 1024 steps.
fn scale_bytes(bytes: u64) -> (f64, usize) {
    let mut value = bytes as f64;
    let mut idx = 0usize;
    while value >= 1024.0 && idx + 1 < BYTE_UNIT_KEYS.len() {
        value /= 1024.0;
        idx += 1;
    }
    // Rounding to the display precision can push the value back up to 1024
    // (1023.97 KiB -> "1024.0 KB"); promote one more step so the unit is right.
    if idx + 1 < BYTE_UNIT_KEYS.len() {
        let factor = 10f64.powi(byte_fraction_digits(idx) as i32);
        if (value * factor).round() / factor >= 1024.0 {
            value /= 1024.0;
            idx += 1;
        }
    }
    (value, idx)
}

/// Render `bytes` through the `format.size` skeleton (`1.4 GB`).
pub fn format_bytes(language: impl Into<Scope>, bytes: u64) -> String {
    let scope = language.into();
    let (value, idx) = scale_bytes(bytes);
    let unit = skeleton(&scope, BYTE_UNIT_KEYS[idx], BYTE_UNIT_FALLBACK[idx]);
    let number = format_decimal(&scope, value, byte_fraction_digits(idx));
    let pattern = skeleton(&scope, "format.size", "{value} {unit}");
    format::interpolate(&pattern, &[("value", &number), ("unit", &unit)])
}

/// Render a transfer rate through the `format.rate` skeleton (`1.2 MB/秒`).
pub fn format_rate(language: impl Into<Scope>, bytes_per_second: u64) -> String {
    let scope = language.into();
    let (value, idx) = scale_bytes(bytes_per_second);
    let unit = skeleton(&scope, BYTE_UNIT_KEYS[idx], BYTE_UNIT_FALLBACK[idx]);
    let number = format_decimal(&scope, value, byte_fraction_digits(idx));
    let pattern = skeleton(&scope, "format.rate", "{value} {unit}/s");
    format::interpolate(&pattern, &[("value", &number), ("unit", &unit)])
}

/// `12.3 MB / 45.6 MB` — a byte-progress pair via `format.progress_of`.
pub fn format_byte_progress(language: impl Into<Scope>, done: u64, total: u64) -> String {
    let scope = language.into();
    let pattern = skeleton(&scope, "format.progress_of", "{done} / {total}");
    format::interpolate(
        &pattern,
        &[
            ("done", &format_bytes(&scope, done)),
            ("total", &format_bytes(&scope, total)),
        ],
    )
}

/// Render an already-scaled percentage (`42.5` -> `42.5%`).
pub fn format_percent(language: impl Into<Scope>, percent: f64, fraction_digits: usize) -> String {
    let scope = language.into();
    let number = format_decimal(&scope, percent, fraction_digits);
    let pattern = skeleton(&scope, "format.percent", "{value}%");
    format::interpolate(&pattern, &[("value", &number)])
}

/// Percentage of `done` out of `total`; a zero `total` is 0 % (never `NaN`).
pub fn format_ratio_percent(
    language: impl Into<Scope>,
    done: u64,
    total: u64,
    fraction_digits: usize,
) -> String {
    let scope = language.into();
    let percent = if total == 0 {
        0.0
    } else {
        done as f64 * 100.0 / total as f64
    };
    format_percent(&scope, percent, fraction_digits)
}

/// `59.9 FPS` — the AWT/renderer overlay's frame-rate readout.
pub fn format_fps(language: impl Into<Scope>, fps: f64) -> String {
    let scope = language.into();
    let number = format_decimal(&scope, fps, 1);
    let pattern = skeleton(&scope, "format.fps", "{value} FPS");
    format::interpolate(&pattern, &[("value", &number)])
}

/// One duration piece (`3 minutes`), plural-aware, with a compiled-in fallback.
///
/// Deliberately *not* [`super::t_plural_in`]: see [`DURATION_UNITS`].
fn duration_piece(scope: &Scope, base: &str, fallback: &str, count: i64) -> String {
    // The scope's own plural rule, so a pack declaring `_meta.plural = one_other`
    // pluralises correctly even though it is not a `Language` variant.
    let key = format!("{}.{}", base, scope.plural_rule().category(count).suffix());
    let template = match super::lookup_scoped(scope, &key) {
        Some(v) if !v.is_empty() => v,
        _ => fallback.to_string(),
    };
    let n = count.to_string();
    format::interpolate(&template, &[("count", n.as_str())])
}

/// Humanise a duration, showing at most [`DEFAULT_DURATION_PARTS`] units.
///
/// The sign is ignored (use [`format_relative_time`] for direction).
pub fn format_duration(language: impl Into<Scope>, seconds: i64) -> String {
    let scope = language.into();
    format_duration_parts(&scope, seconds, DEFAULT_DURATION_PARTS)
}

/// [`format_duration`] with an explicit cap on how many units to show.
///
/// Zero-valued units are skipped rather than terminating the walk, so 3605 s is
/// the honest `1 hour 5 seconds` instead of a lossy `1 hour`.
pub fn format_duration_parts(language: impl Into<Scope>, seconds: i64, max_parts: usize) -> String {
    let scope = language.into();
    let zero = || skeleton(&scope, "duration.zero", "0 s");
    let mut remaining = seconds.saturating_abs();
    if remaining == 0 || max_parts == 0 {
        return zero();
    }

    let mut parts: Vec<String> = Vec::with_capacity(max_parts.min(DURATION_UNITS.len()));
    for (size, base, fallback) in DURATION_UNITS {
        if parts.len() == max_parts {
            break;
        }
        let n = remaining / size;
        if n == 0 {
            continue;
        }
        remaining -= n * size;
        parts.push(duration_piece(&scope, base, fallback, n));
    }

    let pattern = skeleton(&scope, "format.duration_join", "{first} {second}");
    let mut it = parts.into_iter();
    let mut acc = match it.next() {
        Some(first) => first,
        None => return zero(),
    };
    for next in it {
        acc = format::interpolate(&pattern, &[("first", &acc), ("second", &next)]);
    }
    acc
}

/// Phrase a timestamp relative to now: `delta_seconds = now - timestamp`, so a
/// **positive** delta is in the past.
pub fn format_relative_time(language: impl Into<Scope>, delta_seconds: i64) -> String {
    let scope = language.into();
    if delta_seconds.saturating_abs() < RELATIVE_NOW_THRESHOLD {
        return skeleton(&scope, "relative.now", "just now");
    }
    let duration = format_duration(&scope, delta_seconds);
    let key = if delta_seconds > 0 {
        "relative.past"
    } else {
        "relative.future"
    };
    let pattern = skeleton(&scope, key, "{duration}");
    format::interpolate(&pattern, &[("duration", &duration)])
}

/// `剩余 3 分 20 秒` — a download ETA; negative input is treated as zero.
pub fn format_eta(language: impl Into<Scope>, seconds: i64) -> String {
    let scope = language.into();
    let duration = format_duration(&scope, seconds.max(0));
    let pattern = skeleton(&scope, "download.eta", "{duration}");
    format::interpolate(&pattern, &[("duration", &duration)])
}

/// Every catalogue key this module reads, for the consistency tests and for
/// `check_i18n.py` (which asserts the catalogue actually ships them).
pub fn required_keys() -> Vec<String> {
    let mut keys: Vec<String> = vec![
        "format.group_separator".into(),
        "format.decimal_separator".into(),
        "format.invalid_number".into(),
        "format.size".into(),
        "format.rate".into(),
        "format.percent".into(),
        "format.progress_of".into(),
        "format.duration_join".into(),
        "format.fps".into(),
        "duration.zero".into(),
        "relative.now".into(),
        "relative.past".into(),
        "relative.future".into(),
        "download.eta".into(),
    ];
    keys.extend(BYTE_UNIT_KEYS.iter().map(|k| (*k).to_string()));
    for (_, base, _) in DURATION_UNITS {
        keys.push(format!("{base}.one"));
        keys.push(format!("{base}.other"));
    }
    keys.sort();
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{catalog, Language};

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        super::super::GLOBAL_I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn every_key_this_module_reads_is_shipped_in_every_language() {
        let _g = lock();
        for language in Language::ALL {
            for key in required_keys() {
                assert!(
                    catalog::lookup(language, &key).is_some(),
                    "{} lacks {}",
                    language.tag(),
                    key
                );
            }
        }
    }

    #[test]
    fn groups_integers_by_three() {
        let _g = lock();
        assert_eq!(format_int(Language::En, 0), "0");
        assert_eq!(format_int(Language::En, 7), "7");
        assert_eq!(format_int(Language::En, 999), "999");
        assert_eq!(format_int(Language::En, 1_000), "1,000");
        assert_eq!(format_int(Language::En, 12_345), "12,345");
        assert_eq!(format_int(Language::En, 1_234_567), "1,234,567");
        assert_eq!(format_int(Language::En, -1_234_567), "-1,234,567");
        assert_eq!(format_uint(Language::ZhCn, 1_234_567), "1,234,567");
    }

    #[test]
    fn integer_extremes_saturate_instead_of_overflowing() {
        let _g = lock();
        // `abs()` on i64::MIN would panic in debug builds.
        assert_eq!(
            format_int(Language::En, i64::MIN),
            "-9,223,372,036,854,775,808"
        );
        assert_eq!(
            format_int(Language::En, i64::MAX),
            "9,223,372,036,854,775,807"
        );
        assert_eq!(
            format_uint(Language::En, u64::MAX),
            "18,446,744,073,709,551,615"
        );
    }

    #[test]
    fn decimals_round_and_use_the_locale_separators() {
        let _g = lock();
        assert_eq!(format_decimal(Language::En, 1.25, 1), "1.3");
        assert_eq!(format_decimal(Language::En, 1234.5678, 2), "1,234.57");
        assert_eq!(format_decimal(Language::En, 1234.5678, 0), "1,235");
        assert_eq!(format_decimal(Language::En, -12.34, 1), "-12.3");
    }

    #[test]
    fn rounds_half_away_from_zero_like_java() {
        let _g = lock();
        // Rust's own `{:.1}` would give "1.2"/"1.4" (half-to-even) here; the
        // Kotlin mirror uses String.format (half-up), so we must too.
        assert_eq!(format_decimal(Language::En, 1.25, 1), "1.3");
        assert_eq!(format_decimal(Language::En, 1.35, 1), "1.4");
        assert_eq!(format_decimal(Language::En, 2.5, 0), "3");
        assert_eq!(format_decimal(Language::En, 3.5, 0), "4");
        assert_eq!(format_decimal(Language::En, -2.5, 0), "-3");
        // Huge magnitudes have no fraction left: no precision-losing round-trip.
        let big = format_decimal(Language::En, 1e18, 2);
        assert!(big.starts_with("1,000,000,000,000,000,000"), "{big}");
    }

    #[test]
    fn negative_zero_is_never_signed() {
        let _g = lock();
        assert_eq!(format_decimal(Language::En, -0.04, 1), "0.0");
        assert_eq!(format_decimal(Language::En, -0.0, 1), "0.0");
        assert_eq!(format_decimal(Language::En, -0.0, 0), "0");
    }

    #[test]
    fn non_finite_numbers_degrade_to_a_placeholder() {
        let _g = lock();
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let out = format_decimal(Language::En, bad, 2);
            assert_eq!(out, "—", "{bad}");
            assert!(!format_percent(Language::En, bad, 1).contains("NaN"));
            assert!(!format_fps(Language::ZhCn, bad)
                .to_lowercase()
                .contains("inf"));
        }
    }

    #[test]
    fn fraction_digits_are_clamped() {
        let _g = lock();
        let out = format_decimal(Language::En, 1.5, usize::MAX);
        // Clamped to MAX_FRACTION_DIGITS, not a gigabyte of zeroes.
        assert_eq!(out.len(), "1.".len() + MAX_FRACTION_DIGITS);
    }

    #[test]
    fn bytes_scale_through_binary_units() {
        let _g = lock();
        assert_eq!(format_bytes(Language::En, 0), "0 B");
        assert_eq!(format_bytes(Language::En, 512), "512 B");
        assert_eq!(format_bytes(Language::En, 1_023), "1,023 B");
        assert_eq!(format_bytes(Language::En, 1_024), "1.0 KB");
        assert_eq!(format_bytes(Language::En, 1_536), "1.5 KB");
        assert_eq!(format_bytes(Language::En, 1_048_576), "1.0 MB");
        assert_eq!(format_bytes(Language::En, 1_500_000_000), "1.4 GB");
    }

    #[test]
    fn rounding_promotes_the_unit_instead_of_printing_1024() {
        let _g = lock();
        // 1023.97 KiB must not render as "1024.0 KB".
        let almost_a_mib = 1_048_540u64;
        let out = format_bytes(Language::En, almost_a_mib);
        assert_eq!(out, "1.0 MB", "{out}");
        // Same guard at the top of the ladder: never overflow past PB.
        let huge = format_bytes(Language::En, u64::MAX);
        assert!(huge.ends_with("PB"), "{huge}");
    }

    #[test]
    fn rates_and_progress_use_their_own_skeletons() {
        let _g = lock();
        assert_eq!(format_rate(Language::En, 1_258_291), "1.2 MB/s");
        assert_eq!(format_rate(Language::ZhCn, 1_258_291), "1.2 MB/秒");
        assert_eq!(
            format_byte_progress(Language::En, 1_048_576, 4_194_304),
            "1.0 MB / 4.0 MB"
        );
    }

    #[test]
    fn percentages_never_divide_by_zero() {
        let _g = lock();
        assert_eq!(format_percent(Language::En, 42.5, 1), "42.5%");
        assert_eq!(format_ratio_percent(Language::En, 1, 4, 0), "25%");
        assert_eq!(format_ratio_percent(Language::En, 0, 0, 1), "0.0%");
        assert_eq!(format_ratio_percent(Language::ZhCn, 3, 4, 1), "75.0%");
    }

    #[test]
    fn durations_are_plural_aware_per_language() {
        let _g = lock();
        assert_eq!(format_duration(Language::En, 1), "1 second");
        assert_eq!(format_duration(Language::En, 2), "2 seconds");
        assert_eq!(format_duration(Language::En, 60), "1 minute");
        assert_eq!(format_duration(Language::En, 200), "3 minutes 20 seconds");
        assert_eq!(format_duration(Language::En, 3_600), "1 hour");
        assert_eq!(format_duration(Language::En, 90_061), "1 day 1 hour");
        // Chinese has a single form and joins without a space.
        assert_eq!(format_duration(Language::ZhCn, 200), "3 分 20 秒");
        assert_eq!(format_duration(Language::ZhHant, 3_600), "1 小時");
    }

    #[test]
    fn durations_handle_zero_negative_and_part_limits() {
        let _g = lock();
        assert_eq!(format_duration(Language::En, 0), "0 s");
        assert_eq!(format_duration(Language::ZhCn, 0), "0 秒");
        // The sign is ignored; direction is `format_relative_time`'s job.
        assert_eq!(format_duration(Language::En, -200), "3 minutes 20 seconds");
        assert_eq!(
            format_duration(Language::En, i64::MIN),
            format_duration(Language::En, i64::MAX)
        );
        assert_eq!(format_duration_parts(Language::En, 200, 1), "3 minutes");
        assert_eq!(format_duration_parts(Language::En, 200, 0), "0 s");
        assert_eq!(
            format_duration_parts(Language::En, 90_061, 4),
            "1 day 1 hour 1 minute 1 second"
        );
        // Zero units are skipped, not treated as a terminator.
        assert_eq!(
            format_duration_parts(Language::En, 3_605, 2),
            "1 hour 5 seconds"
        );
    }

    #[test]
    fn relative_time_reads_in_both_directions() {
        let _g = lock();
        assert_eq!(format_relative_time(Language::En, 0), "just now");
        assert_eq!(format_relative_time(Language::En, 59), "just now");
        assert_eq!(format_relative_time(Language::En, -59), "just now");
        assert_eq!(format_relative_time(Language::En, 120), "2 minutes ago");
        assert_eq!(format_relative_time(Language::En, -120), "in 2 minutes");
        assert_eq!(format_relative_time(Language::ZhCn, 120), "2 分前");
        assert_eq!(format_relative_time(Language::ZhCn, -120), "2 分后");
        // i64::MIN must not overflow on `abs`.
        let _ = format_relative_time(Language::En, i64::MIN);
    }

    #[test]
    fn eta_clamps_negative_input() {
        let _g = lock();
        assert_eq!(
            format_eta(Language::En, 200),
            "3 minutes 20 seconds remaining"
        );
        assert_eq!(format_eta(Language::ZhCn, 200), "剩余 3 分 20 秒");
        assert_eq!(format_eta(Language::En, -5), "0 s remaining");
    }

    #[test]
    fn every_language_renders_every_helper_without_leaking_a_key() {
        let _g = lock();
        for language in Language::ALL {
            for out in [
                format_bytes(language, 1_536),
                format_rate(language, 1_536),
                format_percent(language, 12.5, 1),
                format_fps(language, 59.94),
                format_duration(language, 3_671),
                format_relative_time(language, 3_671),
                format_eta(language, 3_671),
                format_byte_progress(language, 1, 2),
            ] {
                assert!(!out.is_empty(), "{}", language.tag());
                assert!(!out.contains('{'), "unresolved placeholder: {out}");
                assert!(!out.contains("unit."), "leaked key: {out}");
                assert!(!out.contains("duration."), "leaked key: {out}");
                assert!(!out.contains("format."), "leaked key: {out}");
            }
        }
    }

    #[test]
    fn a_translator_can_relocalise_the_skeletons() {
        let _g = lock();
        catalog::clear_overlay();
        // A French-style overlay: space grouping, comma decimals, `octet`.
        super::super::install_overlay_text(
            Language::En,
            "format.group_separator = \\u0020\n\
             format.decimal_separator = ,\n\
             unit.kib = Kio\n\
             format.size = {value} {unit} !\n",
        );
        assert_eq!(format_int(Language::En, 1_234_567), "1 234 567");
        assert_eq!(format_bytes(Language::En, 1_536), "1,5 Kio !");
        catalog::clear_overlay();
        assert_eq!(format_bytes(Language::En, 1_536), "1.5 KB");
    }

    #[test]
    fn an_explicitly_empty_group_separator_disables_grouping() {
        let _g = lock();
        catalog::clear_overlay();
        // Present-but-empty means "this locale does not group digits"; a blank
        // *decimal* separator would fuse the digits, so that one still falls back.
        super::super::install_overlay_text(
            Language::En,
            "format.group_separator =\nformat.decimal_separator =\n",
        );
        assert_eq!(format_int(Language::En, 1_234_567), "1234567");
        assert_eq!(format_uint(Language::En, 1_234_567), "1234567");
        assert_eq!(format_decimal(Language::En, 1_234.5, 1), "1234.5");
        catalog::clear_overlay();
        assert_eq!(format_int(Language::En, 1_234_567), "1,234,567");
    }

    #[test]
    fn duration_pieces_never_leak_a_catalogue_key() {
        let _g = lock();
        catalog::clear_overlay();
        // Blank out the pieces the way a broken overlay could: the label must
        // degrade to the compiled-in terse form, never to `duration.minute.other`.
        super::super::install_overlay_text(
            Language::En,
            "duration.minute.other =\nduration.second.other =\nduration.zero =\n",
        );
        for out in [
            format_duration(Language::En, 200),
            format_eta(Language::En, 200),
            format_relative_time(Language::En, 200),
            format_duration(Language::En, 0),
        ] {
            assert!(!out.contains("duration."), "leaked a key: {out}");
            assert!(!out.contains('{'), "unresolved placeholder: {out}");
            assert!(!out.trim().is_empty());
        }
        assert_eq!(format_duration(Language::En, 200), "3 min 20 s");
        catalog::clear_overlay();
        assert_eq!(format_duration(Language::En, 200), "3 minutes 20 seconds");
    }

    #[test]
    fn an_empty_overlay_value_falls_back_to_the_compiled_default() {
        let _g = lock();
        catalog::clear_overlay();
        // A translator wiping a separator must not produce `1234567` silently
        // *or* an empty unit — the compiled-in default takes over.
        super::super::install_overlay_text(Language::En, "unit.mib =\nformat.size =\n");
        let out = format_bytes(Language::En, 1_048_576);
        assert_eq!(out, "1.0 MB", "{out}");
        catalog::clear_overlay();
    }
}
