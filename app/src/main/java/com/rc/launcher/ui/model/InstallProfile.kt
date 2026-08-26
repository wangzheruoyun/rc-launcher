package com.rc.launcher.ui.model

/** Hard cap on an instance display name, to keep derived file-system ids sane. */
const val MAX_INSTANCE_NAME_LENGTH: Int = 64

/** Java major versions the launcher can actually provision (tasks 12/13). */
val SUPPORTED_JAVA_VERSIONS: Set<Int> = setOf(8, 17, 21)

/**
 * Data model for the version-installation wizard (task 13).
 *
 * The wizard walks the user through choosing a loader family, a game version and
 * (for modded loaders) a concrete loader build, then per-instance configuration
 * such as version isolation. Everything here is plain, framework-free Kotlin so
 * it can be unit-tested on the JVM and later persisted/validated by the Rust
 * core (task 4). The structure intentionally mirrors FCLCore/game's
 * `VersionJson`/`ResolvedVersion` (loader + game version + inheritsFrom-style
 * profile) and Zalith's version-management models.
 */

/** A concrete loader build available for a given game version. */
data class LoaderVersion(
    /** Unique id, e.g. "fabric-loader-0.16.0-1.20.1". */
    val id: String,
    /** Human version string, e.g. "0.16.0". */
    val version: String,
    /** Game version this build targets, e.g. "1.20.1". */
    val gameVersion: String,
    /** Whether this is a stable (vs. experimental/nightly) build. */
    val stable: Boolean = true,
) {
    /** Display label shown in the loader-version picker. */
    val displayName: String get() = buildString {
        append(version)
        if (!stable) append(" (实验)")
    }
}

/**
 * Built-in loader-version catalogue.
 *
 * In production this is fed by the Rust core (task 4) which queries the official
 * Forge/Fabric/Quilt/OptiFine metadata endpoints through the China-optimised
 * mirror client (task 2/3). The static fallback below keeps the wizard fully
 * functional offline and gives the UI something deterministic to render & test.
 */
object LoaderCatalog {
    /** Recent vanilla releases, newest first. Used by the game-version picker. */
    val gameVersions: List<String> = listOf(
        "1.21.1", "1.21", "1.20.6", "1.20.4", "1.20.1",
        "1.19.4", "1.19.2", "1.18.2", "1.16.5", "1.12.2", "1.7.10",
    )

    /**
     * Loader builds available for ([loader], [gameVersion]).
     * Returns an empty list for vanilla (no separate loader build).
     */
    fun loaderVersions(loader: ModLoader, gameVersion: String): List<LoaderVersion> {
        if (loader == ModLoader.VANILLA) return emptyList()
        return when (loader) {
            ModLoader.FABRIC -> fabricFor(gameVersion)
            ModLoader.QUILT -> quiltFor(gameVersion)
            ModLoader.FORGE -> forgeFor(gameVersion)
            ModLoader.OPTIFINE -> optifineFor(gameVersion)
            else -> emptyList()
        }
    }

    private fun fabricFor(gv: String): List<LoaderVersion> =
        listOf("0.16.0", "0.15.11", "0.15.7", "0.14.25").map { v ->
            LoaderVersion("fabric-$v-$gv", v, gv, stable = true)
        }

    private fun quiltFor(gv: String): List<LoaderVersion> =
        listOf("0.25.0", "0.24.0", "0.23.1", "0.22.0").map { v ->
            LoaderVersion("quilt-$v-$gv", v, gv, stable = true)
        }

    private fun forgeFor(gv: String): List<LoaderVersion> =
        listOf("latest", "recommended").mapIndexed { idx, channel ->
            val v = when (gv) {
                "1.20.1" -> "47.3.0"
                "1.20.4" -> "49.0.0"
                "1.19.2" -> "43.4.0"
                "1.18.2" -> "40.2.0"
                "1.16.5" -> "36.2.0"
                "1.12.2" -> "14.23.5"
                else -> "0.0.0"
            }
            LoaderVersion("forge-$v-$gv", if (v == "0.0.0") channel else v, gv, stable = idx == 0)
        }

    private fun optifineFor(gv: String): List<LoaderVersion> =
        listOf("HD_U_I6", "HD_U_I5", "HD_U_H9").map { v ->
            LoaderVersion("optifine-$v-$gv", v, gv, stable = true)
        }
}

/**
 * A single wizard step. The linear flow is [LOADER] -> [GAME_VERSION] ->
 * ([LOADER_VERSION] for modded loaders) -> [CONFIGURE] -> [REVIEW].
 */
enum class InstallStep {
    LOADER,
    GAME_VERSION,
    LOADER_VERSION,
    CONFIGURE,
    REVIEW,
}

