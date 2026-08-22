package com.rc.launcher.ui.model

import androidx.compose.ui.graphics.Color
import kotlin.math.max

/**
 * Version-isolation strategy for an installed instance.
 *
 * Mirrors FCL's `GameDirectoryType` (FCLCore/game): each instance can either
 * share the default `.minecraft` directory, get a dedicated isolated directory
 * (so mods/configs/saves never clash between versions), or point at a fully
 * custom directory chosen by the user.
 */
enum class GameDirectoryType(val label: String, val description: String) {
    DEFAULT("默认目录", "所有实例共享 .minecraft"),
    ISOLATED("独立目录", "按实例隔离，互不干扰"),
    CUSTOM("自定义目录", "使用指定的本地路径");

    companion object {
        fun fromName(name: String?): GameDirectoryType =
            entries.firstOrNull { it.name.equals(name, ignoreCase = true) } ?: DEFAULT
    }
}

/**
 * Loader family a game instance is built on. Mirrors the install targets listed
 * in task 13 (vanilla / Forge / Fabric / Quilt / OptiFine) and the mod metadata
 * parsed by the Rust core (task 8).
 */
enum class ModLoader(val label: String, val accent: Long) {
    VANILLA("原版", 0xFF66BB6A),
    FABRIC("Fabric", 0xFF42A5F5),
    FORGE("Forge", 0xFFEF5350),
    QUILT("Quilt", 0xFFAB47BC),
    OPTIFINE("OptiFine", 0xFF8D6E63);

    /** Compose colour used for the loader badge. */
    val color: Color get() = Color(accent)

    companion object {
        fun fromName(name: String?): ModLoader =
            entries.firstOrNull { it.name.equals(name, ignoreCase = true) } ?: VANILLA
    }
}

/**
 * A locally-installed Minecraft instance. This is the dashboard's primary
 * domain object (task 12) and the persisted unit managed by the install wizard
 * and detail editor (task 13).
 *
 * It is a plain, immutable, framework-free data class so it can be unit-tested
 * on the JVM and serialised to JSON later (task 13 "settings editing & version
 * isolation"). The launch/version settings below intentionally mirror FCL's
 * `VersionSetting` + `GameDirectoryType` so the Rust-core persistence layer
 * (task 4 / 13) can round-trip them without a second model.
 *
 * @param id               stable unique id, also used as the navigation argument.
 * @param name             user-facing display name.
 * @param version          game version, e.g. "1.20.1".
 * @param modLoader        loader family (see [ModLoader]).
 * @param loaderVersion    concrete loader build, e.g. "0.16.0" for Fabric.
 * @param javaVersion      required Java major version (8/17/21, ...), or null = auto.
 * @param gameDirectoryType version-isolation strategy (see [GameDirectoryType]).
 * @param customGameDir    local path used when [GameDirectoryType.CUSTOM].
 * @param lastPlayed       epoch millis of the last launch, or 0 if never launched.
 * @param notes            free-form user note.
 * @param iconColor        ARGB seed used to generate a deterministic cover colour.
 * @param isFavorite       pinned to the top of the dashboard.
 */
data class GameInstance(
    val id: String,
    val name: String,
    val version: String,
    val modLoader: ModLoader = ModLoader.VANILLA,
    val loaderVersion: String? = null,
    val javaVersion: Int? = null,
    val gameDirectoryType: GameDirectoryType = GameDirectoryType.DEFAULT,
    val customGameDir: String? = null,
    val lastPlayed: Long = 0L,
    val notes: String = "",
    val iconColor: Long = 0xFF4CAF50,
    val isFavorite: Boolean = false,
) {
    /** Human readable loader + loader-version, e.g. "Fabric 0.16.0". */
    val loaderLabel: String
        get() = buildString {
            append(modLoader.label)
            if (loaderVersion != null) append(" $loaderVersion")
        }

    /** Human readable Java requirement, or "自动". */
    val javaLabel: String get() = javaVersion?.let { "Java $it" } ?: "自动"
}

/** Instances the user has launched at least once, most-recent first. */
fun List<GameInstance>.recentlyPlayed(limit: Int = Int.MAX_VALUE): List<GameInstance> =
    filter { it.lastPlayed > 0L }
        .sortedWith(compareByDescending<GameInstance> { it.lastPlayed })
        .take(limit)

/**
 * Dashboard ordering: favorites first, then most-recently played, then by name.
 * Keeps the most relevant instance at the top of both the home grid and the
 * instances list without any extra sorting at the call site.
 */
fun List<GameInstance>.dashboardOrder(): List<GameInstance> =
    sortedWith(
        compareByDescending<GameInstance> { it.isFavorite }
            .thenByDescending { it.lastPlayed }
            .thenBy { it.name },
    )

/** Human-readable "last played" relative to [now], in zh-CN. */
fun GameInstance.lastPlayedLabel(now: Long = System.currentTimeMillis()): String {
    if (lastPlayed <= 0L) return "从未游玩"
    val diff = max(0L, now - lastPlayed)
    val min = 60_000L
    val hour = 60 * min
    val day = 24 * hour
    return when {
        diff < min -> "刚刚"
        diff < hour -> "${diff / min} 分钟前"
        diff < day -> "${diff / hour} 小时前"
        diff < 30 * day -> "${diff / day} 天前"
        else -> "${diff / (30 * day)} 个月前"
    }
}

/**
 * Resolve the effective game directory for this instance relative to [baseDir],
 * honouring the version-isolation strategy ([GameDirectoryType]).
 *
 * - [GameDirectoryType.DEFAULT]  -> the shared base directory.
 * - [GameDirectoryType.ISOLATED]-> `<base>/instances/<id>` (fully isolated).
 * - [GameDirectoryType.CUSTOM]  -> the user-provided [customGameDir], falling
 *   back to the isolated path when it is blank.
 */
fun GameInstance.effectiveGameDir(baseDir: String): String = when (gameDirectoryType) {
    GameDirectoryType.DEFAULT -> baseDir
    GameDirectoryType.ISOLATED -> "$baseDir/instances/$id"
    GameDirectoryType.CUSTOM -> (customGameDir?.takeIf { it.isNotBlank() } ?: "$baseDir/instances/$id")
}
