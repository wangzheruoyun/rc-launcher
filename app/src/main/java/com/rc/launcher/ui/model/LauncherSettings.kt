package com.rc.launcher.ui.model

import kotlin.math.roundToInt
import kotlin.comparisons.minOf

/**
 * A download mirror usable for the China mainland (task 3 / task 14).
 *
 * Mirrors the Rust core's `crate::net::mirror::MirrorSource`: a *path-preserving*
 * rewrite of a canonical Mojang CDN URL onto a domestic host. The UI only ever
 * stores the [id]; the Rust core ([RustBridge.getDefaultMirrors]) owns the real
 * rewrite logic and the reachability probing.
 */
data class MirrorSource(
    val id: String,
    val name: String,
    val baseUrl: String,
    val description: String = "",
    /** True for the pseudo "connect directly to Mojang" choice (no rewrite). */
    val official: Boolean = false,
)

/** Built-in mirror catalogue, kept in lock-step with the Rust core defaults. */
object MirrorCatalog {
    val OFFICIAL = MirrorSource(
        "official", "官方直连", "",
        "直接连接 Mojang 官方源（不推荐，国内通常较慢）", official = true,
    )
    val BMCLAPI = MirrorSource(
        "bmclapi", "BMCLAPI", "https://bmclapi2.bangbang93.com",
        "BMCLAPI 镜像（bangbang93）",
    )
    val MCBBS = MirrorSource(
        "mcbbs", "MCBBS", "https://download.mcbbs.net",
        "我的世界中文论坛镜像",
    )
    val ALIYUN = MirrorSource(
        "aliyun", "Aliyun", "https://mirrors.aliyun.com/minecraft",
        "阿里云开源镜像站",
    )

    /** All selectable mirrors, "official" first for the picker. */
    val all: List<MirrorSource> = listOf(OFFICIAL, BMCLAPI, MCBBS, ALIYUN)

    fun fromId(id: String?): MirrorSource = all.firstOrNull { it.id == id } ?: BMCLAPI
    fun isValidId(id: String?): Boolean = id != null && all.any { it.id == id }
}

/** A DNS-over-HTTPS upstream, matching the Rust core's [RustBridge.getDefaultDohServers]. */
data class DohServer(val id: String, val name: String, val url: String)

/** Built-in DoH catalogue (Aliyun / DNSPod / 360 / Cloudflare / Google). */
object DohCatalog {
    val ALIYUN = DohServer("aliyun", "阿里 DNS", "https://dns.aliyun.com/dns-query")
    val DNSPOD = DohServer("dnspod", "DNSPod", "https://doh.pub/dns-query")
    val QIHOO360 = DohServer("360", "360 DNS", "https://doh.360.cn/dns-query")
    val CLOUDFLARE = DohServer("cloudflare", "Cloudflare", "https://cloudflare-dns.com/dns-query")
    val GOOGLE = DohServer("google", "Google", "https://dns.google/dns-query")

    val all: List<DohServer> = listOf(ALIYUN, DNSPOD, QIHOO360, CLOUDFLARE, GOOGLE)

    fun fromUrl(url: String?): DohServer = all.firstOrNull { it.url == url } ?: ALIYUN
    fun isValidUrl(url: String?): Boolean = url != null && all.any { it.url == url }
}

/**
 * A selectable OpenGL(ES) translation stack (task 9 / task 14).
 *
 * The [id] / [glLibname] values mirror the Rust core's `Renderer` enum so the
 * launch engine can consume the persisted choice directly (task 7).
 */
enum class RendererOption(
    val id: String,
    val label: String,
    val description: String,
    val glLibname: String,
) {
    GL4ES(
        "opengles2", "GL4ES",
        "默认，兼容性最好的 OpenGL→GLES 转译层（libgl4es_114.so）",
        "libgl4es_114.so",
    ),
    NG_GL4ES(
        "opengles2_ng", "NG-GL4ES",
        "较新的 GL4ES 分支，着色器支持更好（libng_gl4es.so）",
        "libng_gl4es.so",
    ),
    VIRGL(
        "opengles2_vgpu", "VirGL",
        "虚拟化的 Gallium 驱动，适合高性能设备（libvgpu.so）",
        "libvgpu.so",
    ),
    ZINK(
        "opengles3_desktopgl_zink_kopper", "Zink",
        "Mesa 上以 Vulkan 实现桌面 OpenGL（libOSMesa_8.so）",
        "libOSMesa_8.so",
    ),
    ANGLE(
        "opengles3_angle", "ANGLE",
        "基于 Vulkan 的 GLES 实现，性能稳定（libGLESv2_angle.so）",
        "libGLESv2_angle.so",
    );

    companion object {
        val DEFAULT = GL4ES
        fun fromId(id: String?): RendererOption = entries.firstOrNull { it.id == id } ?: DEFAULT
        fun isValidId(id: String?): Boolean = id != null && entries.any { it.id == id }
    }
}

