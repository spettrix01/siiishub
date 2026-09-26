package dev.siiis.siiishub.player

import android.app.Activity
import android.app.ActivityManager
import android.app.DownloadManager
import android.app.UiModeManager
import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract
import android.content.Context
import android.content.pm.ActivityInfo
import android.content.pm.PackageManager
import android.content.res.Configuration
import android.media.AudioManager
import android.media.MediaCodecList
import android.os.Looper
import android.util.Base64
import android.util.Log
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.webkit.WebView
import android.widget.FrameLayout
import androidx.core.graphics.Insets
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import dev.jdtech.mpv.MPVLib
import java.io.File
import java.security.KeyStore
import java.security.cert.X509Certificate
import java.util.concurrent.CountDownLatch

@InvokeArg
class StartArgs {
    lateinit var channel: Channel
}

@InvokeArg
class CommandArgs {
    lateinit var args: Array<String>
}

@InvokeArg
class SetPropertyArgs {
    lateinit var name: String
    var value: Any? = null
}

@InvokeArg
class NameArgs {
    lateinit var name: String
}

@InvokeArg
class GeometryArgs {
    var x: Int = 0
    var y: Int = 0
    var w: Int = 0
    var h: Int = 0
}

@InvokeArg
class VisibleArgs {
    var visible: Boolean = true
}

@InvokeArg
class OpenFolderArgs {
    lateinit var path: String
}

@InvokeArg
class OpenUrlArgs {
    lateinit var url: String
}

/**
 * libmpv embedded under the webview.
 *
 * A SurfaceView sits behind the (transparent) WebView inside the activity
 * content frame; mpv draws into its Surface with vo=gpu/gpu-context=android.
 * The web player UI drives mpv exactly as on desktop: commands, properties
 * and observers come in through the plugin commands below, mpv events go
 * back to Rust through the channel handed over by `start`.
 *
 * `dev.jdtech.mpv.MPVLib` 0.4.x is a static singleton (one mpv per process).
 */
@TauriPlugin
class MpvPlugin(private val activity: Activity) : Plugin(activity), MPVLib.EventObserver, SurfaceHolder.Callback {
    private var webView: WebView? = null
    @Volatile private var created = false
    private var surfaceView: SurfaceView? = null
    private var channel: Channel? = null
    private val observed = HashSet<String>()

    // Edge-to-edge (Android 15+): the web UI is padded away from the system
    // bars, except while the player is open, which goes immersive landscape.
    private var playerVisible = false
    private var barInsets: Insets = Insets.NONE
    private var imeInsets: Insets = Insets.NONE

    // Playback bookkeeping for end-file reasons and screen/audio handling.
    @Volatile private var hasFile = false
    @Volatile private var paused = true
    @Volatile private var eofReached = false
    @Volatile private var expectStop = false
    private var keepAwake = false
    private var audioFocus = false

    override fun load(webView: WebView) {
        this.webView = webView
        // The page is laid out for the display: no pinch or double-tap zoom.
        webView.settings.apply {
            setSupportZoom(false)
            builtInZoomControls = false
            displayZoomControls = false
        }
        // A full-screen scroller (the details card, the home grid) becomes the
        // root scroller and its scrollbar is drawn by this View, where CSS
        // cannot hide it: touch scrolling needs no track.
        webView.isVerticalScrollBarEnabled = false
        webView.isHorizontalScrollBarEnabled = false
        // The page takes the keys from the start: with nothing focused in the
        // window, the first press of a TV remote's D-pad would only move
        // Android's focus onto the WebView.
        webView.isFocusable = true
        webView.isFocusableInTouchMode = true
        webView.post { webView.requestFocus() }
        activity.requestedOrientation = defaultOrientation()
        val root = contentRoot() ?: return
        ViewCompat.setOnApplyWindowInsetsListener(root) { _, insets ->
            barInsets = insets.getInsets(
                WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout()
            )
            // The keyboard shrinks the page too, so a focused field (and the
            // popup editing it) stays above it.
            imeInsets = insets.getInsets(WindowInsetsCompat.Type.ime())
            applyInsets()
            WindowInsetsCompat.CONSUMED
        }
        ViewCompat.requestApplyInsets(root)
    }

