package com.rc.launcher.ui.model

import java.io.File

/**
 * A Minecraft shader pack discovered in an instance's `shaderpacks/` directory
 * (task 27).
 *
 * Unlike mods and resource packs, shader packs have **no** embedded manifest,
 * so identity is the folder/zip name. Validity is "does it contain a
 * ``shaders/`` tree with at least one ``.fsh`` / ``.vsh`` program or a
 * ``shaders.properties`` file."
 */
data class ShaderPackData(
    /** Base name (without ``.disabled`` suffix). */
    val name: String,
    /** Absolute path (may end in ``.disabled`` when the pack is disabled). */
    val path: File,
    /** Whether the pack is currently enabled (no ``.disabled`` suffix). */
    val enabled: Boolean,
    /** Whether the archive/folder actually contains a valid ``shaders/`` tree. */
    val valid: Boolean,
) {
    /** Whether the pack is a `.zip` archive (vs. a folder). */
    val isArchive: Boolean get() = path.name.endsWith(".zip", ignoreCase = true)
}