/** Windowing / resolution strategy for the game surface. */
enum class ResolutionMode(val label: String) {
    AUTO("自动"),
    CUSTOM("自定义");

    companion object {
        fun fromName(name: String?): ResolutionMode =
            entries.firstOrNull { it.name == name } ?: AUTO
    }
}

/**
 * Game window / surface size. Non-zero, clamped to sane bounds (mirrors the
 * Rust core's `WindowSize`).
 */
data class WindowSize(val width: Int, val height: Int) {
    init {
        require(width in MIN_W..MAX_W && height in MIN_H..MAX_H) {
            "Resolution out of range: ${width}x${height}"
        }
    }

    val label: String get() = "${width}x${height}"

    /** Scale by [scale] (mimics FCL's resolution scaler), clamped to bounds. */
    fun scaled(scale: Float): WindowSize {
        val w = (width * scale).roundToInt().coerceIn(MIN_W, MAX_W)
        val h = (height * scale).roundToInt().coerceIn(MIN_H, MAX_H)
        return WindowSize(w, h)
    }

    companion object {
        const val MIN_W = 320
        const val MAX_W = 7680
        const val MIN_H = 240
        const val MAX_H = 4320
        val DEFAULT = WindowSize(1280, 720)
    }
}

/**
 * Global launcher settings (task 14: Settings Center).
 *
 * A plain, immutable, framework-free data class so it round-trips through the
 * Rust core's launch options (task 7) and is unit-testable on the JVM. Every
 * field carries a safe default; [sanitized] clamps / repairs user input and
 * never throws, which is the robustness contract required by task 19.
 *
 * Persisted by [SettingsRepository] (one key per primitive). Only the *ids* of
 * mirrors / renderers / DoH servers are stored — the catalogues above are the
 * single source of truth for their human-readable metadata.
 */