    private fun contentRoot(): ViewGroup? =
        (webView?.parent as? ViewGroup) ?: activity.findViewById(android.R.id.content)

    /** Runs on the UI thread. */
    private fun applyInsets() {
        val root = contentRoot() ?: return
        if (playerVisible) {
            // Immersive, no bars; a keyboard (search fields in the player
            // settings) still pushes the page up.
            root.setPadding(0, 0, 0, imeInsets.bottom)
        } else {
            root.setPadding(
                barInsets.left, barInsets.top, barInsets.right,
                maxOf(barInsets.bottom, imeInsets.bottom)
            )
        }
    }

    /** Runs on the UI thread: immersive landscape while the player is open. */
    private fun setPlayerMode(on: Boolean) {
        playerVisible = on
        val window = activity.window
        val controller = WindowCompat.getInsetsController(window, window.decorView)
        if (on) {
            controller.systemBarsBehavior =
                WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            controller.hide(WindowInsetsCompat.Type.systemBars())
            activity.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
        } else {
            controller.show(WindowInsetsCompat.Type.systemBars())
            activity.requestedOrientation = defaultOrientation()
        }
        applyInsets()
    }

    /** Phones keep the browsing UI portrait (like Stremio), tablets rotate
     *  freely and TVs keep their screen as it is: a 1080p TV is only 540 dp
     *  high, so the screen size alone would make it a phone. */
    private fun defaultOrientation(): Int {
        if (isTv()) return ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED
        val swDp = activity.resources.configuration.smallestScreenWidthDp
        return if (swDp < 600) ActivityInfo.SCREEN_ORIENTATION_USER_PORTRAIT
        else ActivityInfo.SCREEN_ORIENTATION_USER
    }

    private fun isTv(): Boolean {
        val uiMode = activity.resources.configuration.uiMode and Configuration.UI_MODE_TYPE_MASK
        return uiMode == Configuration.UI_MODE_TYPE_TELEVISION ||
            activity.packageManager.hasSystemFeature(PackageManager.FEATURE_LEANBACK)
    }

    // ---------------------------------------------------------------- commands

    @Command
    fun start(invoke: Invoke) {
        val args = invoke.parseArgs(StartArgs::class.java)
        channel = args.channel
        invoke.resolve()
    }

