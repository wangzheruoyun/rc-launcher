package com.rc.launcher.core

import org.json.JSONObject

/**
 * JNI bridge to the Rust core ([librc_launcher.so]).
 *
 * The native library is shipped inside the :core AAR (compiled from the Cargo
 * workspace via cargo-ndk) and loaded once on first access. Compose UI in :app
 * talks to the core exclusively through this object, keeping all native / unsafe
 * concerns isolated in :core.
 */
object RustBridge {
    init {
        System.loadLibrary("rc_launcher")
    }

    /** Returns the Rust core version string. */
    external fun getVersion(): String

    /** Simple echo used to validate the JNI boundary end-to-end. */
    external fun greet(name: String): String

    /**
     * Returns the built-in mirror list (task 3) as a JSON string. Each entry has
     * `id`, `name`, `base_url`, `path_prefix` and `hosts`.
     */
    external fun getDefaultMirrors(): String

    /**
     * Returns the built-in DNS-over-HTTPS upstream list (task 3) as a JSON
     * string of URLs (Aliyun / DNSPod / 360 / Cloudflare / Google).
     */
    /**
     * Returns the built-in DNS-over-HTTPS upstream list (task 3) as a JSON
     * string of URLs (Aliyun / DNSPod / 360 / Cloudflare / Google).
     */
    external fun getDefaultDohServers(): String

    // === Account & authentication (task 5) ==================================
    //
    // All methods exchange JSON strings. The Microsoft login flow is split so
    // the UI stays responsive:
    //   1. `authBeginMicrosoft()`   -> device-code challenge JSON (show `message`).
    //   2. `authCompleteMicrosoft(challengeJson)` -> account JSON (blocks while
    //      the user signs in; MUST be called from a background thread / coroutine).
    //
    // Secure storage: on Android, derive a 32-byte AES key from the
    // **Android Keystore** ("AndroidKeyStore" entry) and pass it hex-encoded as
    // `key_hex` to `authInit` together with an on-disk `path`. The Rust core
    // then encrypts the token database with AES-256-GCM under that key, so the
    // key never leaves Keystore and the file is useless without it.

    /**
     * (Re)configure the global account store.
     * `configJson = {"path"?:string,"key_hex"?:string,"client_id"?:string,
     *   "redirect_uri"?:string,"proxy"?:string}`.
     *
     * `redirect_uri` (task 28) sets a custom callback address for the Microsoft
     * browser redirect; the embedded `microsoft_auth.html` asset page can serve
     * as this address so users can complete sign-in via the in-app callback
     * page. When omitted, Microsoft uses its default redirect behaviour.
     *
     * `proxy` (task 28) optionally configures an HTTP/HTTPS/SOCKS5 proxy for
     * the auth transport so Microsoft/Xbox/Mojang token-exchange calls can
     * reach the network through the Great Firewall.
     */
    external fun authInit(configJson: String): String

    /**
     * Returns the default `redirect_uri` for the embedded callback page
     * (`file:///android_asset/microsoft_auth.html`) — task 28.
     */
    external fun authDefaultRedirectUri(): String

    /**
     * Returns the embedded `microsoft_auth.html` callback page content,
     * translated to the current UI language — task 28. The caller can write
     * this string to `assets/microsoft_auth.html` at first launch to guarantee
     * the callback asset exists even when the APK build omits it.
     */
    external fun authGetCallbackHtml(): String

    /** JSON array of redacted accounts (no secrets). */
    external fun authListAccounts(): String

    /** Add an offline account. Returns the account JSON (or `{"error":...}`). */
    external fun authAddOfflineAccount(name: String): String

    /** Begin Microsoft device-code login. Returns the challenge JSON.
     *  The challenge includes `redirect_uri` when one was configured via
     *  `authInit` (task 28). */
    external fun authBeginMicrosoft(): String

    /**
     * Complete Microsoft login (blocks; background thread). Returns account JSON
     * or `{"error":...,"cn_fallback_hint":...}` on failure. The
     * `cn_fallback_hint` is populated on network-level errors so the UI can
     * suggest a proxy / mirror for mainland-China players (task 28).
     */
    external fun authCompleteMicrosoft(challengeJson: String): String

    /** Remove an account by uuid. Returns `{"removed":bool}`. */
    external fun authRemoveAccount(uuid: String): String

    /**
     * Force-refresh a Microsoft account's token. Returns account JSON or
     * `{"error":...,"cn_fallback_hint":...}` on network failure (task 28).
     */
    external fun authRefreshAccount(uuid: String): String

    /**
     * Return a fresh account, transparently refreshing if the token is
     * expiring. Returns account JSON or `{"error":...,"cn_fallback_hint":...}`
     * on network failure (task 28).
     */
    external fun authEnsureFresh(uuid: String): String

    /**
     * Discover an external (third-party) auth server's metadata by URL (task 10).
     * Returns the server-info JSON (or `{"error":...}` with an optional
     * `cn_fallback_hint` when the lookup failed on the mainland-China network).
     */
    external fun authBeginThirdParty(serverUrl: String): String

    /**
     * Complete a third-party login (Authlib-Injector / token relay). `loginJson`
     * is the `ThirdPartyLogin` payload. Blocks while authenticating; call from a
     * background thread. Returns the account JSON (or `{"error":...,
     * "cn_fallback_hint":...}` on a network failure).
     */
    external fun authCompleteThirdParty(loginJson: String): String

    // === Skin preview (task 22) ==============================================
    //
    // Fetch / upload the player's Minecraft skin and cape metadata from
    // Mojang's session profile API. The Rust core returns a [`crate::auth::model::SkinModel`]
    // JSON (skin_url, cape_url, model type, cache/timestamp) which the Compose
    // UI renders as a 2D preview and caches for offline display.

    /** Fetch skin + cape metadata for [uuid] from Mojang's session profile API.
     *  Returns [SkinModel] JSON (or `{"error":...}`). The skin PNG itself is
     *  downloaded by the UI from `skin_url`/`cape_url`. Call from a background thread.
     *  The account must have a valid Minecraft token first (`authEnsureFresh`). */
    external fun authFetchSkin(uuid: String): String

    /**
     *  Upload a custom skin for [uuid]. `model` is "slim" or "classic";
     *  `skinBase64` is the PNG bytes base64-encoded. Returns `{"ok":true}`
     *  on success or `{"error":...}` on failure. Call from a background thread.
     */
    external fun authUploadSkin(uuid: String, model: String, skinBase64: String): String

    // === Gamepad mapping database + input calibration (task 4) ==========
    //
    // These mirror the Rust core's `gamepad` module; the Compose UI currently
    // uses the pure-Kotlin [com.rc.launcher.ui.model.GamepadDatabase] mirror so it
    // works without loading the native library, but the same data/behaviour is
    // available here for native-accelerated paths.

    /** Built-in controller profile metadata as a JSON array (task 4). */
    external fun getControllerProfiles(): String

    /**
     * Plug-and-play identification by USB vendor/product id. Returns the matched
     * profile metadata as JSON, or the generic fallback.
     */
    external fun identifyController(vendorId: Int, productId: Int): String

