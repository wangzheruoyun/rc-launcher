package com.rc.launcher.ui.model

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

/**
 * In-memory source of truth for installed game instances (task 12).
 *
 * Real persistence (JSON on disk / the Rust `game` subsystem of tasks 4 & 13)
 * plugs in behind this same surface later; the UI only depends on [instances]
 * and [recordPlayed]. A process-wide singleton keeps the home dashboard and the
 * instances list consistent without any scoping ceremony.
 */
object InstanceRepository {
    private val _instances = MutableStateFlow(seed())
    val instances: StateFlow<List<GameInstance>> = _instances.asStateFlow()

    /** Stamp [id] as just-launched so it bubbles up under "最近游玩". */
    fun recordPlayed(id: String) {
        _instances.update { list ->
            list.map { inst ->
                if (inst.id == id) inst.copy(lastPlayed = System.currentTimeMillis()) else inst
            }
        }
    }

    /** Add (or replace by id) an instance. Used by task 13 later. */
    fun add(instance: GameInstance) {
        _instances.update { list ->
            (list.filter { it.id != instance.id } + instance).distinctBy { it.id }
        }
    }

    /** Remove an instance by id. */
    fun remove(id: String) {
        _instances.update { list -> list.filter { it.id != id } }
    }

    /** Persist edits to an existing instance (replace by id). */
    fun update(instance: GameInstance) {
        _instances.update { list -> list.map { if (it.id == instance.id) instance else it } }
    }

    /** Look up a single instance by id, or null. */
    fun getById(id: String): GameInstance? = _instances.value.firstOrNull { it.id == id }

    /** Test-only: replace the whole list (e.g. to reset between test cases). */
    fun replaceAll(list: List<GameInstance>) {
        _instances.value = list
    }

    private fun seed(): List<GameInstance> {
        val now = System.currentTimeMillis()
        val min = 60_000L
        return listOf(
            GameInstance(
                id = "vanilla-1.20.1",
                name = "我的世界 1.20.1",
                version = "1.20.1",
                modLoader = ModLoader.VANILLA,
                lastPlayed = now - 2 * 60 * min,
                iconColor = 0xFF66BB6A,
            ),
            GameInstance(
                id = "fabric-1.20.1",
                name = "Fabric 整合包 1.20.1",
                version = "1.20.1",
                modLoader = ModLoader.FABRIC,
                lastPlayed = now - 3 * 24 * 60 * min,
                iconColor = 0xFF42A5F5,
            ),
            GameInstance(
                id = "forge-1.12.2",
                name = "Forge 1.12.2（整合示例）",
                version = "1.12.2",
                modLoader = ModLoader.FORGE,
                lastPlayed = 0L,
                iconColor = 0xFFEF5350,
            ),
            GameInstance(
                id = "quilt-1.20.4",
                name = "Quilt 1.20.4",
                version = "1.20.4",
                modLoader = ModLoader.QUILT,
                lastPlayed = now - 5 * min,
                iconColor = 0xFFAB47BC,
            ),
            GameInstance(
                id = "optifine-1.20.2",
                name = "OptiFine 1.20.2",
                version = "1.20.2",
                modLoader = ModLoader.OPTIFINE,
                lastPlayed = 0L,
                iconColor = 0xFF8D6E63,
                isFavorite = true,
            ),
        )
    }
}