/** The full, mutable state collected by the install wizard. */
data class InstallRequest(
    val loader: ModLoader = ModLoader.VANILLA,
    val gameVersion: String = "",
    val loaderVersion: LoaderVersion? = null,
    val name: String = "",
    val iconColor: Long = 0xFF4CAF50,
    val notes: String = "",
    val javaVersion: Int? = null,
    val gameDirectoryType: GameDirectoryType = GameDirectoryType.ISOLATED,
    val customGameDir: String = "",
) {
    /** Modded loaders need an extra loader-version selection step. */
    val requiresLoaderVersion: Boolean get() = loader != ModLoader.VANILLA

    /** First validation error, or null when the request is complete & valid. */
    fun validationError(): String? = when {
        gameVersion.isBlank() -> "请选择游戏版本"
        requiresLoaderVersion && loaderVersion == null -> "请选择 ${loader.label} 版本"
        name.isBlank() -> "请填写实例名称"
        name.length > MAX_INSTANCE_NAME_LENGTH ->
            "实例名称过长（最多 ${MAX_INSTANCE_NAME_LENGTH} 字）"
        gameDirectoryType == GameDirectoryType.CUSTOM && customGameDir.isBlank() ->
            "自定义目录不能为空"
        javaVersion != null && javaVersion !in SUPPORTED_JAVA_VERSIONS ->
            "Java 版本不受支持"
        else -> null
    }

    val isValid: Boolean get() = validationError() == null

    /** Build the immutable [GameInstance]; defaults fill in any blank fields. */
    fun buildInstance(id: String = defaultId()): GameInstance = GameInstance(
        id = id,
        name = name.trim().ifBlank { defaultName() },
        version = gameVersion.trim(),
        modLoader = loader,
        loaderVersion = loaderVersion?.version,
        notes = notes.trim(),
        iconColor = iconColor,
        javaVersion = javaVersion,
        gameDirectoryType = gameDirectoryType,
        customGameDir = customGameDir.trim().ifBlank { null },
    )

    private fun defaultName(): String =
        buildString {
            append(loader.label)
            append(' ')
            append(gameVersion)
            loaderVersion?.let { append(" (${it.version})") }
        }

    /** Deterministic, filesystem-safe id derived from the selection. */
    fun defaultId(): String {
        val base = "${loader.name.lowercase()}-$gameVersion"
        val suffix = loaderVersion?.version?.let { "-$it" } ?: ""
        return (base + suffix).replace(Regex("[^a-z0-9.-]"), "")
    }
}

/** Advance to the next wizard step, skipping the loader step for vanilla. */
fun InstallStep.next(request: InstallRequest): InstallStep? = when (this) {
    InstallStep.LOADER -> InstallStep.GAME_VERSION
    InstallStep.GAME_VERSION ->
        if (request.requiresLoaderVersion) InstallStep.LOADER_VERSION else InstallStep.CONFIGURE
    InstallStep.LOADER_VERSION -> InstallStep.CONFIGURE
    InstallStep.CONFIGURE -> InstallStep.REVIEW
    InstallStep.REVIEW -> null
}

/** Step back, mirroring [next] (so vanilla never visits the loader step). */
fun InstallStep.previous(request: InstallRequest): InstallStep? = when (this) {
    InstallStep.LOADER -> null
    InstallStep.GAME_VERSION -> InstallStep.LOADER
    InstallStep.LOADER_VERSION -> InstallStep.GAME_VERSION
    InstallStep.CONFIGURE ->
        if (request.requiresLoaderVersion) InstallStep.LOADER_VERSION else InstallStep.GAME_VERSION
    InstallStep.REVIEW -> InstallStep.CONFIGURE
}

/** Whether the user may advance from [this] step given [request]. */
fun InstallStep.canProceed(request: InstallRequest): Boolean = when (this) {
    InstallStep.LOADER -> true
    InstallStep.GAME_VERSION -> request.gameVersion.isNotBlank()
    InstallStep.LOADER_VERSION -> request.loaderVersion != null
    InstallStep.CONFIGURE -> request.name.isNotBlank()
    InstallStep.REVIEW -> request.isValid
}

/** 1-based position of this step in the visible flow for [request]. */
fun InstallStep.stepNumber(request: InstallRequest): Int = when (this) {
    InstallStep.LOADER -> 1
    InstallStep.GAME_VERSION -> 2
    InstallStep.LOADER_VERSION -> 3
    InstallStep.CONFIGURE -> if (request.requiresLoaderVersion) 4 else 3
    InstallStep.REVIEW -> if (request.requiresLoaderVersion) 5 else 4
}

/** Total number of visible steps for [request] (4 for vanilla, 5 for modded). */
fun InstallStep.totalSteps(request: InstallRequest): Int =
    if (request.requiresLoaderVersion) 5 else 4
