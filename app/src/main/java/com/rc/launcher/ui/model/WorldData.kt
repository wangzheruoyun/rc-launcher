package com.rc.launcher.ui.model

import java.io.File

/**
 * A Minecraft save/archive discovered in an instance's `saves/` directory
 * (task 26).
 *
 * Each save is a folder (or `.zip` archive) containing at minimum a
 * `level.dat` (which carries the world name, game mode, game time and the
 * random seed). The thumbnail is the `level.dat`-derived preview or the
 * `screenshot.jpg`/`world.png` that the game writes next to `level.dat`.
 *
 * The whole model is plain Kotlin (no Android / Compose imports) so it is
 * unit-testable on the JVM and can be serialised by [MiniJson] for caching
 * across restarts — matching how FCL/Zalith surface saves in their
 * "world management" panels.
 */
data class WorldData(
    /** Folder or archive name (without ``.disabled`` / ``.zip`` suffix). */
    val name: String,
    /** Absolute path to the save folder or ``.zip`` archive. */
    val path: File,
    /** Whether this entry is a folder (``true``) or a ``.zip`` archive (``false``). */
    val isDirectory: Boolean,
    /** Human-readable game mode loaded from ``level.dat`` (e.g. "survival"). */
    val gameMode: String,
    /** In-game day-time ticks; 0 if unparseable. */
    val gameTime: Long,
    /** Epoch-millis of the last modification, for sorting "most recent first". */
    val lastModified: Long,
    /** Path to the thumbnail image (world.png / screenshot.jpg), or null. */
    val thumbnail: File?,
    /** Whether a backup archive already exists for this save. */
    val hasBackup: Boolean,
) {
    /** Display name (folder/file name without extension and without ``.disabled``). */
    val displayName: String get() = name
}
