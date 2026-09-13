//! Embedded Microsoft OAuth callback page (task 28).
//!
//! When a custom `redirect_uri` is configured for the Microsoft device-code flow,
//! Microsoft redirects the user's browser to that address after they complete
//! sign-in. On Android this is typically `file:///android_asset/microsoft_auth.html`
//! — a local HTML page bundled in the APK's `assets/` directory.
//!
//! The APK may not always include this asset (e.g. a stripped build or a build
//! that forgot to package it), so the page content is compiled into the Rust
//! core as [`MICROSOFT_AUTH_HTML`]. The Kotlin side can call
//! `RustBridge.authGetCallbackHtml()` at first launch and write the result to
//! `assets/microsoft_auth.html`, guaranteeing the callback page always exists.
//!
//! See also [`DEFAULT_REDIRECT_URI`].

/// The default redirect URI for the embedded callback page.
///
/// Microsoft OAuth accepts this as the `redirect_uri` parameter on the device-code
/// endpoint; after the user completes sign-in in the browser, Microsoft redirects
/// to this address and the embedded HTML page shows a "you may close this" message.
pub const DEFAULT_REDIRECT_URI: &str = "file:///android_asset/microsoft_auth.html";

/// The embedded `microsoft_auth.html` callback page template.
///
/// Placeholders (filled by [`callback_html`]):
/// * `%lang%`  — BCP-47 language tag for the `<html lang>` attribute.
/// * `%title%` — page `<title>`.
/// * `%message%` — instructional text shown to the user.
/// * `%close-button%` — the close-button label (or empty for auto-close only).
///
/// The page auto-closes after 5 s via `window.close()` and also offers a manual
/// close button so users who don't see the auto-close still get out.
pub const MICROSOFT_AUTH_HTML: &str = r#"<!DOCTYPE html>
<html lang="%lang%">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>%title%</title>
    <style>
        body { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
               display: flex; align-items: center; justify-content: center;
               min-height: 100vh; margin: 0; background: #f5f5f5; text-align: center; }
        .card { background: #fff; padding: 32px 24px; border-radius: 12px;
                box-shadow: 0 2px 12px rgba(0,0,0,0.08); max-width: 320px; }
        h1 { font-size: 20px; margin: 0 0 8px; color: #1a1a1a; }
        p { font-size: 14px; margin: 0 0 16px; color: #666; }
        button { padding: 10px 20px; font-size: 14px; border: none; border-radius: 8px;
                 background: #1976d2; color: #fff; cursor: pointer; }
        button:hover { background: #1565c0; }
    </style>
</head>
<body>
    <div class="card">
        <h1>%heading%</h1>
        <p>%message%</p>
        %close-button%
    </div>
    <script>
        // Auto-close after 5 s so the user returns to the launcher promptly.
        setTimeout(function() {
            try { open("about:blank","_self").close(); } catch(e) {}
        }, 5000);
    </script>
</body>
</html>
"#;

/// Fill in the [`MICROSOFT_AUTH_HTML`] template with translated text for the
/// given `language`. Returns ready-to-write HTML content.
///
/// * `success` — when `true` shows a success heading + message; when `false`
///   shows an error heading + message (e.g. the user cancelled sign-in).
pub fn callback_html(language: crate::i18n::Language, success: bool) -> String {
    let lang = language.tag();
    let title = crate::i18n::t_in(language, "auth.callback.title");
    let heading = if success {
        crate::i18n::t_in(language, "auth.callback.successHeading")
    } else {
        crate::i18n::t_in(language, "auth.callback.errorHeading")
    };
    let message = if success {
        crate::i18n::t_in(language, "auth.callback.successMessage")
    } else {
        crate::i18n::t_in(language, "auth.callback.errorMessage")
    };
    let close_btn = format!(
        "<button onclick=\"open('about:blank','_self').close();\">{}</button>",
        crate::i18n::t_in(language, "auth.callback.closeButton")
    );
    MICROSOFT_AUTH_HTML
        .replace("%lang%", lang)
        .replace("%title%", &title)
        .replace("%heading%", &heading)
        .replace("%message%", &message)
        .replace("%close-button%", &close_btn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn default_redirect_uri_is_android_asset() {
        assert_eq!(
            DEFAULT_REDIRECT_URI,
            "file:///android_asset/microsoft_auth.html"
        );
    }

    #[test]
    fn html_template_has_placeholders() {
        assert!(MICROSOFT_AUTH_HTML.contains("%lang%"));
        assert!(MICROSOFT_AUTH_HTML.contains("%title%"));
        assert!(MICROSOFT_AUTH_HTML.contains("%message%"));
        assert!(MICROSOFT_AUTH_HTML.contains("%close-button%"));
        assert!(MICROSOFT_AUTH_HTML.contains("setTimeout"));
        assert!(
            MICROSOFT_AUTH_HTML.contains("window.close")
                || MICROSOFT_AUTH_HTML.contains(".close()")
        );
    }

    #[test]
    fn callback_html_substitutes_placeholders() {
        let _g = crate::i18n::GLOBAL_I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let restore = crate::i18n::current_language();
        crate::i18n::set_language(Language::En);
        let html = callback_html(Language::En, true);
        assert!(!html.contains("%lang%"));
        assert!(!html.contains("%title%"));
        assert!(!html.contains("%message%"));
        assert!(!html.contains("%close-button%"));
        assert!(html.contains("Sign-in complete"));
        assert!(html.contains("Close"));
        crate::i18n::set_language(restore);
    }

    #[test]
    fn callback_html_error_mode() {
        let _g = crate::i18n::GLOBAL_I18N_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let restore = crate::i18n::current_language();
        crate::i18n::set_language(Language::En);
        let html = callback_html(Language::En, false);
        assert!(html.contains("Sign-in failed"));
        assert!(html.contains("return to the launcher"));
        crate::i18n::set_language(restore);
    }
}
