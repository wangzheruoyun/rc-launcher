package com.rc.launcher.ui.model

import java.io.File

/**
 * In-memory state container for the control-layout library (task 15).
 *
 * The screen needs a single immutable [ControlLayoutLibraryState] it can render
 * without race conditions; mutations go through the ViewModel and produce a new
 * instance. Keeping the state here (next to the model) lets unit tests assert
 * against it without instantiating the Compose UI.
 */
data class ControlLayoutLibraryState(
    /** All known sources (built-in catalogue entries). */
    val sources: List<ControlLayoutSource> = ControlLayoutLibraryCatalog.all,
    /** Sources the user has downloaded and is now considering. */
    val downloaded: List<ControlLayoutSource> = emptyList(),
    /** Per-source download progress (0..1) keyed by source id. */
    val progress: Map<String, Float> = emptyMap(),
    /** Per-source status text (transient). */
    val status: Map<String, String> = emptyMap(),
    /** Decoded packages the user has *imported* in this session. */
    val imported: List<ControlLayoutPackage> = emptyList(),
    /** Last error encountered, or null. */
    val error: String? = null,
    /** True while at least one download is in flight. */
    val busy: Boolean = false,
) {
    fun progressFor(id: String): Float = progress[id] ?: 0f
    fun statusFor(id: String): String? = status[id]

    companion object {
        val Empty: ControlLayoutLibraryState = ControlLayoutLibraryState()
    }
}

/**
 * Materialised result of fetching a community layout (task 15).
 *
 * Mirrors the FCL/Zalith "import" payload contract: a source becomes a
 * downloadable [ControlLayoutSource], which is then fetched into a [File] and
 * decoded into a [ControlLayoutPackage] by [ControlLayoutImporter.decode].
 */
data class LibraryFetch(
    val source: ControlLayoutSource,
    val file: File,
    val pkg: ControlLayoutPackage,
)