    /**
     * Calibrate a single analog axis (dead-zone + sensitivity + invert).
     * Returns the calibrated value in `[-1, 1]`.
     */
    external fun calibrateAxis(
        value: Float,
        deadzone: Float,
        sensitivity: Float,
        invert: Boolean,
    ): Float

    /**
     * Calibrate a 2-D thumbstick (radial dead-zone + sensitivity + per-axis
     * invert). Returns the calibrated `(x, y)` as a JSON pair string.
     */
    external fun calibrateStick(
        x: Float,
        y: Float,
        deadzone: Float,
        sensitivity: Float,
        invertX: Boolean,
        invertY: Boolean,
    ): String

    // === Launch engine (task 7) =============================================
    //
    // JSON-in / JSON-out. The core assembles the whole JVM command line
    // (classpath with LWJGL substitution, java.library.path, caciocavallo AWT
    // bridge, renderer properties, templated game arguments), runs the preflight
    // checks and classifies crashes. Streaming the live game log and the
    // start/stop lifecycle events arrives with the event bus in task 10.

    /**
     * Assemble (and optionally preflight) the JVM command line.
     *
     * `requestJson` = `{"options":<LaunchOptions>,"version":<version.json>,
     * "preflight":Boolean}`. Returns the prepared-launch JSON (never contains the
     * access token) or `{"error":...}`. Use `preflight = true` before actually
     * launching: it verifies the JRE, the `app_runtime/` bundle and every
     * classpath entry, and creates the runtime directories.
     */
    external fun launchPreview(requestJson: String): String

    /**
     * Classify a finished game session.
     *
     * `requestJson` = `{"exit_code":Int?,"signal":Int?,"log":String,
     * "requested_stop":Boolean}`. Returns `{"category","summary","advice",
     * "advice_zh","crashed","evidence","exception","hs_err_files",...}` —
     * everything a crash screen needs, localisable through `category`.
     */
    external fun launchDiagnose(requestJson: String): String

    /** JSON array of selectable renderers (`id`, `gl_libname`, `env`). */
    external fun launchRenderers(): String

    // === Crash report management (task 24) ===================================
    //
    // The crash diagnosis lives in the Rust core (`launch::crash`): `launchDiagnose`
    // classifies a single finished session into a verdict with evidence, actions and
    // mirror/proxy recovery suggestions. These four entry points manage the
    // *persisted* crash-log store that backs the crash-history / crash-detail screen:
    // install the panic hook, list stored reports, fetch the live log tail, and prune
    // old reports.

    /**
     * Install the process-wide panic hook ("the launcher never silently dies").
     * `requestJson` = `{"data_root": String}`. Returns `{"ok":true,"installed":bool,
     * "already_installed":bool}`. Idempotent — safe to call at app startup.
     */
    external fun crashInstallReporter(requestJson: String): String

    /**
     * List every persisted crash log under the given directory.
     * `requestJson` = `{"dir": String}`. Returns `{"ok":true,"count":N,"logs":[CrashLog]},`
     * where each CrashLog carries `id`, `timestamp`, `kind`, `message`, `logs` (the
     * captured log tail) and `context` (the full CrashReport verdict: category, evidence,
     * exception, hs_err_files, device_info, actions, recovery).
     */
    external fun crashListLogs(requestJson: String): String

    /**
     * Most recent `n` (default 200) log lines from the process-wide ring buffer,
     * newest first. `requestJson` = `{"n": Int?}`. Returns `{"ok":true,"n":N,"logs":[`
     * {ts, level, line}]}`. Feeds the diagnostics card and the crash snapshot (tasks
     * 10, 19, 21).
     */
    external fun crashRecentLogs(requestJson: String): String

    /**
     * Delete old crash logs beyond the `keep` count (default 50). `requestJson` =
     * `{"dir": String, "keep": Int?}`. Returns `{"ok":true,"removed":N,"remaining":N}`.
     */
    external fun crashPruneLogs(requestJson: String): String

    // === Screen orientation / adaptive layout (task 9) ======================
    //
    // The core owns the orientation policy and the window size-class table
    // (`display` module); the Compose layer mirrors the table locally in
    // `ui/AdaptiveLayout.kt` because a rotation must not cost a JNI round trip
    // per recomposition. These two entry points are what keeps the mirror
    // honest (the Kotlin parity test compares them) and what the diagnostics
    // screen shows.

    /**
     * JSON array of the supported orientation policies:
     * `[{"id":"system","android_screen_orientation":"user","forced":null}, …]`.
     * The `id`s are exactly the ones [com.rc.launcher.core.RustBridge] accepts in
     * `LaunchOptions.orientation` and the Kotlin `OrientationMode` persists.
     */
    external fun displayOrientations(): String

    /**
     * Resolve the adaptive-layout decisions for one window.
     *
     * `requestJson` = `{"width_dp":Int,"height_dp":Int,"orientation"?:String}`.
     * Returns `{"orientation","width_class","height_class","landscape","short",
     * "navigation_rail","instance_columns","settings_columns",
     * "dashboard_columns","content_padding_dp","max_content_width_dp"}`, plus
     * `policy` + the oriented `window` when an `orientation` policy id is given.
     */
    external fun displayLayout(requestJson: String): String

    // === Discord Rich Presence (task 5) =====================================
    //
    // These mirror the Rust core's `discord` subsystem (a safe, always-available
    // wrapper over the native `libdiscord-rpc.so` FCL bundles in its APK). The
    // settings screen drives the bridge through them: a toggle + a custom
    // application id, plus live presence updates. Every call returns a JSON
    // `DiscordStateInfo` snapshot (`{ "enabled", "connected", "application_id",
    // "status", "detail" }`) so the UI can render the current state directly.

    /**
     * (Re)configure the Discord Rich Presence bridge.
     *
     * `requestJson` = `{"enabled":Boolean,"application_id"?:String,
     * "library_path"?:String}`. Returns the `DiscordStateInfo` JSON snapshot.
     */
    external fun discordConfigure(requestJson: String): String

    /**
     * Update the rich presence. `presenceJson` is a
     * `{ "state"?:String, "details"?:String, "start_timestamp"?:Long,
     *   "large_image_key"?:String, ... }` object; any omitted field is left
     * unset. Returns the `DiscordStateInfo` JSON snapshot.
     */
    external fun discordUpdate(presenceJson: String): String

    /**
     * Clear the "now playing" card without disconnecting. Returns the
     * `DiscordStateInfo` JSON snapshot.
     */
    external fun discordClear(): String

    /**
     * Current bridge state as a `DiscordStateInfo` JSON snapshot (no side
     * effects).
     */
    external fun discordStatus(): String

    /**
     * Fully disconnect from Discord and release the native library. Returns the
     * `DiscordStateInfo` JSON snapshot.
     */
    external fun discordShutdown(): String