data class LauncherSettings(
    // --- Network / China optimisation ---
    val mirrorId: String = MirrorCatalog.BMCLAPI.id,
    val autoSelectFastestMirror: Boolean = true,
    val useDoh: Boolean = true,
    val dohServerUrl: String = DohCatalog.ALIYUN.url,

    // --- Java / memory ---
    val javaHeapMb: Int = DEFAULT_HEAP_MB,
    val javaMinHeapMb: Int? = null,
    val autoAllocateMemory: Boolean = true,
    val javaVersion: Int? = null,
    val javaArgs: String = "",

    // --- Renderer / window ---
    val rendererId: String = RendererOption.DEFAULT.id,
    val resolutionMode: ResolutionMode = ResolutionMode.AUTO,
    val customWidth: Int = WindowSize.DEFAULT.width,
    val customHeight: Int = WindowSize.DEFAULT.height,
    val resolutionScale: Float = 1f,
    val framerateLimit: Int = 0,
    val fullscreen: Boolean = false,

    // --- Controller ---
    val controllerEnabled: Boolean = false,
    val controllerLayoutId: String = CONTROLLER_LAYOUT_DEFAULT,
    val controllerDeadzone: Float = 0.15f,
    val controllerVibration: Boolean = true,

    // --- Directory / misc ---
    val gameFilesRoot: String = "",
    val autoCleanLogs: Boolean = true,
    val keepCrashReports: Boolean = false,
    val rendererOptions: RendererPluginConfig = RendererPluginConfig(),
) {
    companion object {
        const val DEFAULT_HEAP_MB = 1024
        const val MIN_HEAP_MB = 256
        const val MAX_HEAP_MB = 8192
        const val MIN_FRAMERATE = 0 // 0 = unlimited
        const val MAX_FRAMERATE = 240
        const val MIN_SCALE = 0.25f
        const val MAX_SCALE = 2f
        const val CONTROLLER_LAYOUT_DEFAULT = "default"

        /** Heuristic auto heap: ~1/3 of device RAM, clamped to [MIN_HEAP_MB]..[MAX_HEAP_MB]. */
        fun autoHeapFor(deviceTotalMb: Int): Int {
            if (deviceTotalMb <= 0) return DEFAULT_HEAP_MB
            return (deviceTotalMb * 0.33).toInt()
                .coerceIn(MIN_HEAP_MB, MAX_HEAP_MB.coerceAtMost(deviceTotalMb))
        }

        /**
         * Parse a [toBackupString] payload back into [LauncherSettings].
         *
         * Unknown / malformed lines are ignored, missing keys fall back to the
         * defaults, and the reconstructed value always runs through
         * [sanitized] so a tampered backup can never inject an out-of-range
         * value into the Rust core (task 19 robustness). Returns `null` only
         * when the whole payload is structurally unusable.
         */
        fun fromBackupString(text: String): LauncherSettings? {
            val map = LinkedHashMap<String, String>()
            for (raw in text.lineSequence()) {
                val line = raw.trimEnd('\r')
                if (line.isBlank() || line.startsWith('#')) continue
                val eq = line.indexOf('=')
                if (eq <= 0) continue
                val key = line.substring(0, eq).trim()
                val value = line.substring(eq + 1)
                map[key] = value
            }
            val str = { k: String, d: String -> map[k] ?: d }
            val bool = { k: String, d: Boolean ->
                when (map[k]) {
                    "true" -> true
                    "false" -> false
                    else -> d
                }
            }
            val int = { k: String, d: Int -> map[k]?.toIntOrNull() ?: d }
            val intOrNull = { k: String -> map[k]?.toIntOrNull() }
            val float = { k: String, d: Float -> map[k]?.toFloatOrNull() ?: d }
            return try {
                LauncherSettings(
                    mirrorId = str("mirrorId", MirrorCatalog.BMCLAPI.id),
                    autoSelectFastestMirror = bool("autoSelectFastestMirror", true),
                    useDoh = bool("useDoh", true),
                    dohServerUrl = str("dohServerUrl", DohCatalog.ALIYUN.url),
                    javaHeapMb = int("javaHeapMb", DEFAULT_HEAP_MB),
                    javaMinHeapMb = intOrNull("javaMinHeapMb"),
                    autoAllocateMemory = bool("autoAllocateMemory", true),
                    javaVersion = intOrNull("javaVersion"),
                    javaArgs = str("javaArgs", ""),
                    rendererId = str("rendererId", RendererOption.DEFAULT.id),
                    resolutionMode = ResolutionMode.fromName(map["resolutionMode"]),
                    customWidth = int("customWidth", WindowSize.DEFAULT.width),
                    customHeight = int("customHeight", WindowSize.DEFAULT.height),
                    resolutionScale = float("resolutionScale", 1f),
                    framerateLimit = int("framerateLimit", 0),
                    fullscreen = bool("fullscreen", false),
                    controllerEnabled = bool("controllerEnabled", false),
                    controllerLayoutId = str("controllerLayoutId", CONTROLLER_LAYOUT_DEFAULT),
                    controllerDeadzone = float("controllerDeadzone", 0.15f),
                    controllerVibration = bool("controllerVibration", true),
                    gameFilesRoot = str("gameFilesRoot", ""),
                    autoCleanLogs = bool("autoCleanLogs", true),
                    keepCrashReports = bool("keepCrashReports", false),
                    rendererOptions = RendererPluginConfig(
                        zinkVulkanDriver = str("renderer.zinkVulkanDriver", RendererPluginConfig.DEFAULT_ZINK_DRIVER),
                        angleBackend = str("renderer.angleBackend", RendererPluginConfig.DEFAULT_ANGLE_BACKEND),
                        gl4esNoSrgb = bool("renderer.gl4esNoSrgb", false),
                        virglServer = str("renderer.virglServer", ""),
                    ),
                ).sanitized()
            } catch (_: Exception) {
                null
            }
        }
    }

    /** First human-readable validation problem, or null when everything is OK. */
    fun validationError(): String? = when {
        !MirrorCatalog.isValidId(mirrorId) -> "镜像源无效"
        useDoh && !DohCatalog.isValidUrl(dohServerUrl) -> "DoH 服务器地址无效"
        !RendererOption.isValidId(rendererId) -> "渲染器无效"
        javaHeapMb !in MIN_HEAP_MB..MAX_HEAP_MB -> "Java 最大内存超出范围"
        javaMinHeapMb != null && javaMinHeapMb > javaHeapMb -> "Java 初始内存不能大于最大内存"
        javaVersion != null && javaVersion < 8 -> "Java 版本无效"
        resolutionMode == ResolutionMode.CUSTOM &&
            (customWidth !in WindowSize.MIN_W..WindowSize.MAX_W ||
                customHeight !in WindowSize.MIN_H..WindowSize.MAX_H) -> "自定义分辨率超出范围"
        resolutionScale !in MIN_SCALE..MAX_SCALE -> "分辨率缩放超出范围"
        framerateLimit !in MIN_FRAMERATE..MAX_FRAMERATE -> "帧率限制超出范围"
        controllerDeadzone !in 0f..1f -> "手柄死区超出范围"
        else -> null
    }

    /** Clamp / repair every field, returning a safe copy (never throws). */
    fun sanitized(): LauncherSettings {
        // Clamp the max heap first; the initial heap must never exceed it.
        val heap = javaHeapMb.coerceIn(MIN_HEAP_MB, MAX_HEAP_MB)
        return copy(
            mirrorId = if (MirrorCatalog.isValidId(mirrorId)) mirrorId else MirrorCatalog.BMCLAPI.id,
            dohServerUrl = if (useDoh && DohCatalog.isValidUrl(dohServerUrl)) {
                dohServerUrl
            } else {
                DohCatalog.ALIYUN.url
            },
            javaHeapMb = heap,
            javaMinHeapMb = javaMinHeapMb?.coerceIn(0, heap)?.takeIf { it > 0 },
            javaVersion = javaVersion?.coerceAtLeast(8),
            rendererId = if (RendererOption.isValidId(rendererId)) rendererId else RendererOption.DEFAULT.id,
            customWidth = customWidth.coerceIn(WindowSize.MIN_W, WindowSize.MAX_W),
            customHeight = customHeight.coerceIn(WindowSize.MIN_H, WindowSize.MAX_H),
            resolutionScale = resolutionScale.coerceIn(MIN_SCALE, MAX_SCALE),
            framerateLimit = framerateLimit.coerceIn(MIN_FRAMERATE, MAX_FRAMERATE),
            controllerDeadzone = controllerDeadzone.coerceIn(0f, 1f),
            rendererOptions = rendererOptions.sanitized(),
        )
    }

    /** Effective heap in MiB, honouring [autoAllocateMemory]. */
    fun effectiveHeapMb(deviceTotalMb: Int = 0): Int =
        if (autoAllocateMemory) autoHeapFor(deviceTotalMb) else javaHeapMb

    /** The chosen mirror, or null when connecting directly to Mojang. */
    fun mirror(): MirrorSource? = MirrorCatalog.fromId(mirrorId).takeIf { !it.official }

    fun renderer(): RendererOption = RendererOption.fromId(rendererId)

    /** Resolved window size (custom size only applies in [ResolutionMode.CUSTOM]). */
    fun windowSize(): WindowSize = if (resolutionMode == ResolutionMode.CUSTOM) {
        WindowSize(
            customWidth.coerceIn(WindowSize.MIN_W, WindowSize.MAX_W),
            customHeight.coerceIn(WindowSize.MIN_H, WindowSize.MAX_H),
        )
    } else {
        WindowSize.DEFAULT
    }

    /**
     * Serialize every preference to a stable, human-readable, line-based format
     * (`key=value`, one per line, nested renderer options under `renderer.*`).
     * Round-trips through [fromBackupString] for the Settings Center backup /
     * restore feature (task 14).
     */
    fun toBackupString(): String = buildString {
        val s = this@LauncherSettings
        appendLine("mirrorId=" + s.mirrorId)
        appendLine("autoSelectFastestMirror=" + s.autoSelectFastestMirror)
        appendLine("useDoh=" + s.useDoh)
        appendLine("dohServerUrl=" + s.dohServerUrl)
        appendLine("javaHeapMb=" + s.javaHeapMb)
        appendLine("javaMinHeapMb=" + (s.javaMinHeapMb ?: ""))
        appendLine("autoAllocateMemory=" + s.autoAllocateMemory)
        appendLine("javaVersion=" + (s.javaVersion ?: ""))
        appendLine("javaArgs=" + s.javaArgs)
        appendLine("rendererId=" + s.rendererId)
        appendLine("resolutionMode=" + s.resolutionMode.name)
        appendLine("customWidth=" + s.customWidth)
        appendLine("customHeight=" + s.customHeight)
        appendLine("resolutionScale=" + s.resolutionScale)
        appendLine("framerateLimit=" + s.framerateLimit)
        appendLine("fullscreen=" + s.fullscreen)
        appendLine("controllerEnabled=" + s.controllerEnabled)
        appendLine("controllerLayoutId=" + s.controllerLayoutId)
        appendLine("controllerDeadzone=" + s.controllerDeadzone)
        appendLine("controllerVibration=" + s.controllerVibration)
        appendLine("gameFilesRoot=" + s.gameFilesRoot)
        appendLine("autoCleanLogs=" + s.autoCleanLogs)
        appendLine("keepCrashReports=" + s.keepCrashReports)
        appendLine("renderer.zinkVulkanDriver=" + s.rendererOptions.zinkVulkanDriver)
        appendLine("renderer.angleBackend=" + s.rendererOptions.angleBackend)
        appendLine("renderer.gl4esNoSrgb=" + s.rendererOptions.gl4esNoSrgb)
        appendLine("renderer.virglServer=" + s.rendererOptions.virglServer)
    }
}

