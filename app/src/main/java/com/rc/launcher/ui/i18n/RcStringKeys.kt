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
    const val COMMON_OPEN_IN_BROWSER = "common.open_in_browser"

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

    // --- custom launcher background (task 11) ---
    const val BACKGROUND_TITLE = "background.title"
    const val BACKGROUND_ENABLE = "background.enable"
    const val BACKGROUND_PICK = "background.pick"
    const val BACKGROUND_NONE_SELECTED = "background.none_selected"
    const val BACKGROUND_INVALID = "background.invalid"
    const val BACKGROUND_EFFECT = "background.effect"
    const val BACKGROUND_EFFECT_NONE = "background.effect.none"
    const val BACKGROUND_EFFECT_BLUR = "background.effect.blur"
    const val BACKGROUND_EFFECT_DARKEN = "background.effect.darken"
    const val BACKGROUND_EFFECT_BLUR_DARKEN = "background.effect.blur_darken"
    const val BACKGROUND_BLUR = "background.blur"
    const val BACKGROUND_DARKEN = "background.darken"
    const val BACKGROUND_FOLLOW_THEME = "background.follow_theme"
    const val BACKGROUND_FOLLOW_THEME_SUMMARY = "background.follow_theme.summary"

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
        COMMON_UNAVAILABLE, COMMON_DEFAULT, COMMON_OPEN_IN_BROWSER,
        NAV_HOME, NAV_INSTANCES, NAV_DOWNLOADS, NAV_SETTINGS, NAV_ACCOUNTS,
        SCREEN_INSTANCE_DETAIL, SCREEN_INSTALL, SCREEN_CONTROLLER, SCREEN_AWT,
        THEME_NIGHT_TOGGLE, THEME_NIGHT_SYSTEM, THEME_NIGHT_LIGHT, THEME_NIGHT_DARK,
        BACKGROUND_TITLE, BACKGROUND_ENABLE, BACKGROUND_PICK, BACKGROUND_NONE_SELECTED,
        BACKGROUND_INVALID, BACKGROUND_EFFECT, BACKGROUND_EFFECT_NONE,
        BACKGROUND_EFFECT_BLUR, BACKGROUND_EFFECT_DARKEN, BACKGROUND_EFFECT_BLUR_DARKEN,
        BACKGROUND_BLUR, BACKGROUND_DARKEN, BACKGROUND_FOLLOW_THEME,
        BACKGROUND_FOLLOW_THEME_SUMMARY,
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
    ) + RcValueFormat.requiredKeys() + listOf(
        TRANSLATE_TITLE, TRANSLATE_SUBTITLE, TRANSLATE_ENABLE,
        TRANSLATE_SHOW_ORIGINAL, TRANSLATE_SHOW_TRANSLATED,
        TRANSLATE_LANGUAGE, TRANSLATE_LANGUAGE_AUTO,
        TRANSLATE_LANGUAGE_ZH_CN, TRANSLATE_LANGUAGE_ZH_HANT,
        TRANSLATE_LANGUAGE_EN,
        TRANSLATE_MODE, TRANSLATE_MODE_HYBRID, TRANSLATE_MODE_ONLINE, TRANSLATE_MODE_OFFLINE,
        TRANSLATE_SOURCE, TRANSLATE_SOURCE_PASSTHROUGH,
        TRANSLATE_SOURCE_DICTIONARY, TRANSLATE_SOURCE_CACHE,
        TRANSLATE_SOURCE_GATEWAY, TRANSLATE_SOURCE_UNAVAILABLE,
        TRANSLATE_REFRESH, TRANSLATE_BATCH, TRANSLATE_BATCH_SUMMARY,
        TRANSLATE_EMPTY,
        TRANSLATE_GATEWAY_TITLE, TRANSLATE_GATEWAY_URL, TRANSLATE_GATEWAY_MODEL,
        TRANSLATE_GATEWAY_AUTH, TRANSLATE_GATEWAY_SYSTEM_PROMPT,
        TRANSLATE_CACHE_TITLE, TRANSLATE_CACHE_CLEAR,
        TRANSLATE_CACHE_COUNT, TRANSLATE_CACHE_SIZE,
        TRANSLATE_NETWORK_FALLBACK, TRANSLATE_NETWORK_FALLBACK_SUMMARY,
        TRANSLATE_CN_HINT,
        TUTORIAL_TITLE, TUTORIAL_WELCOME_TITLE, TUTORIAL_WELCOME_BODY,
        TUTORIAL_STEP_ACCOUNT_TITLE, TUTORIAL_STEP_ACCOUNT_BODY,
        TUTORIAL_STEP_INSTANCE_TITLE, TUTORIAL_STEP_INSTANCE_BODY,
        TUTORIAL_STEP_RENDERER_TITLE, TUTORIAL_STEP_RENDERER_BODY,
        TUTORIAL_STEP_IMPORT_TITLE, TUTORIAL_STEP_IMPORT_BODY,
        TUTORIAL_STEP_DONE_TITLE, TUTORIAL_STEP_DONE_BODY,
        TUTORIAL_SKIP, TUTORIAL_NEXT, TUTORIAL_PREVIOUS, TUTORIAL_FINISH,
        TUTORIAL_REWATCH, TUTORIAL_STEP_INDICATOR,
        TUTORIAL_REPLAY_BUTTON, TUTORIAL_REPLAY_SUMMARY,
        SCREEN_FILE_MANAGER, FILE_MANAGER_NEW_FOLDER, FILE_MANAGER_IMPORT,
        FILE_MANAGER_DELETE, FILE_MANAGER_REFRESH, FILE_MANAGER_BACK,
        FILE_MANAGER_UP, FILE_MANAGER_MORE, FILE_MANAGER_OPEN,
        FILE_MANAGER_RENAME, FILE_MANAGER_COPY, FILE_MANAGER_EXTRACT_HERE,
        FILE_MANAGER_CONFIRM_DELETE_TITLE, FILE_MANAGER_CONFIRM_DELETE_BODY_DIRS,
        FILE_MANAGER_CONFIRM_DELETE_BODY, FILE_MANAGER_CONFIRM_DELETE_IRREVERSIBLE,
        FILE_MANAGER_CONFIRM_OVERWRITE_TITLE, FILE_MANAGER_CONFIRM_OVERWRITE_BODY,
        FILE_MANAGER_DIALOG_CREATE_TITLE, FILE_MANAGER_DIALOG_CREATE_LABEL,
        FILE_MANAGER_DIALOG_RENAME_TITLE, FILE_MANAGER_DIALOG_RENAME_LABEL,
        FILE_MANAGER_DIALOG_COPY_TITLE, FILE_MANAGER_DIALOG_COPY_LABEL,
        FILE_MANAGER_EMPTY, FILE_MANAGER_KIND_DIR, FILE_MANAGER_KIND_FILE,
        FILE_MANAGER_KIND_LINK,
        // --- task 20: in-game floating HUD ---
        HUD_MENU_TITLE,
        HUD_ACTION_OPEN_LOG, HUD_ACTION_LOG_PASSTHROUGH, HUD_ACTION_SWITCH_INPUT,
        HUD_ACTION_SCREENSHOT, HUD_ACTION_FORCE_QUIT,
        HUD_SETTING_TITLE, HUD_SETTING_OPACITY, HUD_SETTING_AUTO_HIDE,
        HUD_SETTING_AUTO_HIDE_SUMMARY, HUD_SETTING_ANTI_MISOPERATION,
        HUD_SETTING_ANTI_MISOPERATION_SUMMARY,
        HUD_INPUT_MODE_TOUCH, HUD_INPUT_MODE_MOUSE,
        HUD_EXPAND_CONTENT_DESCRIPTION, HUD_COLLAPSE_CONTENT_DESCRIPTION,
        HUD_DRAG_CONTENT_DESCRIPTION, HUD_CLOSE_CONTENT_DESCRIPTION,
        // --- task 21: expanded log viewer ---
        HUD_LOG_OVERLAY_TITLE, HUD_LOG_PASSTHROUGH, HUD_LOG_PASSTHROUGH_SUMMARY,
        HUD_LOG_FILTER_ALL, HUD_LOG_FILTER_STDOUT, HUD_LOG_FILTER_STDERR,
        HUD_LOG_FILTER_ERRORS, HUD_LOG_FILTER_WARN,
        HUD_LOG_SEARCH_PLACEHOLDER,
        HUD_LOG_EXPORT, HUD_LOG_EXPORTING, HUD_LOG_EXPORT_DONE, HUD_LOG_EXPORT_FAILED,
        HUD_LOG_SNAPSHOT, HUD_LOG_CLEAR,
        HUD_LOG_LEVEL_INFO, HUD_LOG_LEVEL_WARN, HUD_LOG_LEVEL_ERROR,
        HUD_LOG_LEVEL_DEBUG, HUD_LOG_LEVEL_TRACE, HUD_LOG_LEVEL_FATAL,
        HUD_LOG_LEVEL_STDOUT, HUD_LOG_LEVEL_STDERR, HUD_LOG_LEVEL_UNKNOWN,
        HUD_CRASH_EXIT_CODE, HUD_CRASH_SIGNAL, HUD_CRASH_NA,
        // --- task 22: skin preview labels ---
        SKIN_PREVIEW_TITLE, SKIN_SELECT_MODEL, SKIN_FETCHING, SKIN_UPLOADING,
        SKIN_PICK_LOCAL, SKIN_UPLOAD, SKIN_UPLOADING_LABEL,
        SKIN_VIEW_IN_BROWSER,
        SKIN_MODEL_STEVE, SKIN_MODEL_ALEX,
        SKIN_SOURCE_OFFICIAL, SKIN_SOURCE_CUSTOM,
        SKIN_HAS_CAPE, SKIN_CACHED_OFFLINE,
        SKIN_UPLOAD_CONFIRM, SKIN_UPLOAD_CANCEL, SKIN_UPLOAD_NOTE,
        SKIN_INVALID_DIMENSIONS, SKIN_NO_TEXTURE,
        SKIN_READ_ERROR,
        SKIN_OFFLINE_ACCOUNT,
        // --- task 23: skin import tutorial ---
        SKIN_TUTORIAL_TITLE, SKIN_TUTORIAL_STEP_INDICATOR,
        SKIN_TUTORIAL_STEP_GET_TITLE, SKIN_TUTORIAL_STEP_GET_BODY,
        SKIN_TUTORIAL_STEP_FORMAT_TITLE, SKIN_TUTORIAL_STEP_FORMAT_BODY,
        SKIN_TUTORIAL_STEP_IMPORT_TITLE, SKIN_TUTORIAL_STEP_IMPORT_BODY,
        SKIN_TUTORIAL_STEP_APPLY_TITLE, SKIN_TUTORIAL_STEP_APPLY_BODY,
        SKIN_TUTORIAL_STEP_DONE_TITLE, SKIN_TUTORIAL_STEP_DONE_BODY,
        SKIN_TUTORIAL_START, SKIN_TUTORIAL_REWATCH, SKIN_TUTORIAL_SKIP, SKIN_TUTORIAL_NEXT,
        SKIN_TUTORIAL_PREVIOUS, SKIN_TUTORIAL_FINISH, SKIN_TUTORIAL_PICK,
        SKIN_TUTORIAL_OFFICIAL_NOTE, SKIN_TUTORIAL_THIRDPARTY_NOTE,
        SKIN_TUTORIAL_FORMAT_NOTE, SKIN_TUTORIAL_LINK_TASK14,
        // --- task 25/26/27: mod expand/collapse, world & pack management ---
        MOD_EXPAND, MOD_COLLAPSE, MOD_DESCRIPTION, MOD_VERSION, MOD_DEPENDENCIES,
        SCREEN_WORLD_MANAGER_TITLE, WORLD_PLAY_TIME, WORLD_GAME_MODE,
        WORLD_LAST_MODIFIED, WORLD_BACKUP, WORLD_RECOVER, WORLD_RENAME,
        WORLD_DELETE, WORLD_EXPORT, WORLD_IMPORT, WORLD_THUMBNAIL,
        WORLD_EMPTY, WORLD_HAS_BACKUP, WORLD_BACKUP_DONE, WORLD_BACKUP_FAILED,
        WORLD_RECOVER_DONE, WORLD_RECOVER_FAILED, WORLD_RENAME_DONE,
        WORLD_RENAME_FAILED, WORLD_DELETE_DONE, WORLD_DELETE_FAILED,
        WORLD_EXPORT_DONE, WORLD_EXPORT_FAILED, WORLD_IMPORT_DONE,
        WORLD_IMPORT_FAILED, WORLD_CONFIRM_DELETE_TITLE, WORLD_CONFIRM_DELETE_BODY,
        WORLD_CONFIRM_RENAME_TITLE, WORLD_CONFIRM_RENAME_LABEL,
        SCREEN_RESOURCE_PACK_TITLE, SCREEN_SHADER_PACK_TITLE,
        RESOURCE_PACK_ENABLE, RESOURCE_PACK_DISABLE, RESOURCE_PACK_PREVIEW,
        RESOURCE_PACK_IMPORT, RESOURCE_PACK_DELETE, RESOURCE_PACK_DOWNLOAD,
        RESOURCE_PACK_EMPTY, RESOURCE_PACK_PACK_FORMAT, RESOURCE_PACK_COMPATIBLE,
        RESOURCE_PACK_INCOMPATIBLE, RESOURCE_PACK_DESCRIPTION,
        RESOURCE_PACK_DELETE_CONFIRM,
        SHADER_PACK_ENABLE, SHADER_PACK_DISABLE, SHADER_PACK_PREVIEW,
        SHADER_PACK_IMPORT, SHADER_PACK_DELETE, SHADER_PACK_DOWNLOAD,
        SHADER_PACK_EMPTY, SHADER_PACK_VALID, SHADER_PACK_INVALID,
        SHADER_PACK_DELETE_CONFIRM,
    )
}

    // --- task 13: mod browser inline translation ---
    const val TRANSLATE_TITLE = "translate.title"
    const val TRANSLATE_SUBTITLE = "translate.subtitle"
    const val TRANSLATE_ENABLE = "translate.enable"
    const val TRANSLATE_SHOW_ORIGINAL = "translate.show_original"
    const val TRANSLATE_SHOW_TRANSLATED = "translate.show_translated"
    const val TRANSLATE_LANGUAGE = "translate.language"
    const val TRANSLATE_LANGUAGE_AUTO = "translate.language.auto"
    const val TRANSLATE_LANGUAGE_ZH_CN = "translate.language.zh_cn"
    const val TRANSLATE_LANGUAGE_ZH_HANT = "translate.language.zh_hant"
    const val TRANSLATE_LANGUAGE_EN = "translate.language.en"
    const val TRANSLATE_MODE = "translate.mode"
    const val TRANSLATE_MODE_HYBRID = "translate.mode.hybrid"
    const val TRANSLATE_MODE_ONLINE = "translate.mode.online"
    const val TRANSLATE_MODE_OFFLINE = "translate.mode.offline"
    const val TRANSLATE_SOURCE = "translate.source"
    const val TRANSLATE_SOURCE_PASSTHROUGH = "translate.source.passthrough"
    const val TRANSLATE_SOURCE_DICTIONARY = "translate.source.dictionary"
    const val TRANSLATE_SOURCE_CACHE = "translate.source.cache"
    const val TRANSLATE_SOURCE_GATEWAY = "translate.source.gateway"
    const val TRANSLATE_SOURCE_UNAVAILABLE = "translate.source.unavailable"
    const val TRANSLATE_REFRESH = "translate.refresh"
    const val TRANSLATE_BATCH = "translate.batch"
    /** Carries a `{count}` placeholder. */
    const val TRANSLATE_BATCH_SUMMARY = "translate.batch_summary"
    const val TRANSLATE_EMPTY = "translate.empty"
    const val TRANSLATE_GATEWAY_TITLE = "translate.gateway.title"
    const val TRANSLATE_GATEWAY_URL = "translate.gateway.url"
    const val TRANSLATE_GATEWAY_MODEL = "translate.gateway.model"
    const val TRANSLATE_GATEWAY_AUTH = "translate.gateway.auth"
    const val TRANSLATE_GATEWAY_SYSTEM_PROMPT = "translate.gateway.system_prompt"
    const val TRANSLATE_CACHE_TITLE = "translate.cache.title"
    const val TRANSLATE_CACHE_CLEAR = "translate.cache.clear"
    /** Carries a `{count}` placeholder. */
    const val TRANSLATE_CACHE_COUNT = "translate.cache.count"
    /** Carries a `{bytes}` placeholder. */
    const val TRANSLATE_CACHE_SIZE = "translate.cache.size"
    const val TRANSLATE_NETWORK_FALLBACK = "translate.network_fallback"
    const val TRANSLATE_NETWORK_FALLBACK_SUMMARY = "translate.network_fallback.summary"
    const val TRANSLATE_CN_HINT = "translate.cn_hint"

    // --- task 14: newbie tutorial & onboarding flow ---
    const val TUTORIAL_TITLE = "tutorial.title"
    const val TUTORIAL_WELCOME_TITLE = "tutorial.welcome.title"
    const val TUTORIAL_WELCOME_BODY = "tutorial.welcome.body"
    const val TUTORIAL_STEP_ACCOUNT_TITLE = "tutorial.step.account.title"
    const val TUTORIAL_STEP_ACCOUNT_BODY = "tutorial.step.account.body"
    const val TUTORIAL_STEP_INSTANCE_TITLE = "tutorial.step.instance.title"
    const val TUTORIAL_STEP_INSTANCE_BODY = "tutorial.step.instance.body"
    const val TUTORIAL_STEP_RENDERER_TITLE = "tutorial.step.renderer.title"
    const val TUTORIAL_STEP_RENDERER_BODY = "tutorial.step.renderer.body"
    const val TUTORIAL_STEP_IMPORT_TITLE = "tutorial.step.import.title"
    const val TUTORIAL_STEP_IMPORT_BODY = "tutorial.step.import.body"
    const val TUTORIAL_STEP_DONE_TITLE = "tutorial.step.done.title"
    const val TUTORIAL_STEP_DONE_BODY = "tutorial.step.done.body"
    const val TUTORIAL_SKIP = "tutorial.skip"
    const val TUTORIAL_NEXT = "tutorial.next"
    const val TUTORIAL_PREVIOUS = "tutorial.previous"
    const val TUTORIAL_FINISH = "tutorial.finish"
    const val TUTORIAL_REWATCH = "tutorial.rewatch"
    /** Carries `{current}` and `{total}` placeholders. */
    const val TUTORIAL_STEP_INDICATOR = "tutorial.step_indicator"
    const val TUTORIAL_REPLAY_BUTTON = "tutorial.replay_button"
    const val TUTORIAL_REPLAY_SUMMARY = "tutorial.replay_summary"

    // --- task 19: in-app small file manager ---------------------------------
    const val SCREEN_FILE_MANAGER = "screen.file_manager.title"
    const val FILE_MANAGER_NEW_FOLDER = "file_manager.new_folder"
    const val FILE_MANAGER_IMPORT = "file_manager.import"
    const val FILE_MANAGER_DELETE = "file_manager.delete"
    const val FILE_MANAGER_REFRESH = "file_manager.refresh"
    const val FILE_MANAGER_BACK = "file_manager.back"
    const val FILE_MANAGER_UP = "file_manager.up"
    const val FILE_MANAGER_MORE = "file_manager.more"
    const val FILE_MANAGER_OPEN = "file_manager.open"
    const val FILE_MANAGER_RENAME = "file_manager.rename"
    const val FILE_MANAGER_COPY = "file_manager.copy"
    const val FILE_MANAGER_EXTRACT_HERE = "file_manager.extract_here"
    const val FILE_MANAGER_CONFIRM_DELETE_TITLE = "file_manager.confirm_delete_title"
    const val FILE_MANAGER_CONFIRM_DELETE_BODY_DIRS = "file_manager.confirm_delete_body_dirs"
    const val FILE_MANAGER_CONFIRM_DELETE_BODY = "file_manager.confirm_delete_body"
    const val FILE_MANAGER_CONFIRM_DELETE_IRREVERSIBLE = "file_manager.confirm_delete_irreversible"
    const val FILE_MANAGER_CONFIRM_OVERWRITE_TITLE = "file_manager.confirm_overwrite_title"
    const val FILE_MANAGER_CONFIRM_OVERWRITE_BODY = "file_manager.confirm_overwrite_body"
    const val FILE_MANAGER_DIALOG_CREATE_TITLE = "file_manager.dialog.create.title"
    const val FILE_MANAGER_DIALOG_CREATE_LABEL = "file_manager.dialog.create.label"
    const val FILE_MANAGER_DIALOG_RENAME_TITLE = "file_manager.dialog.rename.title"
    const val FILE_MANAGER_DIALOG_RENAME_LABEL = "file_manager.dialog.rename.label"
    const val FILE_MANAGER_DIALOG_COPY_TITLE = "file_manager.dialog.copy.title"
    const val FILE_MANAGER_DIALOG_COPY_LABEL = "file_manager.dialog.copy.label"
    const val FILE_MANAGER_EMPTY = "file_manager.empty"
    const val FILE_MANAGER_KIND_DIR = "file_manager.kind.dir"
    const val FILE_MANAGER_KIND_FILE = "file_manager.kind.file"
    const val FILE_MANAGER_KIND_LINK = "file_manager.kind.link"

    // --- task 20: in-game floating HUD (overlays / replaces the dashboard HUD) ---
    const val HUD_MENU_TITLE = "hud.menu_title"
    const val HUD_ACTION_OPEN_LOG = "hud.action_open_log"
    const val HUD_ACTION_LOG_PASSTHROUGH = "hud.action_log_passthrough"
    const val HUD_ACTION_SWITCH_INPUT = "hud.action_switch_input"
    const val HUD_ACTION_SCREENSHOT = "hud.action_screenshot"
    const val HUD_ACTION_FORCE_QUIT = "hud.action_force_quit"
    const val HUD_SETTING_TITLE = "hud.setting_title"
    const val HUD_SETTING_OPACITY = "hud.setting_opacity"
    const val HUD_SETTING_AUTO_HIDE = "hud.setting_auto_hide"
    const val HUD_SETTING_AUTO_HIDE_SUMMARY = "hud.setting_auto_hide_summary"
    const val HUD_SETTING_ANTI_MISOPERATION = "hud.setting_anti_misoperation"
    const val HUD_SETTING_ANTI_MISOPERATION_SUMMARY = "hud.setting_anti_misoperation_summary"
    const val HUD_INPUT_MODE_TOUCH = "hud.input_mode_touch"
    const val HUD_INPUT_MODE_MOUSE = "hud.input_mode_mouse"
    const val HUD_EXPAND_CONTENT_DESCRIPTION = "hud.expand_content_description"
    const val HUD_COLLAPSE_CONTENT_DESCRIPTION = "hud.collapse_content_description"
    const val HUD_DRAG_CONTENT_DESCRIPTION = "hud.drag_content_description"
    const val HUD_CLOSE_CONTENT_DESCRIPTION = "hud.close_content_description"

    // Task 21: expanded log viewer
    const val HUD_LOG_FILTER_ALL = "hud.log_filter_all"
    const val HUD_LOG_FILTER_STDOUT = "hud.log_filter_stdout"
    const val HUD_LOG_FILTER_STDERR = "hud.log_filter_stderr"
    const val HUD_LOG_FILTER_ERRORS = "hud.log_filter_errors"
    const val HUD_LOG_FILTER_WARN = "hud.log_filter_warn"
    const val HUD_LOG_SEARCH_PLACEHOLDER = "hud.log_search_placeholder"
    const val HUD_LOG_EXPORT = "hud.log_export"
    const val HUD_LOG_EXPORTING = "hud.log_exporting"
    const val HUD_LOG_EXPORT_DONE = "hud.log_export_done"
    const val HUD_LOG_EXPORT_FAILED = "hud.log_export_failed"
    const val HUD_LOG_SNAPSHOT = "hud.log_snapshot"
    const val HUD_LOG_CLEAR = "hud.log_clear"
    const val HUD_LOG_LEVEL_INFO = "hud.log_level_info"
    const val HUD_LOG_LEVEL_WARN = "hud.log_level_warn"
    const val HUD_LOG_LEVEL_ERROR = "hud.log_level_error"
    const val HUD_LOG_LEVEL_DEBUG = "hud.log_level_debug"
    const val HUD_LOG_LEVEL_TRACE = "hud.log_level_trace"
    const val HUD_LOG_LEVEL_FATAL = "hud.log_level_fatal"
    const val HUD_LOG_LEVEL_STDOUT = "hud.log_level_stdout"
    const val HUD_LOG_LEVEL_STDERR = "hud.log_level_stderr"
    const val HUD_LOG_LEVEL_UNKNOWN = "hud.log_level_unknown"

    // TODO: log overlay
    const val HUD_LOG_OVERLAY_TITLE = "hud.log_overlay_title"
    const val HUD_LOG_PASSTHROUGH = "hud.log_passthrough"
    const val HUD_LOG_PASSTHROUGH_SUMMARY = "hud.log_passthrough_summary"

    // --- task 21: crash snapshot banner labels ---
    const val HUD_CRASH_EXIT_CODE = "hud.crash_exit_code"
    const val HUD_CRASH_SIGNAL = "hud.crash_signal"
    const val HUD_CRASH_NA = "hud.crash_na"

    // --- task 22: in-app skin preview ---
    const val SKIN_PREVIEW_TITLE = "skin.preview.title"
    const val SKIN_SELECT_MODEL = "skin.select_model"
    const val SKIN_FETCHING = "skin.fetching"
    const val SKIN_UPLOADING = "skin.uploading"
    const val SKIN_PICK_LOCAL = "skin.pick_local"
    const val SKIN_UPLOAD = "skin.upload"
    const val SKIN_UPLOADING_LABEL = "skin.uploading_label"
    const val SKIN_VIEW_IN_BROWSER = "skin.view_in_browser"
    const val SKIN_MODEL_STEVE = "skin.model.steve"
    const val SKIN_MODEL_ALEX = "skin.model.alex"
    const val SKIN_SOURCE_OFFICIAL = "skin.source.official"
    const val SKIN_SOURCE_CUSTOM = "skin.source.custom"
    const val SKIN_HAS_CAPE = "skin.has_cape"
    const val SKIN_CACHED_OFFLINE = "skin.cached_offline"
    const val SKIN_OFFLINE_ACCOUNT = "skin.offline_account"
    const val SKIN_UPLOAD_CONFIRM = "skin.upload_confirm"
    const val SKIN_UPLOAD_CANCEL = "skin.upload_cancel"
    const val SKIN_UPLOAD_NOTE = "skin.upload_note"
    const val SKIN_INVALID_DIMENSIONS = "skin.invalid_dimensions"
    const val SKIN_NO_TEXTURE = "skin.no_texture"
    const val SKIN_READ_ERROR = "skin.read_error"

    // --- task 23: skin import tutorial ----------------------------------
    const val SKIN_TUTORIAL_TITLE = "skin.tutorial.title"