    // === Task 16: complete and auto-updating game-version list ==============
    //
    // These three entry points back the version picker. The Rust core owns a
    // process-wide TTL cache (`VersionListCache`); the Compose layer asks for
    // "the current best list" and gets back `{ manifest, info, groups,
    // filtered }`. The native side handles every network concern: DoH, mirror
    // fallback (BMCLAPI/MCBBS/Aliyun/...), parallel speed-test selection,
    // graceful degradation to the last-good / offline built-in manifest.
    //
    // `requestJson` for [gameFetchVersionList] is
    // `{
    //   ttl_secs?: Int,        // override the 6h default TTL
    //   force_refresh?: Bool,  // bypass TTL even when the cache is fresh
    //   query?: String,        // case-insensitive substring filter
    //   group?: String,        // release|snapshot|pre_release|old_alpha|old_beta|special
    //   mirror_mode?: String,  // all|mirrors_only|auto|off
    //   dns_mode?: { mode: "doh"|"system", servers?: [String] }
    // }`. An empty / missing request returns the default list.
    //
    // The reply envelope is
    // `{ manifest: { latest, versions: [...] }, info: { fresh, fetched_at_unix,
    // stale_fallback, offline_only, total, groups: { release: N, ... } },
    // groups: { release: [...], snapshot: [...], pre_release: [...],
    // old_alpha: [...], old_beta: [...], special: [...] }, filtered: [...],
    // query: String, group: String? }`.

    /**
     * Fetch the game-version list. Honours the cache, falls back to the
     * last-good snapshot and then to the offline built-in manifest if every
     * mirror is unreachable, and surfaces `info.fresh` / `info.stale_fallback`
     * / `info.offline_only` so the UI can render an honest status badge.
     */
    external fun gameFetchVersionList(requestJson: String): String

    /**
     * Force a network refresh of the version list (ignores the TTL cache).
     * Same reply envelope as [gameFetchVersionList]. Useful for the
     * "refresh" / "check for updates" buttons in the version picker.
     */
    external fun gameRefreshVersionList(requestJson: String): String

    /**
     * Inspect the version-list cache without performing any IO. Returns the
     * `VersionListInfo` JSON: `{ fresh, fetched_at_unix, stale_fallback,
     * offline_only, total, groups }`.
     */
    external fun gameVersionListCacheInfo(requestJson: String): String

    /**
     * Drop both the fresh and last-good cache slots. The next
     * [gameFetchVersionList] call will re-fetch from the network (or fall
     * back to the offline built-in manifest). Returns `{ "cleared": true }`.
     */
    external fun gameVersionListClearCache(): String

    // === Discord Rich Presence typed wrappers (task 5) ======================
    //
    // Thin, allocation-cheap Kotlin helpers over the raw `external fun`s above so
    // the Compose settings screen can drive the bridge with real types instead of
    // hand-built JSON. Each returns the `DiscordStateInfo` JSON
    // (`{ "enabled", "connected", "application_id", "status", "detail" }`) parsed
    // into a [JSONObject].

    /** Configure the bridge from typed args. `appId`/`libraryPath` fall back to
     *  the Rust defaults when null. */
    fun discordConfigure(
        enabled: Boolean,
        appId: String? = null,
        libraryPath: String? = null,
    ): JSONObject {
        val cfg = JSONObject().apply {
            put("enabled", enabled)
            if (appId != null) put("application_id", appId)
            if (libraryPath != null) put("library_path", libraryPath)
        }
        return JSONObject(discordConfigure(cfg.toString()))
    }

    /** Update the rich presence; any null field is left unset on the Rust side. */
    fun discordUpdate(
        state: String? = null,
        details: String? = null,
        startTimestamp: Long? = null,
        largeImageKey: String? = null,
        largeImageText: String? = null,
    ): JSONObject {
        val p = JSONObject()
        state?.let { p.put("state", it) }
        details?.let { p.put("details", it) }
        startTimestamp?.let { p.put("start_timestamp", it) }
        largeImageKey?.let { p.put("large_image_key", it) }
        largeImageText?.let { p.put("large_image_text", it) }
        return JSONObject(discordUpdate(p.toString()))
    }

    /** Clear the "now playing" card (keeps the connection). */
    fun discordClearStatus(): JSONObject = JSONObject(discordClear())

    /** Current bridge state as a parsed `DiscordStateInfo` object. */
    fun discordState(): JSONObject = JSONObject(discordStatus())

    /** Fully disconnect from Discord and release the native library. */
    fun discordDisconnect(): JSONObject = JSONObject(discordShutdown())

    // === FFI / JNI bridge: event bus + async callbacks (task 10) ============
    //
    // Long-running work (downloads, launches, auth) is started fire-and-forget
    // and reports exclusively through an event bus. Kotlin subscribes once with
    // an [RcEventSink]; the Rust core then invokes `onEvent(json)` on a
    // background thread for every progress / log / lifecycle / error event.
    // This mirrors MCTier's two-layer Rust<->Kotlin bridge
    // (libeasytier_ffi.so -> thin JNI wrapper -> Kotlin).

    /**
     * Subscribe a Kotlin [RcEventSink] as the bus sink. Returns `true` if a
     * previous sink was replaced. The callback is invoked on a background
     * thread for every event -- never on the calling thread.
     */
    external fun eventBusSubscribe(sink: RcEventSink): Boolean

    /** Detach the current Kotlin sink (events become no-ops). */
    external fun eventBusUnsubscribe()

    /** Whether a Kotlin sink is currently attached. */
    external fun eventBusHasSink(): Boolean

    /**
     * Inject a pre-serialised JSON event into the bus (also used to test the
     * round-trip and to replay logs from the Kotlin side). Returns `false` if
     * `json` is not valid event JSON.
     */
    external fun eventBusPublish(json: String): Boolean

    /**
     * Fire-and-forget async job that streams progress / lifecycle / error
     * events to the bus and returns immediately. `specJson` =
     * `{"scope"?:String,"label"?:String,"steps"?:Int,"fail_at"?:Int,
     * "delay_ms"?:Long}`. Returns `{"ok":Boolean,"scope":String}`.
     */
    external fun runAsync(specJson: String): String

    /** Cancel a running async job by scope. Returns `true` if a job was found. */
    external fun cancelAsync(scope: String): Boolean

    /**
     * Fire-and-forget async *download* job (task 2 ⇄ task 10 integration). A batch
     * of download tasks is driven by the resumable download manager on the Rust
     * side; progress / lifecycle / error events are streamed through the event bus
     * ([RcEventBus]) exactly like [runAsync]. `specJson` =
     * `{"scope"?:String,"label"?:String,"concurrency"?:Int,"tasks":[{"url":String,
     * "dest":String,"size"?:Long,"sha1"?:String,"md5"?:String,
     * "mirrors"?:[String]}]}`. Returns `{"ok":Boolean,"scope":String}`.
     */
    external fun downloadAsync(specJson: String): String

    /**
     * Convenience wrapper: start a download job from a pre-built spec JSON and
     * return its [RcJobHandle] (ok flag + scope) so callers can later
     * [cancelAsync] it or correlate the bus events by `scope`.
     */
    fun runDownloadAsync(specJson: String): RcJobHandle {
        val out = JSONObject(downloadAsync(specJson))
        return RcJobHandle(out.optBoolean("ok", false), out.optString("scope", ""))
    }

