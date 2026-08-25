package com.rc.launcher.ui.i18n

/**
 * The i18n keys the Compose UI references (task 20).
 *
 * Constants rather than string literals at the call sites so that
 *  * a renamed key breaks the *compile*, not the UI at runtime,
 *  * [required] can be checked against the shipped catalogues by a unit test,
 *  * and the keys stay identical to the Rust core's
 *    `rust/crates/rc-launcher-core/i18n/<tag>.properties` (the source of truth).
 *
 * Keys use dots; the generated Android resource names use underscores — see
 * [RcStringResources] and `scripts/gen_android_strings.py`.
 */
object RcStringKeys {
    // --- app ---
    const val APP_NAME = "app.name"
    const val APP_TAGLINE = "app.tagline"

    // --- common actions ---
    const val COMMON_OK = "common.ok"
    const val COMMON_CANCEL = "common.cancel"
    const val COMMON_SAVE = "common.save"
    const val COMMON_DELETE = "common.delete"
    const val COMMON_RETRY = "common.retry"
    const val COMMON_BACK = "common.back"
    const val COMMON_NEXT = "common.next"
    const val COMMON_PREVIOUS = "common.previous"
    const val COMMON_CLOSE = "common.close"
    const val COMMON_EDIT = "common.edit"
    const val COMMON_ADD = "common.add"
    const val COMMON_REFRESH = "common.refresh"
    const val COMMON_APPLY = "common.apply"
    const val COMMON_LOADING = "common.loading"
    const val COMMON_UNAVAILABLE = "common.unavailable"
    const val COMMON_DEFAULT = "common.default"

    // --- bottom navigation ---
    const val NAV_HOME = "nav.home"
    const val NAV_INSTANCES = "nav.instances"
    const val NAV_DOWNLOADS = "nav.downloads"
    const val NAV_SETTINGS = "nav.settings"
    const val NAV_ACCOUNTS = "nav.accounts"

    // --- screen titles ---
    const val SCREEN_INSTANCE_DETAIL = "screen.instance_detail.title"
    const val SCREEN_INSTALL = "screen.install.title"
    const val SCREEN_CONTROLLER = "screen.controller.title"
    const val SCREEN_AWT = "screen.awt.title"

    // --- theme / night mode ---
    const val THEME_NIGHT_TOGGLE = "theme.night.toggle"
    const val THEME_NIGHT_SYSTEM = "theme.night.system"
    const val THEME_NIGHT_LIGHT = "theme.night.light"
    const val THEME_NIGHT_DARK = "theme.night.dark"

    // --- settings sections ---
    const val SETTINGS_SECTION_APPEARANCE = "settings.section.appearance"
    const val SETTINGS_SECTION_LANGUAGE = "settings.section.language"
    const val SETTINGS_SECTION_NETWORK = "settings.section.network"
    const val SETTINGS_SECTION_JAVA = "settings.section.java"
    const val SETTINGS_SECTION_RENDERER = "settings.section.renderer"
    const val SETTINGS_SECTION_CONTROLLER = "settings.section.controller"
    const val SETTINGS_SECTION_DIRECTORY = "settings.section.directory"
    const val SETTINGS_SECTION_ABOUT = "settings.section.about"

    // --- language settings ---
    const val SETTINGS_LANGUAGE_TITLE = "settings.language.title"
    const val SETTINGS_LANGUAGE_SUBTITLE = "settings.language.subtitle"
    const val SETTINGS_LANGUAGE_FOLLOW_SYSTEM = "settings.language.follow_system"
    /** Carries a `{language}` placeholder. */
    const val SETTINGS_LANGUAGE_APPLIED = "settings.language.applied"
    const val LANGUAGE_SYSTEM = "language.system"
    const val LANGUAGE_ZH_CN = "language.zh_cn"
    const val LANGUAGE_ZH_HANT = "language.zh_hant"
    const val LANGUAGE_EN = "language.en"

    // --- launch lifecycle ---
    const val LAUNCH_STATE_IDLE = "launch.state.idle"
    const val LAUNCH_STATE_PREPARING = "launch.state.preparing"
    const val LAUNCH_STATE_LAUNCHING = "launch.state.launching"
    const val LAUNCH_STATE_RUNNING = "launch.state.running"
    const val LAUNCH_STATE_STOPPED = "launch.state.stopped"
    const val LAUNCH_STATE_CRASHED = "launch.state.crashed"

    /** Plural *base* key — use [RcStrings.plural], never this key directly. */
    const val DOWNLOAD_FILES = "download.files"

    /** The i18n key of a language's own name in the picker. */
    fun nameKeyOf(language: AppLanguage): String = when (language) {
        AppLanguage.SYSTEM -> LANGUAGE_SYSTEM
        AppLanguage.ZH_CN -> LANGUAGE_ZH_CN
        AppLanguage.ZH_HANT -> LANGUAGE_ZH_HANT
        AppLanguage.EN -> LANGUAGE_EN
    }

    /**
     * Every key the UI needs. A unit test asserts each one exists in each shipped
     * catalogue, so a typo or a deleted translation fails the build instead of
     * showing a raw key to the user.
     */
    val required: List<String> = listOf(
        APP_NAME, APP_TAGLINE,
        COMMON_OK, COMMON_CANCEL, COMMON_SAVE, COMMON_DELETE, COMMON_RETRY,
        COMMON_BACK, COMMON_NEXT, COMMON_PREVIOUS, COMMON_CLOSE, COMMON_EDIT,
        COMMON_ADD, COMMON_REFRESH, COMMON_APPLY, COMMON_LOADING,
        COMMON_UNAVAILABLE, COMMON_DEFAULT,
        NAV_HOME, NAV_INSTANCES, NAV_DOWNLOADS, NAV_SETTINGS, NAV_ACCOUNTS,
        SCREEN_INSTANCE_DETAIL, SCREEN_INSTALL, SCREEN_CONTROLLER, SCREEN_AWT,
        THEME_NIGHT_TOGGLE, THEME_NIGHT_SYSTEM, THEME_NIGHT_LIGHT, THEME_NIGHT_DARK,
        SETTINGS_SECTION_APPEARANCE, SETTINGS_SECTION_LANGUAGE,
        SETTINGS_SECTION_NETWORK, SETTINGS_SECTION_JAVA, SETTINGS_SECTION_RENDERER,
        SETTINGS_SECTION_CONTROLLER, SETTINGS_SECTION_DIRECTORY,
        SETTINGS_SECTION_ABOUT,
        SETTINGS_LANGUAGE_TITLE, SETTINGS_LANGUAGE_SUBTITLE,
        SETTINGS_LANGUAGE_FOLLOW_SYSTEM, SETTINGS_LANGUAGE_APPLIED,
        LANGUAGE_SYSTEM, LANGUAGE_ZH_CN, LANGUAGE_ZH_HANT, LANGUAGE_EN,
        LAUNCH_STATE_IDLE, LAUNCH_STATE_PREPARING, LAUNCH_STATE_LAUNCHING,
        LAUNCH_STATE_RUNNING, LAUNCH_STATE_STOPPED, LAUNCH_STATE_CRASHED,
        "$DOWNLOAD_FILES.one", "$DOWNLOAD_FILES.other",
    ) + RcValueFormat.requiredKeys()
}
