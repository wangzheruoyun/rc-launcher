package com.rc.launcher.ui.translate

import android.content.Context
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.rc.launcher.core.RustBridge
import com.rc.launcher.ui.i18n.AppLanguage
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/**
 * Mod-browser **inline translation** state container (task 13).
 *
 * Holds the player's translation preferences (enabled / target language /
 * mode / show-original-or-translated toggle), the in-flight list of
 * translated entries, and the cache stats the settings UI displays.
 *
 * The actual translation work happens in the Rust core's
 * `TranslationService` (see `rust/crates/rc-launcher-core/src/translate/`);
 * this viewmodel only marshals JSON and stitches Compose state on top of
 * the result.
 *
 * Cheap to instantiate (true no-arg), so `viewModel()` can build it via
 * reflection. The first call to [ensureInitialised] talks to the native
 * library — callers should kick it off from a background coroutine
 * (e.g. `LaunchedEffect(Unit) { vm.ensureInitialised(context) }`).
 */
class TranslationViewModel : ViewModel() {

    /** The user's preference: enabled / target / mode / "show original" toggle. */
    data class Preferences(
        val enabled: Boolean = true,
        val target: TranslationTarget = TranslationTarget.Auto,
        val mode: TranslationMode = TranslationMode.Hybrid,
        val showOriginal: Boolean = true,
    )

    /** Target language picker options (mirrors the Rust `TranslationLanguage`). */
    enum class TranslationTarget(val tag: String) {
        Auto("auto"),
        ZhCn("zh-CN"),
        ZhHant("zh-Hant"),
        En("en"),
        ;
        companion object {
            fun fromTag(tag: String?): TranslationTarget = entries.firstOrNull {
                it.tag.equals(tag, ignoreCase = true)
            } ?: Auto
        }
    }

    /** Translation mode options (mirrors the Rust `TranslationMode`). */
    enum class TranslationMode(val id: String) {
        Online("online"),
        Offline("offline"),
        Hybrid("hybrid"),
        ;
        companion object {
            fun fromId(id: String?): TranslationMode = entries.firstOrNull {
                it.id.equals(id, ignoreCase = true)
            } ?: Hybrid
        }
    }

    /** Source of a translation, surfaced as a badge in the UI. */
    enum class TranslationSource(val id: String) {
        Passthrough("passthrough"),
        Dictionary("dictionary"),
        Cache("cache"),
        Gateway("gateway"),
        Unavailable("unavailable"),
        Unknown(""),
        ;
        companion object {
            fun fromId(id: String?): TranslationSource = entries.firstOrNull {
                it.id.equals(id, ignoreCase = true)
            } ?: Unknown
        }
    }

    /** One translated mod / entry, ready for the browser UI. */
    data class TranslatedEntry(
        val original: String,
        val translated: String,
        val source: TranslationSource,
        val offline: Boolean,
    )

    /** Cache statistics, surfaced in the settings screen. */
    data class CacheStats(
        val root: String,
        val entryCount: Long,
        val totalBytes: Long,
        val maxEntries: Long,
        val maxBytes: Long,
    )

    /** Snapshot the UI observes. */
    data class State(
        val prefs: Preferences = Preferences(),
        val cacheStats: CacheStats? = null,
        val initialised: Boolean = false,
        val lastError: String? = null,
    )

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    /** Lazily initialised — only the first call touches the native lib. */
    fun ensureInitialised(context: Context) {
        if (_state.value.initialised) return
        viewModelScope.launch {
            try {
                withContext(Dispatchers.IO) {
                    // Hand the core the canonical cache dir so the on-disk
                    // cache survives app restarts.
                    val cacheRoot = java.io.File(
                        context.cacheDir, "rc_translation",
                    ).absolutePath
                    val prefs = _state.value.prefs
                    val req = JSONObject().apply {
                        put("cache_root", cacheRoot)
                        put("force_offline", false)
                        put("default_mode", prefs.mode.id)
                    }
                    RustBridge.translateInit(req.toString())
                    val stats = parseCacheStats(
                        RustBridge.translateCacheStats(),
                    )
                    _state.value = _state.value.copy(
                        initialised = true,
                        cacheStats = stats,
                        lastError = null,
                    )
                }
            } catch (t: Throwable) {
                _state.value = _state.value.copy(
                    initialised = false,
                    lastError = t.message ?: "init failed",
                )
            }
        }
    }

    fun setEnabled(on: Boolean) {
        _state.value = _state.value.copy(prefs = _state.value.prefs.copy(enabled = on))
    }

    fun setTarget(target: TranslationTarget) {
        _state.value = _state.value.copy(prefs = _state.value.prefs.copy(target = target))
    }