    // === Modpack import (task 17) ===========================================
    //
    // The pipeline accepts a Modrinth `modrinth.index.json`, a CurseForge
    // `manifest.json`, or a MultiMC `instance.cfg + mmc-pack.json` pair (all
    // expressed as a single normalised `Manifest` JSON in the response).
    // `modpackInspect` does pure parsing (no I/O); `modpackImport` runs the
    // full pipeline asynchronously and reports progress through the event
    // bus exactly like `runDownloadAsync`.

    /**
     * Inspect a modpack manifest text. The input is a JSON envelope:
     * `{"text": "...manifest text...", "origin": "..."}`. Returns the
     * normalised `Manifest` JSON (the same shape `modpackImport` consumes),
     * or `{"error": "..."}` on parse failure.
     */
    external fun modpackInspect(specJson: String): String

    /**
     * Inspect a modpack *archive* (`.zip` / `.mrpack`) passed as a base64
     * payload. `{"bytes_b64": "...", "origin": "..."}` -> same reply shape
     * as [modpackInspect].
     */
    external fun modpackInspectArchive(specJson: String): String

    /**
     * Fire-and-forget modpack import (task 17). Returns
     * `{"ok": bool, "scope": "modpack-..."}`. The full progress / lifecycle /
     * error stream is published on the event bus; subscribe through
     * [RcEventBus] to render the import progress bar.
     *
     * `specJson` =
     * ```
     * {
     *   "config": { "instances_root": "/data/.../instances",
     *               "concurrency": 4, "chunk_size": 4194304, "max_retries": 3 },
     *   "manifest": { ... output of modpackInspect ... },
     *   "instance_id": "all-the-mods-9",
     *   "allow_overwrite": false
     * }
     * ```
     */
    external fun modpackImport(specJson: String): String

    /**
     * Synchronously extract the `overrides/` directory out of an archive
     * already saved on disk. `{"bytes_b64": "...", "instance_root": "..."}`
     * -> `{"ok": true, "written": ["..."]}` on success.
     */
    external fun modpackExtractOverrides(specJson: String): String

    /** Typed Kotlin wrapper for [modpackInspect]. Returns the parsed
     *  `Manifest` envelope or throws with `error` in `message`. */
    fun modpackInspectManifest(text: String, origin: String? = null): JSONObject {
        val req = JSONObject().apply {
            put("text", text)
            if (origin != null) put("origin", origin)
        }
        return JSONObject(modpackInspect(req.toString()))
    }

    /** Convenience: inspect a base64 archive. */
    fun modpackInspectArchive(base64: String, origin: String? = null): JSONObject {
        val req = JSONObject().apply {
            put("bytes_b64", base64)
            if (origin != null) put("origin", origin)
        }
        return JSONObject(modpackInspectArchive(req.toString()))
    }

    /** Start a modpack import from a pre-built manifest JSON. */
    fun runModpackImport(
        instancesRoot: String,
        manifestJson: JSONObject,
        instanceId: String,
        allowOverwrite: Boolean = false,
        concurrency: Int = 4,
        chunkSize: Long = 4L * 1024 * 1024,
        maxRetries: Int = 3,
    ): RcJobHandle {
        val req = JSONObject().apply {
            put("config", JSONObject().apply {
                put("instances_root", instancesRoot)
                put("concurrency", concurrency)
                put("chunk_size", chunkSize)
                put("max_retries", maxRetries)
            })
            put("manifest", manifestJson)
            put("instance_id", instanceId)
            put("allow_overwrite", allowOverwrite)
        }
        val out = try {
            JSONObject(modpackImport(req.toString()))
        } catch (t: Throwable) {
            JSONObject().apply { put("error", t.message ?: "native bridge error") }
        }
        return RcJobHandle(out.optBoolean("ok", false), out.optString("scope", ""))
    }

    // === File manager (task 19) ============================================
    //
    // JSON-in / JSON-out bridge for the in-app small file manager. Every
    // entry point takes a JSON envelope with a list of "allowed roots" and
    // (for destructive operations) a `confirm: true` flag. The Rust core
    // canonicalises every path, asserts it falls under an allowed root, and
    // returns either the typed success payload or `{"error": "..."}` so the
    // Compose layer can surface the failure as a snackbar.
    //
    // Path-traversal is rejected at the core boundary (`fs_ops::resolve_under_
    // roots`): the UI cannot trick the bridge into reading or writing outside
    // the roots it was given. This is the same defence-in-depth pattern FCL
    // uses in its `FileFinder`.

    /**
     * List a directory. `requestJson` =
     * `{"path": String, "roots": [String]}`. Returns the
     * `FsListing` envelope `{path, parent, entries, dir_count, file_count}`,
     * where each `entries[i]` carries `name, kind, size, mtime_ms, path,
     * hidden`.
     */
    external fun fsListDir(requestJson: String): String

    /**
     * Create a directory under an allowed root. `requestJson` =
     * `{"parent": String, "name": String, "roots": [String]}`. Returns the
     * `FsOpResult` JSON (`{op, path, files_touched, bytes_written}`) on
     * success.
     */
    external fun fsMkdir(requestJson: String): String

    /**
     * Copy a file or directory tree. `requestJson` =
     * `{"source": String, "destination": String, "roots": [String]}`. Both
     * paths must resolve under an allowed root; the destination must not
     * already exist. Returns `FsOpResult`.
     */
    external fun fsCopy(requestJson: String): String

    /**
     * Move (rename) a file or directory. `requestJson` =
     * `{"source": String, "destination": String, "roots": [String],
     * "confirm": Bool}`. With `confirm: false` and an existing destination,
     * returns an `FsOpPreview` so the UI can ask for confirmation; with
     * `confirm: true`, performs the move and returns `FsOpResult`.
     */
    external fun fsMove(requestJson: String): String

    /**
     * Rename a single entry inside its parent directory. `requestJson` =
     * `{"path": String, "new_name": String, "roots": [String]}`. The new name
     * is checked for `..` / absolute / drive-prefix components.
     */
    external fun fsRename(requestJson: String): String

    /**
     * Delete one or more paths. `requestJson` =
     * `{"paths": [String], "roots": [String], "confirm": Bool}`. The first
     * call without `confirm: true` returns an `FsOpPreview` so the UI can
     * show a confirmation dialog with the exact list of victims + total
     * size; the call with `confirm: true` actually performs the delete.
     */
    external fun fsDelete(requestJson: String): String

    /**
     * Extract a `.zip` archive into `destination`. `requestJson` =
     * `{"archive": String, "destination": String, "roots": [String]}`. Used
     * by the import buttons on the mod / resource-pack / shader-pack
     * browsers. Returns `FsOpResult`.
     */
    external fun fsExtractZip(requestJson: String): String

    /**
     * Write an opaque byte blob (typically decoded from a `content://` URI
     * by the Android side) into `destination`. `requestJson` =
     * `{"destination": String, "data_b64": String, "roots": [String]}`. The
     * destination must be an *exact* file path (no `..`); the parent
     * directory is created on demand.
     */
    external fun fsImportBytes(requestJson: String): String

