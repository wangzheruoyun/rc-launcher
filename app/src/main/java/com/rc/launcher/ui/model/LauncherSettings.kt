package com.rc.launcher.ui.model

import kotlin.math.roundToInt

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

        /** Heuristic auto heap: ~1/3 of device RAM, clamped to [MIN..MAX_HEAP_MB]. */
        fun autoHeapFor(deviceTotalMb: Int): Int {
            if (deviceTotalMb <= 0) return DEFAULT_HEAP_MB
            return (deviceTotalMb * 0.33).toInt()
                .coerceIn(MIN_HEAP_MB, MAX_HEAP_MB.coerceAtMost(deviceTotalMb))
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
}