/**
 * Per-renderer plugin tuning (task 14 extension).
 *
 * Closes the gap called out in the Settings Center scope ("对应 ... 各
 * RendererPlugin 配置项"): the screen used to only pick a renderer from a
 * dropdown, but never exposed the renderer-specific options that FCL's
 * renderer plugins and the bundled native libraries
 * ([FCL_NATIVE_LIBRARIES.md]) actually support:
 *
 *  - **Zink** (`libzink_dri.so` + `libOSMesa_8.so`): which Vulkan driver to
 *    bind — Turnip (Adreno) or Freedreno.
 *  - **ANGLE** (`libGLESv2_angle.so` + `libEGL_angle.so`): rendering backend
 *    (Vulkan / OpenGL / disabled).
 *  - **GL4ES** (`libgl4es_114.so` / `libng_gl4es.so`): toggle the sRGB
 *    emulation that breaks some drivers / mods.
 *  - **VirGL** (`libvgpu.so`): optional remote `virglrenderer` server.
 *
 * String-backed (like [LauncherSettings.mirrorId]) so it serialises cleanly
 * and the catalogue can be extended without breaking persistence.
 */
data class RendererPluginConfig(
    val zinkVulkanDriver: String = DEFAULT_ZINK_DRIVER,
    val angleBackend: String = DEFAULT_ANGLE_BACKEND,
    val gl4esNoSrgb: Boolean = false,
    val virglServer: String = "",
) {
    /** Trim free-form text fields; never throws. */
    fun sanitized(): RendererPluginConfig = copy(virglServer = virglServer.trim())

    companion object {
        const val DEFAULT_ZINK_DRIVER = "auto"
        const val DEFAULT_ANGLE_BACKEND = "vulkan"

        /** Zink Vulkan driver choices (id to human label). */
        val ZINK_DRIVERS: List<Pair<String, String>> = listOf(
            "auto" to "自动",
            "turnip" to "Turnip (Adreno)",
            "freedreno" to "Freedreno",
        )

        /** ANGLE rendering backends (id to human label). */
        val ANGLE_BACKENDS: List<Pair<String, String>> = listOf(
            "vulkan" to "Vulkan",
            "gl" to "OpenGL",
            "null" to "关闭（仅测试）",
        )

        fun isValidZinkDriver(id: String?): Boolean =
            id != null && ZINK_DRIVERS.any { it.first == id }

        fun isValidAngleBackend(id: String?): Boolean =
            id != null && ANGLE_BACKENDS.any { it.first == id }
    }
}