    // === File manager typed wrappers (task 19) ==============================
    //
    // Thin, allocation-cheap Kotlin helpers over the raw `external fun`s so
    // the Compose file-manager screen drives the bridge with real types
    // (and never builds the JSON envelopes by hand). The wrappers catch
    // every Throwable (parse / IO / native panic) and return a sealed
    // [FileManagerResult] so the UI can render success / preview / error
    // states with a `when`.

    /**
     * Outcome of one file-manager operation. Sealed so the UI's `when` is
     * exhaustive at compile time.
     */
    sealed class FileManagerResult {
        /** Operation succeeded; payload is the `FsOpResult` JSON. */
        data class Success(val json: JSONObject) : FileManagerResult()

        /** Destructive operation needs user confirmation; payload is the
         *  `FsOpPreview` JSON (`{op, targets, total_bytes, has_directories}`). */
        data class NeedsConfirmation(val json: JSONObject) : FileManagerResult()

        /** Anything else: `{"error": "..."}` or a native panic. */
        data class Error(val message: String) : FileManagerResult()
    }

    /** Parse a raw bridge reply into a typed [FileManagerResult]. */
    private fun parseFsReply(raw: String, confirmExpected: Boolean): FileManagerResult {
        val obj = try {
            JSONObject(raw)
        } catch (t: Throwable) {
            return FileManagerResult.Error(t.message ?: "bad reply")
        }
        if (obj.has("error")) {
            return FileManagerResult.Error(obj.optString("error"))
        }
        if (confirmExpected && obj.has("op") && obj.has("targets")) {
            return FileManagerResult.NeedsConfirmation(obj)
        }
        return FileManagerResult.Success(obj)
    }

    /** List `path` (must be inside one of `roots`). */
    fun fsListDirTyped(path: String, roots: List<String>): FileManagerResult =
        parseFsReply(
            fsListDir(JSONObject().apply {
                put("path", path)
                put("roots", org.json.JSONArray(roots))
            }.toString()),
            confirmExpected = false,
        )

    /** Create `name` inside `parent`. */
    fun fsMkdirTyped(parent: String, name: String, roots: List<String>): FileManagerResult =
        parseFsReply(
            fsMkdir(JSONObject().apply {
                put("parent", parent)
                put("name", name)
                put("roots", org.json.JSONArray(roots))
            }.toString()),
            confirmExpected = false,
        )

    /** Copy `source` to a not-yet-existing `destination`. */
    fun fsCopyTyped(source: String, destination: String, roots: List<String>): FileManagerResult =
        parseFsReply(
            fsCopy(JSONObject().apply {
                put("source", source)
                put("destination", destination)
                put("roots", org.json.JSONArray(roots))
            }.toString()),
            confirmExpected = false,
        )

    /**
     * Move `source` to `destination`. Pass `confirm = false` to get an
     * `FsOpPreview`; pass `confirm = true` to actually apply.
     */
    fun fsMoveTyped(
        source: String,
        destination: String,
        roots: List<String>,
        confirm: Boolean = false,
    ): FileManagerResult =
        parseFsReply(
            fsMove(JSONObject().apply {
                put("source", source)
                put("destination", destination)
                put("roots", org.json.JSONArray(roots))
                put("confirm", confirm)
            }.toString()),
            confirmExpected = true,
        )

    /** Rename `path` to `newName` (a single leaf, no `..` allowed). */
    fun fsRenameTyped(path: String, newName: String, roots: List<String>): FileManagerResult =
        parseFsReply(
            fsRename(JSONObject().apply {
                put("path", path)
                put("new_name", newName)
                put("roots", org.json.JSONArray(roots))
            }.toString()),
            confirmExpected = false,
        )

    /**
     * Delete `paths`. Pass `confirm = false` to get an `FsOpPreview`;
     * pass `confirm = true` to actually delete.
     */
    fun fsDeleteTyped(
        paths: List<String>,
        roots: List<String>,
        confirm: Boolean = false,
    ): FileManagerResult =
        parseFsReply(
            fsDelete(JSONObject().apply {
                put("paths", org.json.JSONArray(paths))
                put("roots", org.json.JSONArray(roots))
                put("confirm", confirm)
            }.toString()),
            confirmExpected = true,
        )

    /** Extract a `.zip` archive into `destination`. */
    fun fsExtractZipTyped(
        archive: String,
        destination: String,
        roots: List<String>,
    ): FileManagerResult =
        parseFsReply(
            fsExtractZip(JSONObject().apply {
                put("archive", archive)
                put("destination", destination)
                put("roots", org.json.JSONArray(roots))
            }.toString()),
            confirmExpected = false,
        )

    /** Write `bytes` to `destination` (used for `content://` imports). */
    fun fsImportBytesTyped(
        destination: String,
        bytes: ByteArray,
        roots: List<String>,
    ): FileManagerResult =
        parseFsReply(
            fsImportBytes(JSONObject().apply {
                put("destination", destination)
                put("data_b64", android.util.Base64.encodeToString(bytes, android.util.Base64.NO_WRAP))
                put("roots", org.json.JSONArray(roots))
            }.toString()),
            confirmExpected = false,
        )

    // === AWT / Swing compatibility layer (fakefx, task 18) ==================
    //
    // Minecraft's embedded AWT/Swing UI (Forge / OptiFine installers, the Mojang
    // splash, `JOptionPane` crash dialogs, font metrics) is rendered by
    // caciocavallo into an off-screen ARGB desktop inside the game JVM. The Rust
    // core hosts that session: it pumps the named-pipe channels, validates every
    // frame, keeps a double-buffered damage-tracking canvas and translates
    // Compose gestures back into `java.awt.event.*` records.
    //
    // The UI therefore only does two things per frame:
    //   1. `awtPollFrame(buffer)` -- refresh the *direct* ByteBuffer backing the
    //      Compose `Bitmap` with the damaged rows only (zero copy).
    //   2. `awtInput(json)`       -- hand over the batched touches / keys.

    /**
     * Open (or replace) the live AWT session. `configJson` =
     * `{"screen":{"width":Int,"height":Int},"surface":{...},
     *   "scale_mode":"fit"|"stretch"|"fill_crop"|"center","click_slop":Int,
     *   "max_pending_events":Int,"java_version":"jre8"|"jre17"|...,
     *   "transport":{"dir":String}|{"frames":String,"events":String}}`.
     * Every field is optional (defaults: a 1280x720 desktop, `fit`). Returns the
     * session snapshot, or `{"error":...}`.
     */
    external fun awtOpen(configJson: String): String

    /** Close the session and stop its pump threads. `{"closed":Boolean,...}`. */
    external fun awtClose(): String

    /** Session + transport snapshot, or `{"open":false}`. */
    external fun awtInfo(): String

    /**
     * Apply surface / desktop geometry, scale mode, focus and repaint requests:
     * `{"surface":{"width":Int,"height":Int},"screen":{...},"scale_mode":String,
     *   "focus":Boolean,"release_all":Boolean,"reset_input":Boolean,
     *   "clear":Boolean,"fill":Long}`. Returns the session snapshot.
     */
    external fun awtConfigure(requestJson: String): String

