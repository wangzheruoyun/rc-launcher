package com.rc.launcher.ui.model

import com.rc.launcher.core.RustBridge
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/**
 * UI-side facade for the complete and auto-updating game-version list (task 16).
 *
 * The Rust core owns the authoritative cache, the mirror fallback, the TTL,
 * the grouping, and the search. This Kotlin wrapper:
 *
 *   - calls the Rust bridge through the typed helpers in [RustBridge];
 *   - exposes the reply as three Compose-friendly `StateFlow`s (status,
 *     groups, filtered list) so the picker just observes state;
 *   - schedules the initial load on a background dispatcher and lets the
 *     UI `refresh()` / `applyFilter()` without ever blocking on the bridge;
 *   - degrades gracefully: a bridge failure leaves the last-known groups
 *     visible and surfaces the failure through [VersionCatalogState.error].
 *
 * The Compose layer never touches JSON itself; it renders
 * `state.groups[kind]` and `state.filtered` directly.
 */
object VersionCatalog {

    /** One bucket in the version picker. Mirrors the Rust [VersionGroups] keys. */
    enum class Group(val key: String) {
        RELEASE("release"),
        SNAPSHOT("snapshot"),
        PRE_RELEASE("pre_release"),
        OLD_ALPHA("old_alpha"),
        OLD_BETA("old_beta"),
        SPECIAL("special"),
        ;

        companion object {
            /** Map a Rust-side string to a Kotlin enum, falling back to
             *  [SPECIAL] for unknown values (forward compatibility). */
            fun fromKey(key: String?): Group = entries.firstOrNull { it.key == key } ?: SPECIAL
        }
    }

    /** Immutable snapshot of what the UI is currently rendering. */
    data class VersionCatalogState(
        /** True when the most recent fetch succeeded (info.fresh or
         *  info.stale_fallback with non-empty data). */
        val hasData: Boolean = false,
        /** Bridge-side status fields, propagated for the status badge. */
        val fresh: Boolean = false,
        val staleFallback: Boolean = false,
        val offlineOnly: Boolean = false,
        val fetchedAtUnix: Long? = null,
        val totalEntries: Int = 0,
        /** `group -> count` (informational; the counts are also implicit in
         *  [groups]). */
        val groupCounts: Map<Group, Int> = emptyMap(),
        /** The currently displayed buckets, each a list of version ids
         *  ordered newest-first. */
        val groups: Map<Group, List<String>> = emptyMap(),
        /** Optional, currently-active search / group filter; the picker
         *  applies these to derive the rows the user actually sees. */
        val activeQuery: String = "",
        val activeGroup: Group? = null,
        /** Last error message, if any. Cleared by the next successful fetch. */
        val error: String? = null,
        /** True while a fetch is in flight. UI shows a progress indicator. */
        val loading: Boolean = false,
    )

    private val _state = MutableStateFlow(VersionCatalogState())
    /** Observable state for the Compose version picker. */
    val state: StateFlow<VersionCatalogState> = _state.asStateFlow()

    /**
     * Kick off a fetch with the given search / group filter.
     *
     * `ttlSecs == null` -> use the Rust default (6h).
     * `forceRefresh == true` -> ignore the TTL cache.
     */
    suspend fun refresh(
        query: String = _state.value.activeQuery,
        group: Group? = _state.value.activeGroup,
        ttlSecs: Long? = null,
        forceRefresh: Boolean = false,
    ) {
        _state.update { it.copy(loading = true, error = null) }
        val reply = withContext(Dispatchers.IO) {
            if (forceRefresh) {
                RustBridge.refreshVersionList()
            } else {
                RustBridge.fetchVersionList(
                    query = query,
                    group = group?.key,
                    ttlSecs = ttlSecs,
                    forceRefresh = forceRefresh,
                )
            }
        }
        applyReply(reply, query = query, group = group)
    }

    /** Apply an in-memory filter without touching the bridge. Useful for the
     *  Compose search box: keystrokes debounce and call this with the
     *  trimmed query so the picker re-renders without a network round-trip. */
    fun applyFilter(query: String, group: Group?) {
        _state.update { current ->
            current.copy(activeQuery = query.trim(), activeGroup = group)
        }
    }

    /** Bridge-side cache info, surfaced for the status badge / settings. */
    fun cacheInfo(ttlSecs: Long? = null): JSONObject = RustBridge.versionListCacheInfo(ttlSecs)

    /** Drop the bridge-side cache. The next [refresh] re-fetches or
     *  degrades to the offline built-in manifest. */
    fun clearCache(): Boolean = RustBridge.clearVersionListCache()

    /** Update the state from a typed bridge reply. Visible for tests; UI
     *  callers should use [refresh] / [applyFilter] instead. */
    internal fun applyReply(
        reply: RustBridge.VersionListReply,
        query: String,
        group: Group?,
    ) {
        val info = reply.info
        val counts = mutableMapOf<Group, Int>()
        val groups = mutableMapOf<Group, List<String>>()
        for (g in Group.entries) {
            val arr: JSONArray = reply.groups.optJSONArray(g.key) ?: JSONArray()
            val ids = (0 until arr.length()).mapNotNull { i ->
                arr.optJSONObject(i)?.optString("id")?.takeIf { it.isNotEmpty() }
            }
            counts[g] = ids.size
            groups[g] = ids
        }
        _state.update {
            it.copy(
                hasData = reply.manifest.length() > 0,
                fresh = info.optBoolean("fresh", false),
                staleFallback = info.optBoolean("stale_fallback", false),
                offlineOnly = info.optBoolean("offline_only", false),
                fetchedAtUnix = info.optLong("fetched_at_unix", -1L).takeIf { v -> v > 0L },
                totalEntries = info.optInt("total", 0),
                groupCounts = counts,
                groups = groups,
                activeQuery = query,
                activeGroup = group,
                error = info.optString("error").takeIf { e -> e.isNotEmpty() },
                loading = false,
            )
        }
    }

    /** Convenience: the currently-active (query, group) -> list of ids the
     *  picker is meant to render. */
    fun activeIds(state: VersionCatalogState = _state.value): List<String> {
        val g = state.activeGroup
        return if (g == null) {
            // No bucket filter -> intersect every bucket with the query.
            state.groups.values.flatten().distinct().filter { id ->
                state.activeQuery.isEmpty() ||
                    id.contains(state.activeQuery, ignoreCase = true)
            }
        } else {
            state.groups[g].orEmpty().filter { id ->
                state.activeQuery.isEmpty() ||
                    id.contains(state.activeQuery, ignoreCase = true)
            }
        }
    }
}