const val SKIN_TUTORIAL_REWATCH = "skin.tutorial.rewatch"
    /** Carries `{current}` and `{total}` placeholders. */
    const val SKIN_TUTORIAL_STEP_INDICATOR = "skin.tutorial.step_indicator"
    const val SKIN_TUTORIAL_STEP_GET_TITLE = "skin.tutorial.step.get.title"
    const val SKIN_TUTORIAL_STEP_GET_BODY = "skin.tutorial.step.get.body"
    const val SKIN_TUTORIAL_STEP_FORMAT_TITLE = "skin.tutorial.step.format.title"
    const val SKIN_TUTORIAL_STEP_FORMAT_BODY = "skin.tutorial.step.format.body"
    const val SKIN_TUTORIAL_STEP_IMPORT_TITLE = "skin.tutorial.step.import.title"
    const val SKIN_TUTORIAL_STEP_IMPORT_BODY = "skin.tutorial.step.import.body"
    const val SKIN_TUTORIAL_STEP_APPLY_TITLE = "skin.tutorial.step.apply.title"
    const val SKIN_TUTORIAL_STEP_APPLY_BODY = "skin.tutorial.step.apply.body"
    const val SKIN_TUTORIAL_STEP_DONE_TITLE = "skin.tutorial.step.done.title"
    const val SKIN_TUTORIAL_STEP_DONE_BODY = "skin.tutorial.step.done.body"
    const val SKIN_TUTORIAL_START = "skin.tutorial.start"
    const val SKIN_TUTORIAL_SKIP = "skin.tutorial.skip"
    const val SKIN_TUTORIAL_NEXT = "skin.tutorial.next"
    const val SKIN_TUTORIAL_PREVIOUS = "skin.tutorial.previous"
    const val SKIN_TUTORIAL_FINISH = "skin.tutorial.finish"
    const val SKIN_TUTORIAL_PICK = "skin.tutorial.pick"
    const val SKIN_TUTORIAL_OFFICIAL_NOTE = "skin.tutorial.official_note"
    const val SKIN_TUTORIAL_THIRDPARTY_NOTE = "skin.tutorial.thirdparty_note"
    const val SKIN_TUTORIAL_FORMAT_NOTE = "skin.tutorial.format_note"
    const val SKIN_TUTORIAL_LINK_TASK14 = "skin.tutorial.link_task14"

    // --- task 25/26/27: mod expand/collapse, world & pack management ---
    const val MOD_COLLAPSE = "mod.collapse"
    const val MOD_DESCRIPTION = "mod.description"
    const val MOD_DEPENDENCIES = "mod.dependencies"
    const val MOD_VERSION = "mod.version"
    const val MOD_EXPAND = "mod.expand"
    const val RESOURCE_PACK_COMPATIBLE = "resource_pack.compatible"
    const val RESOURCE_PACK_DELETE = "resource_pack.delete"
    const val RESOURCE_PACK_DELETE_CONFIRM = "resource_pack.delete_confirm"
    const val RESOURCE_PACK_DESCRIPTION = "resource_pack.description"
    const val RESOURCE_PACK_DISABLE = "resource_pack.disable"
    const val RESOURCE_PACK_DOWNLOAD = "resource_pack.download"
    const val RESOURCE_PACK_EMPTY = "resource_pack.empty"
    const val RESOURCE_PACK_ENABLE = "resource_pack.enable"
    const val RESOURCE_PACK_IMPORT = "resource_pack.import"
    const val RESOURCE_PACK_INCOMPATIBLE = "resource_pack.incompatible"
    const val RESOURCE_PACK_PACK_FORMAT = "resource_pack.pack_format"
    const val RESOURCE_PACK_PREVIEW = "resource_pack.preview"
    const val SCREEN_RESOURCE_PACK_TITLE = "screen.resource_pack.title"
    const val SCREEN_SHADER_PACK_TITLE = "screen.shader_pack.title"
    const val SCREEN_WORLD_MANAGER_TITLE = "screen.world_manager.title"
    const val SHADER_PACK_DELETE = "shader_pack.delete"
    const val SHADER_PACK_DELETE_CONFIRM = "shader_pack.delete_confirm"
    const val SHADER_PACK_DISABLE = "shader_pack.disable"
    const val SHADER_PACK_DOWNLOAD = "shader_pack.download"
    const val SHADER_PACK_EMPTY = "shader_pack.empty"
    const val SHADER_PACK_ENABLE = "shader_pack.enable"
    const val SHADER_PACK_IMPORT = "shader_pack.import"
    const val SHADER_PACK_INVALID = "shader_pack.invalid"
    const val SHADER_PACK_PREVIEW = "shader_pack.preview"
    const val SHADER_PACK_VALID = "shader_pack.valid"
    const val WORLD_BACKUP = "world.backup"
    const val WORLD_BACKUP_DONE = "world.backup_done"
    const val WORLD_BACKUP_FAILED = "world.backup_failed"
    const val WORLD_CONFIRM_DELETE_BODY = "world.confirm_delete_body"
    const val WORLD_CONFIRM_DELETE_TITLE = "world.confirm_delete_title"
    const val WORLD_CONFIRM_RENAME_LABEL = "world.confirm_rename_label"
    const val WORLD_CONFIRM_RENAME_TITLE = "world.confirm_rename_title"
    const val WORLD_DELETE = "world.delete"
    const val WORLD_DELETE_DONE = "world.delete_done"
    const val WORLD_DELETE_FAILED = "world.delete_failed"
    const val WORLD_EMPTY = "world.empty"
    const val WORLD_EXPORT = "world.export"
    const val WORLD_EXPORT_DONE = "world.export_done"
    const val WORLD_EXPORT_FAILED = "world.export_failed"
    const val WORLD_GAME_MODE = "world.game_mode"
    const val WORLD_HAS_BACKUP = "world.has_backup"
    const val WORLD_IMPORT = "world.import"
    const val WORLD_IMPORT_DONE = "world.import_done"
    const val WORLD_IMPORT_FAILED = "world.import_failed"
    const val WORLD_LAST_MODIFIED = "world.last_modified"
    const val WORLD_PLAY_TIME = "world.play_time"
    const val WORLD_RECOVER = "world.recover"
    const val WORLD_RECOVER_DONE = "world.recover_done"
    const val WORLD_RECOVER_FAILED = "world.recover_failed"
    const val WORLD_RENAME = "world.rename"
    const val WORLD_RENAME_DONE = "world.rename_done"
    const val WORLD_RENAME_FAILED = "world.rename_failed"
    const val WORLD_THUMBNAIL = "world.thumbnail"