    /**
     * Create + pump the named-pipe channels of an already open session
     * (`{"dir":String}` or `{"frames":String,"events":String}`). The same paths
     * must be handed to the game JVM through `LaunchOptions.awt_transport_dir`.
     */
    external fun awtAttachTransport(requestJson: String): String

    /**
     * A batch of input events, one call per UI frame:
     * `{"events":[{"type":"pointer","phase":"down"|"move"|"up","x":Float,
     *   "y":Float,"button":"left"|"middle"|"right"},{"type":"scroll","x":Float,
     *   "y":Float,"ticks":Int},{"type":"key_down","code":Int}|{"name":String},
     *   {"type":"key_up",...},{"type":"text","text":String},
     *   {"type":"focus","gained":Boolean},{"type":"release_all"}]}`.
     * Returns `{"queued":Int,"pending":Int,"modifiers":Int,"focused":Boolean,
     * "pointer":{"x":Int,"y":Int},"rejected":[String]}` -- a malformed event is
     * reported without dropping the rest of the batch.
     */
    external fun awtInput(requestJson: String): String

    /**
     * Hand one encoded `RCAF` frame to the session. Only needed when Kotlin owns
     * the transport instead of the Rust pump (or for a self-test).
     */
    external fun awtSubmitFrame(frame: ByteArray): String

    /**
     * The hot path: convert the damaged region straight into [buffer], which
     * must be the **direct** `ByteBuffer` backing the Compose `Bitmap`
     * (`ByteBuffer.allocateDirect(width * height * 4)`, RGBA8888). Returns
     * `{"changed":false}` when nothing changed, so the UI can skip both the
     * upload and the recomposition, or
     * `{"changed":true,"x":Int,"y":Int,"width":Int,"height":Int,"bytes":Int}`.
     */
    external fun awtPollFrame(buffer: java.nio.ByteBuffer): String

    /** Same as [awtPollFrame] for a heap `ByteArray` (one extra copy each way). */
    external fun awtPollFrameArray(buffer: ByteArray): String

    /**
     * The queued AWT records as 32-byte little-endian rows, for a Kotlin-side
     * transport. Empty when no session is open.
     */
    external fun awtDrainEvents(): ByteArray

    /**
     * The **control plane** of the AWT bridge: everything that crosses it but is
     * not a pixel. Returns
     * `{"control":[{"kind":"cursor"|"title"|"clipboard_set"|"clipboard_request"|
     *   "beep"|"screen_size"|"ime_show"|"ime_hide"|"window_opened"|
     *   "window_closed"|"bye","seq":Int,...}],"count":Int,
     *   "state":{"cursor":String,"cursor_awt_type":Int,"title":String?,
     *     "ime":{"x":Int,"y":Int,"line_height":Int}?,"wants_keyboard":Boolean,
     *     "clipboard_out":String?,"clipboard_requests":Int,
     *     "windows":[{"id":Int,"title":String}],"window_count":Int,"beeps":Int,
     *     "bye":String?},"clipboard_requests":Int}`.
     *
     * Draining is destructive: each message's side effect (push this text to the
     * Android clipboard, buzz once, pop the soft keyboard) must fire exactly once.
     */
    external fun awtDrainControl(): String

    /**
     * The launcher's answers to the control plane:
     * `{"clipboard":String?}` (answer every pending `Clipboard.getContents()`),
     * `{"clipboard_empty":true}` (answer "no text" -- still an answer: a Swing
     * thread may be blocked on it), `{"clipboard_seq":Int,...}` (answer one),
     * `{"pong":Int}` (liveness) or `{"reset":true}` (forget the projection).
     * Returns `{"queued":Int,"clipboard_requests":Int,"state":{...}}`.
     */
    external fun awtControl(requestJson: String): String

    /**
     * Hand one encoded `RCAC` control message to the session -- the mirror of
     * [awtSubmitFrame], for a Kotlin-owned transport or a self-test.
     */
    external fun awtSubmitControl(message: ByteArray): String

    // === Internationalisation (task 20) =====================================
    //
    // The Rust core owns the message catalogues (resource files
    // `rust/crates/rc-launcher-core/i18n/<tag>.properties`) so the core and the
    // UI can never disagree about a crash advice or an error message. All calls
    // exchange JSON and are cheap enough for the UI thread — [i18nBundle] is the
    // one to prefer: it hands over the *whole* catalogue in a single crossing.
    //
    // Chinese-first: an unknown / unsupported tag always resolves to `zh-CN`.

    /**
     * The shipped languages:
     * `{"base":"zh-CN","current":"...","languages":[{"tag","native_name",
     * "english_name","android_qualifier","completeness","messages","base","rtl"}]}`.
     */
    external fun i18nLanguages(): String

    /** The current UI language tag (e.g. `zh-CN`). */
    external fun i18nCurrentLanguage(): String

    /**
     * Switch the UI language. `requestJson` = `{"tag":"zh-Hant"}` or
     * `{"preferred":["zh-Hant-TW","en"]}` (an Android `LocaleList`). Returns the
     * language actually applied.
     */
    external fun i18nSetLanguage(requestJson: String): String

    /**
     * Translate one key:
     * `{"key":"error.checksum","language"?:"en","args"?:{"path":"..."},"count"?:3}`
     * -> `{"key","value","language","missing"}`. With `count` the key is treated
     * as a plural base key and `{count}` is supplied automatically.
     */
    external fun i18nTranslate(requestJson: String): String

    /**
     * The whole resolved catalogue: `{"language":"en","messages":{key:value,...}}`.
     * Used to hydrate the Compose string table in one JNI crossing.
     */
    external fun i18nBundle(requestJson: String): String

    /**
     * Locale-aware value formatting — byte sizes, rates, percentages, durations,
     * relative time:
     * `{"kind":"bytes","value":1536}` -> `{"kind","text":"1.5 KB","language","supported"}`.
     *
     * `kind` is one of `bytes`, `rate`, `byte_progress`, `int`, `decimal`,
     * `percent`, `ratio`, `duration`, `eta`, `relative`, `fps`; `total` pairs with
     * `ratio`/`byte_progress`, `digits` sets the precision and `parts` caps how
     * many duration units are shown. An unknown `kind` still returns a number
     * with `"supported": false`.
     *
     * Compose does **not** need this per label — [i18nBundle] already ships the
     * format skeletons (`format.size`, `unit.mib`, `duration.minute.other`, ...)
     * and `RcValueFormat` assembles them locally. This entry point exists for
     * non-Compose consumers and as the oracle the parity tests check against.
     */
    external fun i18nFormat(requestJson: String): String

    /**
     * Catalogue health: missing keys, orphan keys, placeholder drift, parse
     * problems and the keys that failed to resolve at runtime.
     */
    external fun i18nDiagnostics(): String

