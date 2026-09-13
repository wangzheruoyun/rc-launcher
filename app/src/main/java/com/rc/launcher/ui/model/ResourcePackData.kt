package com.rc.launcher.ui.model

import java.io.File

/**
 * A Minecraft resource pack discovered in an instance's `resourcepacks/`
 * directory (task 27).
 *
 * Mirrors the Rust core's [`crate::mods::resource_pack::ResourcePack`:
 * enable/disable is tracked on-disk by the ``.disabled`` suffix convention;
 * the Kotlin side reads the same files so the two layers cannot disagree about
 * state. The ``packFormat`` / ``compatible`` pair lets the UI warn when a pack
 * targets a Minecraft version the user hasn't selected yet — a frequent source
 * of "black textures" support tickets.
 */
data class ResourcePackData(
    /** Base name (without ``.disabled`` suffix). */
    val name: String,
    /** Absolute path (may end in ``.disabled`` when the pack is disabled). */
    val path: File,
    /** Whether the pack is currently enabled (no ``.disabled`` suffix). */
    val enabled: Boolean,
    /** ``pack_format`` parsed from ``pack.mcmeta``, or null when unknown. */
    val packFormat: Int?,
    /** Human-readable description from ``pack.mcmeta``. */
    val description: String?,
) {
    /** Whether the pack is a `.zip` archive (vs. a folder). */
    val isArchive: Boolean get() = path.name.endsWith(".zip", ignoreCase = true)
}
