// GENERATED FILE — DO NOT EDIT.
//
// Source of truth: rust/crates/rc-launcher-core/i18n/zh-CN.properties
// Regenerate with: python3 scripts/gen_android_strings.py
// Verified in CI : python3 scripts/check_i18n.py
package com.rc.launcher.ui.i18n

import com.rc.launcher.R

/**
 * Maps an i18n **key** (`nav.home`) to its Android resource id
 * (`R.string.nav_home`), for task 20.
 *
 * Generated rather than resolved with `Resources.getIdentifier`, because
 * reflection-style resource lookup breaks under resource shrinking / R8 and
 * silently returns 0 instead of failing the build.
 *
 * This is the *fallback* path: [RcStrings] prefers the live catalogue handed over
 * by the Rust core ([com.rc.launcher.core.RustBridge.i18nBundle]) and only reads
 * Android resources when the native core is unavailable — which is exactly the
 * offline / degraded case task 19 requires the UI to survive.
 */
object RcStringResources {
    /** Every key the generator projected into the `values-...` resource files. */
    val ids: Map<String, Int> = mapOf(
        "app.name" to R.string.app_name,
        "app.tagline" to R.string.app_tagline,
        "common.add" to R.string.common_add,
        "common.apply" to R.string.common_apply,
        "common.back" to R.string.common_back,
        "common.cancel" to R.string.common_cancel,
        "common.close" to R.string.common_close,
        "common.default" to R.string.common_default,
        "common.delete" to R.string.common_delete,
        "common.edit" to R.string.common_edit,
        "common.loading" to R.string.common_loading,
        "common.next" to R.string.common_next,
        "common.ok" to R.string.common_ok,
        "common.previous" to R.string.common_previous,
        "common.refresh" to R.string.common_refresh,
        "common.retry" to R.string.common_retry,
        "common.save" to R.string.common_save,
        "common.unavailable" to R.string.common_unavailable,
        "crash.authentication_failure.advice" to R.string.crash_authentication_failure_advice,
        "crash.authentication_failure.summary" to R.string.crash_authentication_failure_summary,
        "crash.clean_exit.advice" to R.string.crash_clean_exit_advice,
        "crash.clean_exit.summary" to R.string.crash_clean_exit_summary,
        "crash.corrupted_file.advice" to R.string.crash_corrupted_file_advice,
        "crash.corrupted_file.summary" to R.string.crash_corrupted_file_summary,
        "crash.disk_full.advice" to R.string.crash_disk_full_advice,
        "crash.disk_full.summary" to R.string.crash_disk_full_summary,
        "crash.game_error.advice" to R.string.crash_game_error_advice,
        "crash.game_error.summary" to R.string.crash_game_error_summary,
        "crash.graphics_failure.advice" to R.string.crash_graphics_failure_advice,
        "crash.graphics_failure.summary" to R.string.crash_graphics_failure_summary,
        "crash.killed_by_system.advice" to R.string.crash_killed_by_system_advice,
        "crash.killed_by_system.summary" to R.string.crash_killed_by_system_summary,
        "crash.missing_main_class.advice" to R.string.crash_missing_main_class_advice,
        "crash.missing_main_class.summary" to R.string.crash_missing_main_class_summary,
        "crash.missing_native_library.advice" to R.string.crash_missing_native_library_advice,
        "crash.missing_native_library.summary" to R.string.crash_missing_native_library_summary,
        "crash.mod_loader_failure.advice" to R.string.crash_mod_loader_failure_advice,
        "crash.mod_loader_failure.summary" to R.string.crash_mod_loader_failure_summary,
        "crash.native_crash.advice" to R.string.crash_native_crash_advice,
        "crash.native_crash.summary" to R.string.crash_native_crash_summary,
        "crash.out_of_memory.advice" to R.string.crash_out_of_memory_advice,
        "crash.out_of_memory.summary" to R.string.crash_out_of_memory_summary,
        "crash.permission_denied.advice" to R.string.crash_permission_denied_advice,
        "crash.permission_denied.summary" to R.string.crash_permission_denied_summary,
        "crash.unknown.advice" to R.string.crash_unknown_advice,
        "crash.unknown.summary" to R.string.crash_unknown_summary,
        "crash.unsupported_java_version.advice" to R.string.crash_unsupported_java_version_advice,
        "crash.unsupported_java_version.summary" to R.string.crash_unsupported_java_version_summary,
        "crash.user_terminated.advice" to R.string.crash_user_terminated_advice,
        "crash.user_terminated.summary" to R.string.crash_user_terminated_summary,
        "download.eta" to R.string.download_eta,
        "download.files.one" to R.string.download_files_one,
        "download.files.other" to R.string.download_files_other,
        "duration.day.one" to R.string.duration_day_one,
        "duration.day.other" to R.string.duration_day_other,
        "duration.hour.one" to R.string.duration_hour_one,
        "duration.hour.other" to R.string.duration_hour_other,
        "duration.minute.one" to R.string.duration_minute_one,
        "duration.minute.other" to R.string.duration_minute_other,
        "duration.second.one" to R.string.duration_second_one,
        "duration.second.other" to R.string.duration_second_other,
        "duration.zero" to R.string.duration_zero,
        "error.auth" to R.string.error_auth,
        "error.checksum" to R.string.error_checksum,
        "error.launch" to R.string.error_launch,
        "error.missing_file" to R.string.error_missing_file,
        "error.network" to R.string.error_network,
        "error.offline" to R.string.error_offline,
        "error.rate_limited" to R.string.error_rate_limited,
        "error.retry_scheduled" to R.string.error_retry_scheduled,
        "error.severity.fatal" to R.string.error_severity_fatal,
        "error.severity.recoverable" to R.string.error_severity_recoverable,
        "error.severity.transient" to R.string.error_severity_transient,
        "error.timeout" to R.string.error_timeout,
        "error.unknown" to R.string.error_unknown,
        "format.decimal_separator" to R.string.format_decimal_separator,
        "format.duration_join" to R.string.format_duration_join,
        "format.fps" to R.string.format_fps,
        "format.group_separator" to R.string.format_group_separator,
        "format.invalid_number" to R.string.format_invalid_number,
        "format.percent" to R.string.format_percent,
        "format.progress_of" to R.string.format_progress_of,
        "format.rate" to R.string.format_rate,
        "format.size" to R.string.format_size,
        "language.en" to R.string.language_en,
        "language.system" to R.string.language_system,
        "language.zh_cn" to R.string.language_zh_cn,
        "language.zh_hant" to R.string.language_zh_hant,
        "launch.state.crashed" to R.string.launch_state_crashed,
        "launch.state.idle" to R.string.launch_state_idle,
        "launch.state.launching" to R.string.launch_state_launching,
        "launch.state.preparing" to R.string.launch_state_preparing,
        "launch.state.running" to R.string.launch_state_running,
        "launch.state.stopped" to R.string.launch_state_stopped,
        "nav.accounts" to R.string.nav_accounts,
        "nav.downloads" to R.string.nav_downloads,
        "nav.home" to R.string.nav_home,
        "nav.instances" to R.string.nav_instances,
        "nav.settings" to R.string.nav_settings,
        "relative.future" to R.string.relative_future,
        "relative.now" to R.string.relative_now,
        "relative.past" to R.string.relative_past,
        "screen.awt.title" to R.string.screen_awt_title,
        "screen.controller.title" to R.string.screen_controller_title,
        "screen.install.title" to R.string.screen_install_title,
        "screen.instance_detail.title" to R.string.screen_instance_detail_title,
        "settings.language.applied" to R.string.settings_language_applied,
        "settings.language.follow_system" to R.string.settings_language_follow_system,
        "settings.language.subtitle" to R.string.settings_language_subtitle,
        "settings.language.title" to R.string.settings_language_title,
        "settings.section.about" to R.string.settings_section_about,
        "settings.section.appearance" to R.string.settings_section_appearance,
        "settings.section.controller" to R.string.settings_section_controller,
        "settings.section.directory" to R.string.settings_section_directory,
        "settings.section.java" to R.string.settings_section_java,
        "settings.section.language" to R.string.settings_section_language,
        "settings.section.network" to R.string.settings_section_network,
        "settings.section.renderer" to R.string.settings_section_renderer,
        "theme.night.dark" to R.string.theme_night_dark,
        "theme.night.light" to R.string.theme_night_light,
        "theme.night.system" to R.string.theme_night_system,
        "theme.night.toggle" to R.string.theme_night_toggle,
        "unit.byte" to R.string.unit_byte,
        "unit.gib" to R.string.unit_gib,
        "unit.kib" to R.string.unit_kib,
        "unit.mib" to R.string.unit_mib,
        "unit.pib" to R.string.unit_pib,
        "unit.tib" to R.string.unit_tib,
    )

    /** The resource id of [key], or `null` when the key is not a resource. */
    fun idOf(key: String): Int? = ids[key]

    /** Number of generated string resources — asserted by the unit tests. */
    val size: Int get() = ids.size
}