    /**
     * **Dynamic language loading** — register whole new languages at runtime from
     * `.properties` packs, without a new APK.
     *
     * `{"action":"load","path":"/data/.../files/i18n"}` scans a directory,
     * `{"action":"install","text":"_meta.tag = ja\n…"}` registers one document,
     * `{"action":"remove","tag":"ja"}` / `{"action":"clear"}` unregister, and
     * `{"action":"list"}` (the default) just reports state.
     *
     * Returns `{"ok","loaded":[tag…],"skipped":["file: reason"…],"packs":[…],
     * "count","active","current","limits"}`. A loaded pack is a **first-class
     * language**: it shows up in [i18nLanguages], [i18nSetLanguage] can select it
     * and [i18nBundle] hydrates the UI from it.
     *
     * `skipped` carries a human-readable reason per rejected file (too large, tag
     * collides with a built-in language, no messages, bad encoding, …) so the
     * settings screen can tell the user why their pack did not appear.
     *
     * Distinct from [i18nOverlay]: an overlay *re-words* a language we ship, a
     * pack *adds* one we do not.
     */
    external fun i18nLanguagePacks(requestJson: String): String

    /**
     * Install / clear a runtime translation overlay (community translations and
     * wording hot-fixes without a new APK):
     * `{"action":"install","language":"en","text":"key = value\n"}`,
     * `{"action":"dir","path":"/data/.../i18n"}` or `{"action":"clear"}`.
     */
    external fun i18nOverlay(requestJson: String): String

    // === Inline auto-translation for the mod browser (task 13) =======
    //
    // The Rust core owns the translation pipeline: a configurable
    // gateway (default = the kilo.ai chat-completions URL requested
    // in the task brief), an on-disk cache so the player only pays
    // once per sentence, a built-in dictionary for offline / fallback,
    // and a "follow system" or "force offline" toggle.
    //
    // All calls exchange JSON; see the Rust `crate::ffi` module for the
    // exact shape. The Compose layer never holds any native state
    // beyond a Kotlin viewmodel cache.

    /**
     * (Re)configure the process-wide translation service.
     *
     * `requestJson` =
     * `{ "cache_root"?: String, "gateway"?: TranslationGateway,
     *    "force_offline"?: Boolean, "default_mode"?: "online"|"offline"|"hybrid" }`.
     *
     * Returns `{"ok": true}`.
     */
    external fun translateInit(requestJson: String): String

    /**
     * Translate one piece of text. `requestJson` =
     * `{ "text": String, "source"?: String, "target": "zh-CN"|"zh-Hant"|"en"|"auto",
     *    "mode"?: "online"|"offline"|"hybrid", "cache_ttl_secs"?: Long,
     *    "extra_headers"?: { k: v } }`.
     *
     * Returns a JSON object: `{ original, translated, target, detected_source,
     *   source, offline }`. `source` is one of
     *   `passthrough|dictionary|cache|gateway|unavailable` so the UI can
     *   show a badge next to each row.
     */
    external fun translate(requestJson: String): String

    /**
     * Translate a batch of requests in one JNI crossing — preserves input
     * order. Accepts either a bare JSON array or `{ "requests": [...] }`.
     */
    external fun translateBatch(requestJson: String): String

    /**
     * The translation catalogue: `{ languages: [...], modes: [...],
     *   sources: [...] }`. The UI uses it to populate the language picker.
     */
    external fun translateLanguages(): String

    /**
     * Cache stats: `{ root, entry_count, total_bytes, max_entries, max_bytes }`.
     * Surfaced in the settings UI so the player can see how much room the
     * translation cache is taking.
     */
    external fun translateCacheStats(): String

    /**
     * Drop every cached translation. The next call to [translate] will
     * re-hit the gateway / dictionary. Returns `{ "removed": Int }`.
     */
    external fun translateClearCache(): String

    /**
     * The currently configured gateway (URL, model, auth). Surfaced in
     * the settings UI to let the player pick a different translation
     * endpoint or rotate their API key.
     */
    external fun translateGateway(): String

    /**
     * Typed wrapper around [translate] — accepts a Kotlin-friendly
     * argument list and returns a parsed JSONObject (the raw JSON
     * response exactly as the Rust side produced it). Returns
     * `{"error": ...}` on any failure so the UI can degrade gracefully.
     */
    fun translateText(
        text: String,
        target: String,
        source: String? = null,
        mode: String? = null,
        cacheTtlSecs: Long? = null,
    ): JSONObject {
        val req = JSONObject().apply {
            put("text", text)
            put("target", target)
            source?.let { put("source", it) }
            mode?.let { put("mode", it) }
            cacheTtlSecs?.let { put("cache_ttl_secs", it) }
        }
        return try {
            JSONObject(translate(req.toString()))
        } catch (t: Throwable) {
            JSONObject().apply { put("error", t.message ?: "unknown") }
        }
    }

    /**
     * Typed wrapper around [translateBatch] — accepts a list of
     * `(text, target)` pairs and returns a JSON array of results
     * (one per input, in input order). The wrapper tolerates both
     * bare-array and `{ "requests": [...] }` request shapes, and either
     * bare-array or `{ "results": [...] }` response shapes.
     */
    fun translateBatchText(
        requests: List<Pair<String, String>>,
        mode: String? = null,
    ): org.json.JSONArray {
        val arr = org.json.JSONArray()
        for ((text, target) in requests) {
            val req = JSONObject().apply {
                put("text", text)
                put("target", target)
            }
            arr.put(req)
        }
        val payload = if (mode != null) {
            JSONObject().apply {
                put("requests", arr)
                put("mode", mode)
            }.toString()
        } else {
            arr.toString()
        }
        val out = JSONObject(translateBatch(payload))
        val results = out.optJSONArray("results")
        if (results != null) return results
        // translateBatch returns a bare array; tolerate that shape.
        val fallback = org.json.JSONArray()
        for (i in 0 until out.length()) {
            fallback.put(out.get(i))
        }
        return fallback
    }
    // === Task 16 typed wrappers (version list) =============================
    //
    // Thin Kotlin-side helpers over the raw `external fun`s so Compose call
    // sites read naturally and never build raw JSON by hand. The wrappers
    // catch every Throwable (parse / IO / native panic) and degrade to an
    // "offline only" reply so the UI never blocks on a failed native call.

    /**
     * Kotlin envelope for the version-list reply. `manifest` carries
     * `latest.release/snapshot` and the flat `versions` array; `info`
     * carries the cache state for the badge; `groups` is the per-bucket
     * split (`release / snapshot / pre_release / old_alpha / old_beta /
     * special`) the picker renders as tabs; `filtered` is the (optionally
     * queried) subset the UI is currently showing.
     */
    data class VersionListReply(
        val manifest: JSONObject,
        val info: JSONObject,
        val groups: JSONObject,
        val filtered: org.json.JSONArray,
        val query: String,
        val group: String?,
    )

