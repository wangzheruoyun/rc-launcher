package com.rc.launcher.ui.model

import android.content.Context
import android.content.SharedPreferences
import com.rc.launcher.ui.model.json.JsonValue
import com.rc.launcher.ui.model.json.parseJson

/**
 * Persistence contract for custom [ControlLayout]s (task 15).
 *
 * Built-in layouts live in [ControlLayoutCatalog] and are never stored; this
 * repository only owns the user-authored (editable) layouts. The interface is
 * split from the Android [SharedPreferences] implementation so the ViewModel can
 * be unit-tested on the JVM with the [InMemoryControlLayoutRepository] (mirrors
 * the [SettingsRepository] split used by task 14).
 */
interface ControlLayoutRepository {
    /** Metadata for every saved custom layout (empty for a fresh install). */
    fun list(): List<ControlLayoutMeta>

    /** Load a saved layout by id, or null when absent / corrupt. */
    fun load(id: String): ControlLayout?

    /** Persist [layout] (upsert: created when new, replaced when existing). */
    fun save(layout: ControlLayout)

    /** Delete a saved layout. Returns true when something was removed. */
    fun delete(id: String): Boolean
}

/**
 * Volatile, process-local store used by previews and unit tests. Round-trips
 * the in-memory copy exactly, which is what the tests assert against.
 */
class InMemoryControlLayoutRepository(
    initial: List<ControlLayout> = emptyList(),
) : ControlLayoutRepository {
    private val store = LinkedHashMap<String, ControlLayout>().apply {
        for (l in initial) put(l.id, l)
    }

    override fun list(): List<ControlLayoutMeta> = store.values.map { it.meta() }

    override fun load(id: String): ControlLayout? = store[id]

    override fun save(layout: ControlLayout) {
        store[layout.id] = layout.sanitized()
    }

    override fun delete(id: String): Boolean = store.remove(id) != null
}

/**
 * [SharedPreferences]-backed [ControlLayoutRepository].
 *
 * Each layout is stored as a compact JSON string under `ctrl_layout_<id>`; an
 * index (`ctrl_layout_index`) keeps the ordered list of ids + names so the
 * picker can enumerate without parsing every layout. A single corrupt entry
 * never poisons the whole store: [load] drops a broken key and the index is
 * reconciled, satisfying task 19's graceful-degradation goal.
 */
class SharedPreferencesControlLayoutRepository(
    context: Context,
) : ControlLayoutRepository {
    private val prefs: SharedPreferences =
        context.applicationContext.getSharedPreferences(NAME, Context.MODE_PRIVATE)

    override fun list(): List<ControlLayoutMeta> = readIndex()

    override fun load(id: String): ControlLayout? {
        val raw = prefs.getString(keyFor(id), null) ?: return null
        val parsed = runCatching { parseControlLayout(raw) }.getOrNull()
        if (parsed == null) {
            // Corrupt entry: forget it so it cannot break future loads.
            prefs.edit().remove(keyFor(id)).apply()
            writeIndex(readIndex().filter { it.id != id })
        }
        return parsed
    }

    override fun save(layout: ControlLayout) {
        val clean = layout.sanitized()
        prefs.edit().putString(keyFor(clean.id), clean.toJsonString()).apply()
        val idx = readIndex().filter { it.id != clean.id } + clean.meta()
        writeIndex(idx)
    }

    override fun delete(id: String): Boolean {
        val had = prefs.contains(keyFor(id))
        prefs.edit().remove(keyFor(id)).apply()
        writeIndex(readIndex().filter { it.id != id })
        return had
    }

    // ---- index helpers ------------------------------------------------------

    private fun readIndex(): List<ControlLayoutMeta> {
        val raw = prefs.getString(KEY_INDEX, null) ?: return emptyList()
        val root = parseJson(raw) as? JsonValue.Arr ?: return emptyList()
        return root.items.mapNotNull { item ->
            if (item !is JsonValue.Obj) return@mapNotNull null
            val id = item.str("id") ?: return@mapNotNull null
            val name = item.str("name") ?: return@mapNotNull null
            ControlLayoutMeta(id, name, builtIn = false)
        }
    }

    private fun writeIndex(metas: List<ControlLayoutMeta>) {
        val arr = JsonValue.Arr(
            metas.map { m ->
                JsonValue.Obj(
                    mapOf(
                        "id" to JsonValue.Str(m.id),
                        "name" to JsonValue.Str(m.name),
                    ),
                )
            },
        )
        prefs.edit().putString(KEY_INDEX, arr.toJsonString()).apply()
    }

    private fun JsonValue.Obj.str(key: String): String? = (entries[key] as? JsonValue.Str)?.value

    private fun keyFor(id: String): String = "$KEY_PREFIX$id"

    companion object {
        private const val NAME = "rc_control_layouts"
        private const val KEY_INDEX = "ctrl_layout_index"
        private const val KEY_PREFIX = "ctrl_layout_"
    }
}

/**
 * Process-wide control-layout repository holder, mirroring [SettingsRepositories].
 * The real implementation is installed from [com.rc.launcher.RcApplication.onCreate];
 * until then (e.g. Compose previews / unit tests) a throwaway
 * [InMemoryControlLayoutRepository] is used so the UI never crashes for lack of
 * an Android context.
 */
object ControlLayoutRepositories {
    @Volatile
    private var _default: ControlLayoutRepository? = null

    val default: ControlLayoutRepository
        get() = _default ?: InMemoryControlLayoutRepository().also { _default = it }

    fun install(repository: ControlLayoutRepository) {
        _default = repository
    }
}