    @Command
    fun command(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(CommandArgs::class.java)
            ensureCreated()
            val head = args.args.firstOrNull() ?: throw IllegalArgumentException("empty mpv command")
            if (head == "loadfile" || head == "stop") {
                // The next end-file belongs to the file being replaced/stopped.
                expectStop = true
                eofReached = false
            }
            MPVLib.command(args.args)
            invoke.resolve()
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: ex.toString())
        }
    }

    @Command
    fun setProperty(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(SetPropertyArgs::class.java)
            ensureCreated()
            when (val v = args.value) {
                null -> {}
                is Boolean -> MPVLib.setPropertyBoolean(args.name, v)
                is Int -> MPVLib.setPropertyInt(args.name, v)
                is Long -> MPVLib.setPropertyInt(args.name, v.toInt())
                is Float -> MPVLib.setPropertyDouble(args.name, v.toDouble())
                is Double -> MPVLib.setPropertyDouble(args.name, v)
                is String -> MPVLib.setPropertyString(args.name, v)
                else -> MPVLib.setPropertyString(args.name, v.toString())
            }
            invoke.resolve()
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: ex.toString())
        }
    }

    @Command
    fun getProperty(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(NameArgs::class.java)
            ensureCreated()
            val out = JSObject()
            out.put("value", MPVLib.getPropertyString(args.name))
            invoke.resolve(out)
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: ex.toString())
        }
    }

    @Command
    fun observe(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(NameArgs::class.java)
            ensureCreated()
            observeOnce(args.name)
            invoke.resolve()
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: ex.toString())
        }
    }

    @Command
    fun setGeometry(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(GeometryArgs::class.java)
            ensureCreated()
            onUi {
                val sv = surfaceView ?: return@onUi
                val lp = (sv.layoutParams as? FrameLayout.LayoutParams)
                    ?: FrameLayout.LayoutParams(0, 0)
                lp.width = maxOf(1, args.w)
                lp.height = maxOf(1, args.h)
                lp.leftMargin = args.x
                lp.topMargin = args.y
                sv.layoutParams = lp
            }
            invoke.resolve()
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: ex.toString())
        }
    }

    /**
     * Whether the hardware video decoders take 4K (3840x2160), for HEVC and
     * H.264. A 1080p TV stick (Fire TV Stick 3rd generation: 1920x1088 at
     * most) leaves 4K to mpv's software decoder, which it cannot keep up with
     * and which runs it out of memory: the interface steers away from it.
     */
    @Command
    fun decoderCaps(invoke: Invoke) {
        val out = JSObject()
        out.put("hevc4k", hardwareDecodes("video/hevc", 3840, 2160))
        out.put("avc4k", hardwareDecodes("video/avc", 3840, 2160))
        invoke.resolve(out)
    }

    private fun hardwareDecodes(mime: String, width: Int, height: Int): Boolean =
        MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.any { info ->
            val name = info.name.lowercase()
            !info.isEncoder &&
                !name.startsWith("omx.google.") && !name.startsWith("c2.android.") &&
                info.supportedTypes.any { it.equals(mime, ignoreCase = true) } &&
                runCatching {
                    info.getCapabilitiesForType(mime).videoCapabilities.isSizeSupported(width, height)
                }.getOrDefault(false)
        }

    /** Opens a web page (http or https) with the app the system picks: the browser. */
    @Command
    fun openUrl(invoke: Invoke) {
        val uri = Uri.parse(invoke.parseArgs(OpenUrlArgs::class.java).url)
        if (uri.scheme != "http" && uri.scheme != "https") {
            invoke.reject("indirizzo non supportato")
            return
        }
        try {
            activity.startActivity(Intent(Intent.ACTION_VIEW, uri).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            invoke.resolve()
        } catch (e: ActivityNotFoundException) {
            invoke.reject("nessuna app per aprire il link")
        }
    }

    /**
     * Shows a folder of the shared storage in a file manager: Samsung's My
     * Files at the exact path, else the system Files app (DocumentsUI) on the
     * folder, else the generic downloads screen.
     */
    @Command
    fun openFolder(invoke: Invoke) {
        val path = invoke.parseArgs(OpenFolderArgs::class.java).path
        val intents = ArrayList<Intent>()
        intents.add(Intent("samsung.myfiles.intent.action.LAUNCH_MY_FILES").apply {
            setPackage("com.sec.android.app.myfiles")
            putExtra("samsung.myfiles.intent.extra.START_PATH", path)
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        })
        documentIdFor(path)?.let { docId ->
            intents.add(Intent(Intent.ACTION_VIEW).apply {
                setDataAndType(
                    DocumentsContract.buildDocumentUri("com.android.externalstorage.documents", docId),
                    DocumentsContract.Document.MIME_TYPE_DIR
                )
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
            })
        }
        intents.add(Intent(DownloadManager.ACTION_VIEW_DOWNLOADS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        for (intent in intents) {
            try {
                activity.startActivity(intent)
                invoke.resolve()
                return
            } catch (e: ActivityNotFoundException) {
                Log.i("siiishub", "openFolder: nessuna app per " + intent.action)
            } catch (e: SecurityException) {
                Log.w("siiishub", "openFolder: " + intent.action + " rifiutato: " + e.message)
            }
        }
        invoke.reject("nessuna app per aprire la cartella")
    }

    /** `/storage/emulated/<n>/Download/X` -> `primary:Download/X`; null outside the shared storage. */
    private fun documentIdFor(path: String): String? {
        val m = Regex("^(?:/storage/emulated/[0-9]+|/sdcard)(?:/(.*))?$").find(path) ?: return null
        val rel = m.groupValues[1].trimEnd('/')
        return if (rel.isEmpty()) "primary:" else "primary:" + rel
    }

    @Command
    fun setVisible(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(VisibleArgs::class.java)
            ensureCreated()
            onUi {
                setPlayerMode(args.visible)
                surfaceView?.visibility = if (args.visible) View.VISIBLE else View.GONE
            }
            invoke.resolve()
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: ex.toString())
        }
    }

    /** Landscape immersive player mode, from the moment the player overlay opens. */
    @Command
    fun playerMode(invoke: Invoke) {
        try {
            val args = invoke.parseArgs(VisibleArgs::class.java)
            onUi { setPlayerMode(args.visible) }
            invoke.resolve()
        } catch (ex: Exception) {
            invoke.reject(ex.message ?: ex.toString())
        }
    }

    // ------------------------------------------------------------- lifecycle

    /** Plugin commands run on a Rust thread; views must be touched on the UI thread. */
    private fun onUi(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) block() else activity.runOnUiThread(block)
    }

    private fun onUiSync(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            block()
            return
        }
        val latch = CountDownLatch(1)
        var error: Throwable? = null
        activity.runOnUiThread {
            try { block() } catch (t: Throwable) { error = t } finally { latch.countDown() }
        }
        latch.await()
        error?.let { throw it }
    }

    /**
     * Exports the system CA certificates to a PEM bundle in the app's private
     * files: the `system:` entries of the AndroidCAStore, so user-installed
     * certificates are left out, as with the default network security config.
     * Rewritten at every start, so root updates shipped with the system are
     * picked up. Returns null when no bundle could be written.
     */
    private fun writeSystemCaBundle(): File? = try {
        val store = KeyStore.getInstance("AndroidCAStore").apply { load(null, null) }
        val pem = StringBuilder()
        var count = 0
        for (alias in store.aliases()) {
            if (!alias.startsWith("system:")) continue
            val cert = store.getCertificate(alias) as? X509Certificate ?: continue
            pem.append("-----BEGIN CERTIFICATE-----\n")
            Base64.encodeToString(cert.encoded, Base64.NO_WRAP)
                .chunked(64)
                .forEach { pem.append(it).append('\n') }
            pem.append("-----END CERTIFICATE-----\n")
            count++
        }
        if (count == 0) {
            Log.w("mpv", "CA bundle: no system certificates found")
            null
        } else {
            val file = File(activity.filesDir, "cacert.pem")
            val tmp = File(activity.filesDir, "cacert.pem.tmp")
            tmp.writeText(pem.toString())
            if (!tmp.renameTo(file)) {
                file.delete()
                check(tmp.renameTo(file)) { "rename failed" }
            }
            Log.i("mpv", "CA bundle: $count system certificates")
            file
        }
    } catch (t: Throwable) {
        Log.w("mpv", "CA bundle not written: ${t.message}")
        null
    }

    /** Creation always happens on the UI thread, which also serialises it. */
    private fun ensureCreated() {
        if (created) return
        onUiSync { createOnUi() }
        if (!created) throw IllegalStateException("libmpv non inizializzato")
    }

    private fun createOnUi() {
        if (created) return
        MPVLib.create(activity)

        // Same profile as the desktop builds, plus the Android video/audio outputs.
        MPVLib.setOptionString("config", "no")
        MPVLib.setOptionString("vo", "gpu")
        MPVLib.setOptionString("gpu-context", "android")
        MPVLib.setOptionString("opengl-es", "yes")
        MPVLib.setOptionString("hwdec", "mediacodec,mediacodec-copy")
        MPVLib.setOptionString("hwdec-codecs", "h264,hevc,mpeg4,mpeg2video,vp8,vp9,av1")
        MPVLib.setOptionString("ao", "audiotrack,opensles")
        MPVLib.setOptionString("input-default-bindings", "no")
        MPVLib.setOptionString("input-vo-keyboard", "no")
        MPVLib.setOptionString("osc", "no")
        MPVLib.setOptionString("osd-level", "0")
        MPVLib.setOptionString("idle", "yes")
        MPVLib.setOptionString("force-window", "no")
        MPVLib.setOptionString("keep-open", "yes")
        // A TV with little memory (a 1 GB Fire TV Stick) gets a smaller cache
        // and fewer software decoding threads: Android killed the app while a
        // stream loaded. Phones keep the full cache.
        val mem = ActivityManager.MemoryInfo()
        (activity.getSystemService(Context.ACTIVITY_SERVICE) as ActivityManager).getMemoryInfo(mem)
        val isTv = (activity.getSystemService(Context.UI_MODE_SERVICE) as UiModeManager).currentModeType ==
            Configuration.UI_MODE_TYPE_TELEVISION
        val lowRam = isTv && mem.totalMem < 1536L * 1024 * 1024
        MPVLib.setOptionString("demuxer-max-bytes", ((if (lowRam) 24L else 64L) * 1024 * 1024).toString())
        MPVLib.setOptionString("demuxer-max-back-bytes", ((if (lowRam) 8L else 32L) * 1024 * 1024).toString())
        if (lowRam) MPVLib.setOptionString("vd-lavc-threads", "2")
        // HTTPS streams are verified against the system trust store. The
        // bundled FFmpeg (Mbed TLS) has no CA store of its own, so the system
        // roots are handed to it as a PEM bundle. If the bundle cannot be
        // written, playback still works, without verification.
        val caBundle = writeSystemCaBundle()
        if (caBundle != null) {
            MPVLib.setOptionString("tls-verify", "yes")
            MPVLib.setOptionString("tls-ca-file", caBundle.absolutePath)
        } else {
            MPVLib.setOptionString("tls-verify", "no")
        }
        // Subtitles: libass reads the system fonts directly (no fontconfig).
        MPVLib.setOptionString("sub-font-provider", "none")
        MPVLib.setOptionString("sub-fonts-dir", "/system/fonts")
        MPVLib.setOptionString("sub-font", "Roboto")
        MPVLib.setOptionString("msg-level", "all=warn,cplayer=info")
        MPVLib.init()
        MPVLib.addLogObserver(object : MPVLib.LogObserver {
            override fun logMessage(prefix: String, level: Int, text: String) {
                val prio = when {
                    level <= MPVLib.MPV_LOG_LEVEL_ERROR -> Log.ERROR
                    level <= MPVLib.MPV_LOG_LEVEL_WARN -> Log.WARN
                    else -> Log.INFO
                }
                Log.println(prio, "mpv", "[$prefix] ${text.trimEnd()}")
            }
        })
        MPVLib.addObserver(this)
        created = true
        // Internal bookkeeping (the web UI observes the same properties; the
        // dedupe set keeps every property registered once).
        observeOnce("pause")
        observeOnce("eof-reached")

        val sv = SurfaceView(activity)
        sv.holder.addCallback(this)
        sv.visibility = View.GONE
        sv.layoutParams = FrameLayout.LayoutParams(1, 1)
        val parent = (webView?.parent as? ViewGroup)
            ?: activity.findViewById<ViewGroup>(android.R.id.content)
        parent.addView(sv, 0) // behind the webview
        surfaceView = sv
        Log.i("mpv", "libmpv ready under the webview")
    }

    private fun observeOnce(name: String) {
        synchronized(observed) {
            if (!observed.add(name)) return
        }
        MPVLib.observeProperty(name, formatFor(name))
    }

    private fun formatFor(name: String): Int = when (name) {
        "pause", "mute", "eof-reached", "paused-for-cache", "seeking", "core-idle", "idle-active" ->
            MPVLib.MPV_FORMAT_FLAG
        "duration", "time-pos", "volume", "speed", "sub-delay", "audio-delay",
        "demuxer-cache-time", "demuxer-cache-duration", "cache-buffering-state" ->
            MPVLib.MPV_FORMAT_DOUBLE
        "track-list", "video-params" -> MPVLib.MPV_FORMAT_NONE
        else -> MPVLib.MPV_FORMAT_STRING
    }

    // ------------------------------------------------------------- surface

    override fun surfaceCreated(holder: SurfaceHolder) {
        if (!created) return
        MPVLib.attachSurface(holder.surface)
        MPVLib.setOptionString("force-window", "yes")
        MPVLib.setPropertyString("vo", "gpu")
    }

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
        if (!created) return
        MPVLib.setPropertyString("android-surface-size", "${width}x$height")
    }

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        if (!created) return
        MPVLib.setPropertyString("vo", "null")
        MPVLib.setOptionString("force-window", "no")
        MPVLib.detachSurface()
    }

    // ------------------------------------------------------------- events

    private fun emit(obj: JSObject) {
        val ch = channel ?: return
        activity.runOnUiThread { ch.send(obj) }
    }

    private fun eventObject(kind: String): JSObject {
        val o = JSObject()
        o.put("kind", kind)
        return o
    }

    private fun emitProperty(name: String, value: Any?) {
        val o = eventObject("property_changed")
        o.put("name", name)
        o.put("value", value)
        emit(o)
    }

    override fun eventProperty(property: String) {
        val value: Any? = when (property) {
            "track-list" -> readTrackList()
            "video-params" -> readVideoParams()
            else -> null
        }
        emitProperty(property, value)
    }

    override fun eventProperty(property: String, value: Long) = emitProperty(property, value)

    override fun eventProperty(property: String, value: Double) = emitProperty(property, value)

    override fun eventProperty(property: String, value: Boolean) {
        when (property) {
            "pause" -> { paused = value; updateKeepAwake() }
            "eof-reached" -> eofReached = value
        }
        emitProperty(property, value)
    }

    override fun eventProperty(property: String, value: String) {
        // aid/sid/vid arrive as strings: numbers for tracks, "no"/"auto" otherwise.
        val v: Any = value.toLongOrNull() ?: value
        emitProperty(property, v)
    }

    override fun event(eventId: Int) {
        when (eventId) {
            MPVLib.MPV_EVENT_START_FILE -> {
                hasFile = true
                expectStop = false
                emit(eventObject("start_file"))
            }
            MPVLib.MPV_EVENT_FILE_LOADED -> {
                hasFile = true
                eofReached = false
                emit(eventObject("file_loaded"))
            }
            MPVLib.MPV_EVENT_END_FILE -> {
                hasFile = false
                val reason = when {
                    eofReached -> "eof"
                    expectStop -> "stop"
                    else -> "error"
                }
                expectStop = false
                val o = eventObject("end_file")
                o.put("reason", reason)
                emit(o)
            }
            MPVLib.MPV_EVENT_PLAYBACK_RESTART -> emit(eventObject("playback_restart"))
            MPVLib.MPV_EVENT_SEEK -> emit(eventObject("seek"))
            MPVLib.MPV_EVENT_SHUTDOWN -> emit(eventObject("shutdown"))
            else -> {}
        }
        updateKeepAwake()
    }

    private fun readTrackList(): JSArray {
        val out = JSArray()
        val count = MPVLib.getPropertyInt("track-list/count") ?: 0
        for (i in 0 until count) {
            val p = "track-list/$i/"
            val t = JSObject()
            t.put("id", MPVLib.getPropertyInt(p + "id") ?: 0)
            t.put("type", MPVLib.getPropertyString(p + "type") ?: "")
            MPVLib.getPropertyString(p + "lang")?.let { t.put("lang", it) }
            MPVLib.getPropertyString(p + "title")?.let { t.put("title", it) }
            MPVLib.getPropertyString(p + "codec")?.let { t.put("codec", it) }
            t.put("selected", MPVLib.getPropertyBoolean(p + "selected") ?: false)
            t.put("default", MPVLib.getPropertyBoolean(p + "default") ?: false)
            t.put("forced", MPVLib.getPropertyBoolean(p + "forced") ?: false)
            t.put("external", MPVLib.getPropertyBoolean(p + "external") ?: false)
            out.put(t)
        }
        return out
    }

    private fun readVideoParams(): JSObject? {
        val w = MPVLib.getPropertyInt("video-params/w") ?: return null
        val o = JSObject()
        o.put("w", w)
        o.put("h", MPVLib.getPropertyInt("video-params/h") ?: 0)
        o.put("dw", MPVLib.getPropertyInt("video-params/dw") ?: w)
        o.put("dh", MPVLib.getPropertyInt("video-params/dh") ?: 0)
        o.put("aspect", MPVLib.getPropertyDouble("video-params/aspect") ?: 0.0)
        return o
    }

    // ------------------------------------------------------- screen / audio

    private fun updateKeepAwake() {
        val want = hasFile && !paused
        activity.runOnUiThread {
            if (want == keepAwake) return@runOnUiThread
            keepAwake = want
            if (want) {
                activity.window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            } else {
                activity.window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
            }
            val am = activity.getSystemService(Context.AUDIO_SERVICE) as AudioManager
            @Suppress("DEPRECATION")
            if (want && !audioFocus) {
                am.requestAudioFocus(null, AudioManager.STREAM_MUSIC, AudioManager.AUDIOFOCUS_GAIN)
                audioFocus = true
            } else if (!want && audioFocus) {
                am.abandonAudioFocus(null)
                audioFocus = false
            }
        }
    }
}