    /**
     * Fetch the version list. Empty / default arguments produce the standard
     * "give me everything you have, the freshest you can find" reply; combine
     * `query` + `group` to narrow the picker. The wrapper is total: any
     * failure inside the bridge is caught, logged into `info`, and returned
     * as an empty list so the UI keeps rendering.
     */
    fun fetchVersionList(
        query: String = "",
        group: String? = null,
        ttlSecs: Long? = null,
        forceRefresh: Boolean = false,
        mirrorMode: String? = null,
        dnsMode: String? = null,
        dnsServers: List<String> = emptyList(),
    ): VersionListReply {
        val req = JSONObject().apply {
            if (query.isNotEmpty()) put("query", query)
            if (group != null) put("group", group)
            ttlSecs?.let { put("ttl_secs", it) }
            if (forceRefresh) put("force_refresh", true)
            if (mirrorMode != null) put("mirror_mode", mirrorMode)
            if (dnsMode != null) {
                val dns = JSONObject().apply { put("mode", dnsMode) }
                if (dnsServers.isNotEmpty()) {
                    val arr = org.json.JSONArray()
                    for (s in dnsServers) arr.put(s)
                    dns.put("servers", arr)
                }
                put("dns_mode", dns)
            }
        }
        val raw = try {
            gameFetchVersionList(req.toString())
        } catch (t: Throwable) {
            return offlineReply(errorMessage = t.message ?: "native bridge error")
        }
        val obj = try {
            JSONObject(raw)
        } catch (t: Throwable) {
            return offlineReply(errorMessage = t.message ?: "bad JSON from native")
        }
        return VersionListReply(
            manifest = obj.optJSONObject("manifest") ?: JSONObject(),
            info = obj.optJSONObject("info") ?: JSONObject().apply {
                put("offline_only", true)
                put("error", obj.optString("error", "no manifest"))
            },
            groups = obj.optJSONObject("groups") ?: JSONObject(),
            filtered = obj.optJSONArray("filtered") ?: org.json.JSONArray(),
            query = obj.optString("query", query),
            group = obj.optString("group", group).takeIf { obj.has("group") && !obj.isNull("group") },
        )
    }

    /** Force-refresh the version list (ignores TTL). Same reply shape as
     *  [fetchVersionList] but always hits the network first. */
    fun refreshVersionList(): VersionListReply {
        val raw = try {
            gameRefreshVersionList("{}")
        } catch (t: Throwable) {
            return offlineReply(errorMessage = t.message ?: "native bridge error")
        }
        val obj = try {
            JSONObject(raw)
        } catch (t: Throwable) {
            return offlineReply(errorMessage = t.message ?: "bad JSON from native")
        }
        return VersionListReply(
            manifest = obj.optJSONObject("manifest") ?: JSONObject(),
            info = obj.optJSONObject("info") ?: JSONObject().apply {
                put("offline_only", true)
                put("error", obj.optString("error", "no manifest"))
            },
            groups = obj.optJSONObject("groups") ?: JSONObject(),
            filtered = obj.optJSONArray("filtered") ?: org.json.JSONArray(),
            query = "",
            group = null,
        )
    }

    /** Inspect the cache without IO. Returns the `info` field as-is. */
    fun versionListCacheInfo(ttlSecs: Long? = null): JSONObject {
        val req = JSONObject().apply { ttlSecs?.let { put("ttl_secs", it) } }
        val raw = try {
            gameVersionListCacheInfo(req.toString())
        } catch (t: Throwable) {
            return JSONObject().apply {
                put("offline_only", true)
                put("error", t.message ?: "native bridge error")
            }
        }
        return try {
            JSONObject(raw)
        } catch (t: Throwable) {
            JSONObject().apply {
                put("offline_only", true)
                put("error", t.message ?: "bad JSON from native")
            }
        }
    }

    /** Drop the cache. The next [fetchVersionList] re-fetches or degrades
     *  to the offline built-in manifest. */
    fun clearVersionListCache(): Boolean {
        val raw = try {
            gameVersionListClearCache()
        } catch (t: Throwable) {
            return false
        }
        return try {
            JSONObject(raw).optBoolean("cleared", false)
        } catch (_: Throwable) {
            false
        }
    }

    private fun offlineReply(errorMessage: String): VersionListReply {
        return VersionListReply(
            manifest = JSONObject(),
            info = JSONObject().apply {
                put("offline_only", true)
                put("fresh", false)
                put("total", 0)
                put("error", errorMessage)
            },
            groups = JSONObject(),
            filtered = org.json.JSONArray(),
            query = "",
            group = null,
        )
    }


    // === Crash report typed wrappers (task 24) =================================
    //
    // Thin Kotlin helpers over `crashListLogs` / `crashRecentLogs` /
    // `crashInstallReporter` / `crashPruneLogs` so the Compose crash screen
    // works with real types and never builds JSON by hand. Each wrapper
    // catches Throwable (parse / IO / native panic) and returns a safe
    // default so the UI never blocks on a failed native call.

    /**
     * Install the crash reporter (panic hook). Returns true if the hook was
     * newly installed, false if it was already active. Call this once at
     * app startup with the launcher data-root directory.
     */
    fun installCrashReporter(dataRoot: String): Boolean {
        val req = JSONObject().put("data_root", dataRoot).toString()
        val raw = try {
            crashInstallReporter(req)
        } catch (t: Throwable) {
            return false
        }
        return try {
            val obj = JSONObject(raw)
            obj.optBoolean("ok", false) && obj.optBoolean("installed", false)
        } catch (_: Throwable) {
            false
        }
    }

    /**
     * List persisted crash logs. Returns a JSON array of CrashLog objects
     * (each with id, timestamp, kind, message, logs, context). Returns an
     * empty array on any error.
     */
    fun listCrashLogs(crashDir: String): org.json.JSONArray {
        val req = JSONObject().put("dir", crashDir).toString()
        val raw = try {
            crashListLogs(req)
        } catch (t: Throwable) {
            return org.json.JSONArray()
        }
        return try {
            val obj = JSONObject(raw)
            obj.optJSONArray("logs") ?: org.json.JSONArray()
        } catch (_: Throwable) {
            org.json.JSONArray()
        }
    }

    /**
     * Most recent [n] log lines from the process-wide ring buffer, newest
     * first. Returns a JSON array of `{ts, level, line}` objects.
     */
    fun recentLogs(n: Int = 200): org.json.JSONArray {
        val req = JSONObject().put("n", n).toString()
        val raw = try {
            crashRecentLogs(req)
        } catch (t: Throwable) {
            return org.json.JSONArray()
        }
        return try {
            val obj = JSONObject(raw)
            obj.optJSONArray("logs") ?: org.json.JSONArray()
        } catch (_: Throwable) {
            org.json.JSONArray()
        }
    }

    /**
     * Delete old crash logs beyond [keep] (default 50). Returns the number of
     * reports removed (0 on error).
     */
    fun pruneCrashLogs(crashDir: String, keep: Int = 50): Int {
        val req = JSONObject().put("dir", crashDir).put("keep", keep).toString()
        val raw = try {
            crashPruneLogs(req)
        } catch (t: Throwable) {
            return 0
        }
        return try {
            val obj = JSONObject(raw)
            obj.optInt("removed", 0)
        } catch (_: Throwable) {
            0
        }
    }
}