    fun setMode(mode: TranslationMode) {
        _state.value = _state.value.copy(prefs = _state.value.prefs.copy(mode = mode))
        // Push the new mode to the core so it changes the default for
        // any subsequent translate() call without an explicit mode.
        viewModelScope.launch(Dispatchers.IO) {
            try {
                RustBridge.translateInit(
                    JSONObject().apply { put("default_mode", mode.id) }.toString(),
                )
            } catch (_: Throwable) { /* ignore */ }
        }
    }

    fun setShowOriginal(on: Boolean) {
        _state.value = _state.value.copy(prefs = _state.value.prefs.copy(showOriginal = on))
    }

    /**
     * Translate a batch of `(original, optional_hint)` pairs in one
     * native call. Returns the results in the same order as the inputs.
     *
     * When [enabled] is `false` the original text is returned with
     * `source = Passthrough`, so the caller can render the list without
     * branching.
     */
    suspend fun translateBatch(
        texts: List<String>,
        hint: TranslationTarget? = null,
    ): List<TranslatedEntry> = withContext(Dispatchers.IO) {
        if (texts.isEmpty()) return@withContext emptyList()
        val prefs = _state.value.prefs
        if (!prefs.enabled) {
            return@withContext texts.map { TranslatedEntry(it, it, TranslationSource.Passthrough, true) }
        }
        val target = (hint ?: prefs.target).tag
        val reqs: MutableList<Pair<String, String>> = ArrayList(texts.size)
        for (t in texts) reqs.add(t to target)
        try {
            val raw = RustBridge.translateBatchText(reqs, prefs.mode.id)
            parseBatch(raw, texts)
        } catch (t: Throwable) {
            // On any failure, return passthroughs so the UI keeps
            // rendering the browser.
            _state.value = _state.value.copy(lastError = t.message)
            texts.map { TranslatedEntry(it, it, TranslationSource.Unavailable, true) }
        }
    }

    /** Clear the translation cache; refresh [state.cacheStats] afterwards. */
    fun clearCache() {
        viewModelScope.launch(Dispatchers.IO) {
            try {
                RustBridge.translateClearCache()
                val stats = parseCacheStats(RustBridge.translateCacheStats())
                _state.value = _state.value.copy(cacheStats = stats, lastError = null)
            } catch (t: Throwable) {
                _state.value = _state.value.copy(lastError = t.message)
            }
        }
    }

    /** Refresh the cache stats (e.g. after returning from settings). */
    fun refreshCacheStats() {
        viewModelScope.launch(Dispatchers.IO) {
            try {
                val stats = parseCacheStats(RustBridge.translateCacheStats())
                _state.value = _state.value.copy(cacheStats = stats)
            } catch (_: Throwable) { /* ignore */ }
        }
    }

    private fun parseBatch(raw: JSONArray, originals: List<String>): List<TranslatedEntry> {
        val out = ArrayList<TranslatedEntry>(originals.size)
        for (i in originals.indices) {
            val obj = if (i < raw.length()) raw.optJSONObject(i) else null
            if (obj == null) {
                out.add(TranslatedEntry(originals[i], originals[i], TranslationSource.Unavailable, true))
                continue
            }
            if (obj.has("error")) {
                out.add(TranslatedEntry(originals[i], originals[i], TranslationSource.Unavailable, true))
                continue
            }
            out.add(
                TranslatedEntry(
                    original = obj.optString("original", originals[i]),
                    translated = obj.optString("translated", originals[i]),
                    source = TranslationSource.fromId(obj.optString("source")),
                    offline = obj.optBoolean("offline", false),
                ),
            )
        }
        return out
    }

    private fun parseCacheStats(json: String): CacheStats? {
        return try {
            val obj = JSONObject(json)
            CacheStats(
                root = obj.optString("root"),
                entryCount = obj.optLong("entry_count"),
                totalBytes = obj.optLong("total_bytes"),
                maxEntries = obj.optLong("max_entries"),
                maxBytes = obj.optLong("max_bytes"),
            )
        } catch (_: Throwable) {
            null
        }
    }

    companion object {
        /**
         * Resolve a [TranslationTarget] from the current UI [AppLanguage]
         * so a freshly-installed launcher starts translating into the
         * player's chosen app language by default.
         */
        fun defaultTarget(language: AppLanguage): TranslationTarget = when (language) {
            AppLanguage.ZH_CN -> TranslationTarget.ZhCn
            AppLanguage.ZH_HANT -> TranslationTarget.ZhHant
            AppLanguage.EN -> TranslationTarget.En
            AppLanguage.SYSTEM -> TranslationTarget.Auto
        }
    }
}
