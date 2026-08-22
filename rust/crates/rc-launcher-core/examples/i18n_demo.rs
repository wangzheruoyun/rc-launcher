//! End-to-end demo of the i18n framework (task 20).
//!
//! Exercises everything the launcher relies on, on the host, with no Android:
//!
//! ```bash
//! cargo run --example i18n_demo
//! ```
//!
//! 1. **中文优先** — an unknown device locale resolves to `zh-CN`.
//! 2. **Negotiation** — `zh-Hant-TW`, `zh_TW`, `zh-HK`, `en-GB`, `zh-Hans-TW`
//!    all land on the right catalogue.
//! 3. **Switching** — one call re-renders every string (what the settings picker does).
//! 4. **Interpolation & plurals** — `{name}` args and CLDR one/other.
//! 5. **Core integration** — a real crash verdict and a real `RcError` rendered
//!    in three languages from the same resource files.
//! 6. **Fallback** — a key only the base catalogue has still renders (in Chinese).
//! 7. **Overlay** — a community translation / wording hot-fix applied at runtime
//!    and then removed.
//! 8. **Bundle** — the payload the Compose UI hydrates itself from.

use rc_launcher::error::RcError;
use rc_launcher::i18n::{self, Language};
use rc_launcher::launch::crash::diagnose;

fn rule(title: &str) {
    println!("\n\x1b[1m== {} ==\x1b[0m", title);
}

fn main() {
    rule("1. Chinese-first defaults");
    for tag in ["ja-JP", "de", "", "und", "klingon"] {
        println!(
            "  device {:<10} -> {}  (unsupported: falls back to the base locale)",
            format!("{:?}", tag),
            i18n::Language::negotiate(tag)
                .unwrap_or(Language::BASE)
                .tag()
        );
    }
    assert_eq!(Language::BASE, Language::ZhCn);

    rule("2. Locale negotiation");
    for tag in [
        "zh",
        "zh-CN",
        "zh_CN.UTF-8",
        "zh-SG",
        "zh-Hans-TW",
        "zh-TW",
        "zh_TW",
        "zh-HK",
        "zh-Hant-CN",
        "yue",
        "en",
        "en-GB",
    ] {
        println!(
            "  {:<14} -> {}",
            tag,
            Language::negotiate(tag)
                .map(|l| l.tag())
                .unwrap_or("(none)")
        );
    }
    // An Android LocaleList: the first *supported* preference wins.
    assert_eq!(
        Language::negotiate_list(["ja-JP", "en-US", "zh-CN"]),
        Language::En
    );

    rule("3. Switching the UI language");
    for l in Language::ALL {
        i18n::set_language(l);
        println!(
            "  [{}] {} / {} / {}   ({})",
            l.tag(),
            i18n::t("nav.home"),
            i18n::t("nav.settings"),
            i18n::t("settings.section.language"),
            i18n::t("app.name"),
        );
    }

    rule("4. Placeholders and plural rules");
    for l in Language::ALL {
        println!(
            "  [{}] {}",
            l.tag(),
            i18n::t_args_in(
                l,
                "error.retry_scheduled",
                &[("attempt", "3"), ("delay_secs", "8")]
            )
        );
    }
    for n in [0, 1, 5] {
        println!(
            "  n={} en={:<9} zh-CN={}",
            n,
            i18n::t_plural_in(Language::En, "download.files", n),
            i18n::t_plural_in(Language::ZhCn, "download.files", n),
        );
    }

    rule("5. Core-generated text (crash verdict + error)");
    let report = diagnose(
        Some(1),
        None,
        ["java.lang.OutOfMemoryError: Java heap space"],
        false,
    );
    for l in Language::ALL {
        println!(
            "  [{}] {} — {}",
            l.tag(),
            report.category.localized_summary(l),
            report.category.localized_advice(l),
        );
    }
    let err = RcError::ChecksumMismatch {
        path: "/sdcard/rc/versions/1.20.1/1.20.1.jar".into(),
        expected: "abc".into(),
        actual: "def".into(),
    };
    for l in Language::ALL {
        println!(
            "  [{}] {}  [{}]",
            l.tag(),
            err.localized(l),
            err.severity_label(l)
        );
    }
    // The developer-facing Display is unchanged, whatever the UI language is.
    println!("  log     : {}", err);

    rule("6. Fallback for an untranslated key");
    i18n::clear_overlay();
    i18n::install_overlay_text(Language::ZhCn, "brand.new.feature = 全新功能（尚未翻译）\n");
    for l in Language::ALL {
        println!("  [{}] {}", l.tag(), i18n::t_in(l, "brand.new.feature"));
    }
    assert_eq!(
        i18n::t_in(Language::En, "brand.new.feature"),
        i18n::t_in(Language::ZhCn, "brand.new.feature"),
        "an untranslated key must degrade to Chinese, never to a raw key"
    );
    i18n::clear_overlay();

    rule("7. Runtime overlay (hot-fix without a new APK)");
    println!("  before: {}", i18n::t_in(Language::En, "nav.home"));
    i18n::install_overlay_text(Language::En, "nav.home = Dashboard\n");
    println!("  after : {}", i18n::t_in(Language::En, "nav.home"));
    assert_eq!(i18n::t_in(Language::En, "nav.home"), "Dashboard");
    // Other languages are untouched.
    assert_eq!(i18n::t_in(Language::ZhCn, "nav.home"), "主页");
    i18n::clear_overlay();
    println!("  reset : {}", i18n::t_in(Language::En, "nav.home"));

    rule("8. The bundle Compose hydrates from");
    for l in Language::ALL {
        let b = i18n::bundle(l);
        println!(
            "  [{}] {} messages, {:.0}% translated, e.g. nav.settings = {:?}",
            l.tag(),
            b.len(),
            i18n::completeness(l) * 100.0,
            b["nav.settings"],
        );
    }
    let diag = i18n::diagnostics();
    println!(
        "  diagnostics: {} keys, overlay_active={}, languages={}",
        diag["total_keys"],
        diag["overlay_active"],
        diag["languages"].as_array().map(|a| a.len()).unwrap_or(0),
    );
    for lang in diag["languages"].as_array().unwrap() {
        assert!(lang["missing_keys"].as_array().unwrap().is_empty());
        assert!(lang["placeholder_mismatch"].as_array().unwrap().is_empty());
    }

    i18n::set_language(Language::BASE);
    println!("\n\x1b[32mAll i18n invariants held.\x1b[0m");
}
