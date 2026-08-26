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

    /** (Re)configure the global account store. `configJson` =
     *  `{"path"?:string,"key_hex"?:string,"client_id"?:string}`. */
    external fun authInit(configJson: String): String

    /** JSON array of redacted accounts (no secrets). */
    external fun authListAccounts(): String

    /** Add an offline account. Returns the account JSON (or `{"error":...}`). */
    external fun authAddOfflineAccount(name: String): String

    /** Begin Microsoft device-code login. Returns the challenge JSON. */
    external fun authBeginMicrosoft(): String

    /** Complete Microsoft login (blocks; background thread). Returns account JSON. */
    external fun authCompleteMicrosoft(challengeJson: String): String

    /** Remove an account by uuid. Returns `{"removed":bool}`. */
    external fun authRemoveAccount(uuid: String): String

    /** Force-refresh a Microsoft account's token. Returns account JSON. */
    external fun authRefreshAccount(uuid: String): String

    /** Return a fresh account, transparently refreshing if the token is expiring. */
    external fun authEnsureFresh(uuid: String): String

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
}
