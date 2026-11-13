package `fun`.hackerlife.weavelo

import android.app.*
import `fun`.hackerlife.weavelo.ui.LanguageNotificationObserver
import `fun`.hackerlife.weavelo.ui.notificationText
import android.content.Intent
import android.hardware.display.DisplayManager
import android.content.pm.ServiceInfo
import android.util.DisplayMetrics
import android.view.Display
import android.media.*
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.*
import org.json.JSONObject
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean
import `fun`.hackerlife.weavelo.managed.IManagedScreen
import `fun`.hackerlife.weavelo.managed.ManagedScreenBridge
import `fun`.hackerlife.weavelo.managed.ManagedApplications
import `fun`.hackerlife.weavelo.managed.ManagedFallbackInput

/** Owns one OS authorization and one VirtualDisplay for its entire lifetime. */
class ProjectionService : Service() {
    private var languageObserver: LanguageNotificationObserver? = null
    private val running = AtomicBoolean(false)
    private var worker: Thread? = null
    private var projection: MediaProjection? = null
    private var displayListener: DisplayManager.DisplayListener? = null
    @Volatile private var captureGeometry = ProjectionCaptureGeometry(1080, 1920, 0, 0)
    @Volatile private var audioAllowed = false
    private var audioTarget: PlaybackAudioTarget? = null
    @Volatile private var controlAllowed = false
    private var sendTarget: String? = null
    @Volatile private var captureGrant = 0L
    private var audioWorker: PlaybackAudioWorker? = null

    override fun onBind(intent: Intent?) = null
    private fun notification(): Notification {
        getSystemService(NotificationManager::class.java).createNotificationChannel(NotificationChannel("projection", notificationText("Screen sharing"), NotificationManager.IMPORTANCE_LOW))
        val stop = PendingIntent.getService(this, 2, Intent(this, ProjectionService::class.java).setAction("stop"), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val open = PendingIntent.getActivity(this, 3, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        return Notification.Builder(this, "projection").setSmallIcon(android.R.drawable.stat_sys_upload)
            .setContentTitle(notificationText("Weavelo screen sharing")).setContentText(notificationText("Sharing with your devices. Tap to return."))
            .setContentIntent(open).setOngoing(true).addAction(Notification.Action.Builder(null, notificationText("Stop sharing"), stop).build()).build()
    }
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == "stop") { ManagedScreenBridge.enable(this, false); stopSelf(); return START_NOT_STICKY }
        if (running.get()) return START_NOT_STICKY
        val managed = if (intent?.action == "managed") ManagedScreenBridge.attached() else null
        val consent = if (Build.VERSION.SDK_INT >= 33) intent?.getParcelableExtra("consent", Intent::class.java) else @Suppress("DEPRECATION") intent?.getParcelableExtra("consent")
        if (consent == null && managed == null) { stopSelf(); return START_NOT_STICKY }
        val notification = notification()
        try {
            val serviceType = if (managed != null) ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE else ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION
            if (Build.VERSION.SDK_INT >= 29) startForeground(8124, notification, serviceType)
            else startForeground(8124, notification)
            check(WeaveloClient.initialize(applicationContext)) { "Weavelo native client unavailable" }
            if (managed != null) DevicePresenceService.startIfEnabled(applicationContext)
            languageObserver = LanguageNotificationObserver(this) { getSystemService(NotificationManager::class.java).notify(8124, notification()) }
            val metrics = resources.displayMetrics
            val displays = getSystemService(DisplayManager::class.java)
            @Suppress("DEPRECATION")
            val initialRotation = displays.getDisplay(Display.DEFAULT_DISPLAY)?.rotation ?: 0
            captureGeometry = ProjectionCaptureGeometry(metrics.widthPixels, metrics.heightPixels,
                metrics.densityDpi, displayRotationDegrees(initialRotation))
            controlAllowed = intent?.getBooleanExtra("control", false) == true
            sendTarget = intent?.getStringExtra("send-target")
            audioAllowed = managed == null && intent?.getBooleanExtra("audio", false) == true && Build.VERSION.SDK_INT >= 29
            audioTarget = intent?.getStringExtra("audio-package")?.let { name ->
                check(audioAllowed && intent.hasExtra("audio-uid") && intent.hasExtra("audio-updated")) { "Invalid audio source" }
                PlaybackAudioTarget(name, intent.getIntExtra("audio-uid", -1), intent.getLongExtra("audio-updated", -1))
                    .also { check(PlaybackAudioSources.matches(this, it)) { "Selected audio app changed or is unavailable" } }
            }
            PublisherClient.audioEnabled = audioAllowed
            val capture = if (managed != null) null else checkNotNull(getSystemService(MediaProjectionManager::class.java).getMediaProjection(intent?.getIntExtra("result", Activity.RESULT_CANCELED) ?: Activity.RESULT_CANCELED, checkNotNull(consent)))
            projection = capture
            capture?.registerCallback(object : MediaProjection.Callback() {
                override fun onStop() { PublisherClient.controlSize = null; runCatching { PublisherClient.action(1) }; running.set(false); stopSelf() }
                override fun onCapturedContentResize(width: Int, height: Int) {
                    val current = captureGeometry
                    if (width > 1 && height > 1 && (current.width != width || current.height != height)) {
                        PublisherClient.controlSize = null
                        captureGeometry = current.copy(width = width, height = height)
                    }
                }
            }, Handler(Looper.getMainLooper()))
            if (managed != null) {
                val info = JSONObject(managed.info())
                captureGeometry = captureGeometry.copy(width = info.getInt("width"), height = info.getInt("height"),
                    density = info.getInt("density"))
            }
            displayListener = object : DisplayManager.DisplayListener {
                override fun onDisplayAdded(id: Int) = Unit
                override fun onDisplayRemoved(id: Int) = Unit
                override fun onDisplayChanged(id: Int) {
                    if (id != Display.DEFAULT_DISPLAY) return
                    val display = displays.getDisplay(id) ?: return
                    @Suppress("DEPRECATION")
                    val rotation = displayRotationDegrees(display.rotation)
                    val old = captureGeometry
                    var next = old.copy(rotation = rotation)
                    if (managed != null) {
                        val metrics = DisplayMetrics()
                        @Suppress("DEPRECATION")
                        display.getRealMetrics(metrics)
                        if (metrics.widthPixels > 1 && metrics.heightPixels > 1) {
                            next = next.copy(width = metrics.widthPixels, height = metrics.heightPixels, density = metrics.densityDpi)
                        }
                    }
                    if (next != old) {
                        PublisherClient.controlSize = null
                        captureGeometry = next
                    }
                }
            }.also { displays.registerDisplayListener(it, Handler(Looper.getMainLooper())) }
            running.set(true)
            worker = Thread({ publish(capture, managed) }, "remote-projection").apply { start() }
        } catch (e: Exception) { PublisherClient.audioEnabled = false; PublisherClient.status(e.message ?: "Screen sharing failed", false, if (e.message == null) AppText("Screen sharing failed") else null); stopSelf() }
        return START_NOT_STICKY
    }

    private fun publish(capture: MediaProjection?, managed: IManagedScreen?) {
        var display: CaptureDisplay? = null
        var publisher: GpuScreenPublisher? = null
        var apps: ManagedApplications? = null
        var fallbackInput: ManagedFallbackInput? = null
        var appliedGeometry: ProjectionCaptureGeometry? = null
        var demand = JSONObject()
        var nextDemand = 0L
        try {
            val directory = File(filesDir, "received").apply { mkdirs() }
            PublisherClient.action(0, JSONObject().put("bind", "0.0.0.0:5001").put("directory", directory.absolutePath).put("audio", audioAllowed).put("control",controlAllowed).toString())
            captureGrant = JSONObject(PublisherClient.action(2)).getLong("grant")
            val initial = captureGeometry
            // Android 14+ authorizes exactly one VirtualDisplay. Pause and
            // rotation replace its Surface, never create another with this token.
            display = if (managed != null) CaptureDisplay.managed(managed, initial.width, initial.height, initial.density)
                else CaptureDisplay.projection(checkNotNull(capture), initial.width, initial.height, initial.density)
            if (managed != null) {
                apps = ManagedApplications(managed, applicationContext)
                if (controlAllowed) {
                    fallbackInput = ManagedFallbackInput(applicationContext, managed) { running.get() }.also { it.start() }
                }
            }
            val addresses = java.util.Collections.list(java.net.NetworkInterface.getNetworkInterfaces()).filter { it.isUp && !it.isLoopback }
                .flatMap { java.util.Collections.list(it.inetAddresses) }.filterIsInstance<java.net.Inet4Address>().joinToString { "${it.hostAddress}:5001" }
            if (audioAllowed && audioTarget == null) PublisherClient.status("Sharing enabled · $addresses · Phone audio stays on; captured media audio may include other apps.", true,
                AppText("Sharing enabled · %s · Phone audio stays on; captured media audio may include other apps.", listOf(addresses)))
            else if (audioAllowed) PublisherClient.status("Sharing enabled · $addresses · Phone audio stays on.", true,
                AppText("Sharing enabled · %s · Phone audio stays on.", listOf(addresses)))
            else PublisherClient.status("Sharing enabled · $addresses", true, AppText("Sharing enabled · %s", listOf(addresses)))
            sendTarget?.let { target ->
                runCatching { PublisherClient.sendScreen(target) }
                    .onFailure { PublisherClient.status(it.message ?: "Could not send screen", true, appTextOf(it) ?: if (it.message == null) AppText("Could not send screen") else null) }
                sendTarget = null
            }
            var surfaceAttachedAt = 0L
            var nudgedStaticSurface = false
            var lastWakeAttemptAt = 0L
            while (running.get()) {
                if (SystemClock.elapsedRealtime() >= nextDemand) {
                    demand = JSONObject(PublisherClient.action(2)); nextDemand = SystemClock.elapsedRealtime() + 50
                    apps?.update()
                }
                val requestedViews = demand.optJSONArray("views") ?: org.json.JSONArray()
                if (managed != null && requestedViews.length() > 0) {
                    val mainDisplay = getSystemService(DisplayManager::class.java)?.getDisplay(Display.DEFAULT_DISPLAY)
                    val nowElapsed = SystemClock.elapsedRealtime()
                    if (mainDisplay != null && mainDisplay.state != Display.STATE_ON && nowElapsed - lastWakeAttemptAt >= 2000L) {
                        lastWakeAttemptAt = nowElapsed
                        if (runCatching { managed.ensureMainDisplayAwake() }.getOrDefault(false)) {
                            nudgedStaticSurface = false
                            surfaceAttachedAt = SystemClock.elapsedRealtime()
                        }
                    }
                }
                val geometry = captureGeometry
                val size = geometry.width to geometry.height
                if (captureReconfiguration(appliedGeometry, geometry) != null) {
                    PublisherClient.controlSize = null
                    display.surface = null
                    publisher?.finish(); publisher?.close(); publisher = null
                    display.resize(size.first, size.second, geometry.density)
                    // Keep the capture SurfaceTexture continuously bound across idle periods
                    // (zero encoders run while views is empty) so static phone screens
                    // already hold a latched frame the instant another phone subscribes.
                    val nextPublisher = GpuScreenPublisher(size.first, size.second)
                    display.surface = nextPublisher.input
                    publisher = nextPublisher
                    surfaceAttachedAt = SystemClock.elapsedRealtime()
                    nudgedStaticSurface = false
                    PublisherClient.action(3, JSONObject().put("grant", captureGrant).put("width", size.first).put("height", size.second)
                        .put("rotation", geometry.rotation).toString())
                    appliedGeometry = geometry
                    if (captureGeometry == geometry && controlAllowed) PublisherClient.controlSize = size
                    demand = JSONObject(PublisherClient.action(2))
                }
                val views = demand.optJSONArray("views") ?: org.json.JSONArray()
                val activePublisher = publisher
                if (activePublisher != null) {
                    if (!activePublisher.update(views)) nextDemand = 0
                    if (views.length() > 0 && !activePublisher.hasSourceFrame && !nudgedStaticSurface &&
                        SystemClock.elapsedRealtime() - surfaceAttachedAt >= 220L) {
                        nudgedStaticSurface = true
                        runCatching { display.resize(size.first, size.second, geometry.density) }
                    }
                }
                val wantsAudio = audioAllowed && demand.optBoolean("audio")
                if (wantsAudio && capture != null) startAudio(capture, captureGrant)
                if (!wantsAudio) stopAudio()
                val waitMs = (nextDemand - SystemClock.elapsedRealtime()).coerceIn(1, 50)
                val gpu = publisher
                if (gpu != null) gpu.awaitFrame(if (gpu.pendingFrames) 1 else waitMs) else Thread.sleep(waitMs)
            }
        } catch (e: Exception) {
            if (running.get()) { PublisherClient.status(e.message ?: "Projection stopped", false, if (e.message == null) AppText("Projection stopped") else null); runCatching { PublisherClient.action(4, e.message ?: "Projection stopped") } }
        } finally {
            PublisherClient.controlSize = null
            running.set(false); stopAudio()
            PublisherClient.audioEnabled = false
            runCatching { fallbackInput?.close() }
            runCatching { apps?.close() }
            runCatching { display?.surface = null }; runCatching { display?.release() }
            publisher?.close()
            runCatching { PublisherClient.action(1) }
            if (PublisherClient.active) PublisherClient.status("Screen sharing stopped", false, AppText("Screen sharing stopped"))
            stopSelf()
        }
    }

    @Suppress("MissingPermission")
    private fun startAudio(capture: MediaProjection, grant: Long) {
        if (Build.VERSION.SDK_INT < 29) return
        // A timed-out read from the previous subscription must finish before a
        // new recorder starts. Each worker owns its own cancellation flag.
        if (audioWorker?.thread?.isAlive == true) return
        val worker = PlaybackAudioWorker()
        audioWorker = worker
        worker.thread = Thread({
            var record: AudioRecord? = null
            try {
                val builder = AudioPlaybackCaptureConfiguration.Builder(capture)
                    .addMatchingUsage(AudioAttributes.USAGE_MEDIA).addMatchingUsage(AudioAttributes.USAGE_GAME)
                    .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
                val target = audioTarget
                if (target == null) builder.excludeUid(android.os.Process.myUid())
                else {
                    check(PlaybackAudioSources.matches(this, target)) { "Selected audio app changed or is unavailable" }
                    builder.addMatchingUid(target.uid)
                }
                val config = builder.build()
                val format = AudioFormat.Builder().setSampleRate(48000).setEncoding(AudioFormat.ENCODING_PCM_16BIT).setChannelMask(AudioFormat.CHANNEL_IN_STEREO).build()
                record = AudioRecord.Builder().setAudioPlaybackCaptureConfig(config).setAudioFormat(format)
                    .setBufferSizeInBytes(maxOf(7680, AudioRecord.getMinBufferSize(48000, AudioFormat.CHANNEL_IN_STEREO, AudioFormat.ENCODING_PCM_16BIT))).build()
                worker.record = record
                check(record.state == AudioRecord.STATE_INITIALIZED) { "Playback audio capture is unavailable" }
                if (!worker.active.get()) return@Thread
                record.startRecording()
                val samples = ShortArray(1920)
                var filled = 0
                while (worker.active.get() && running.get()) {
                    val count = record.read(samples, filled, samples.size - filled, AudioRecord.READ_BLOCKING)
                    check(count >= 0) { "Audio capture failed: $count" }; filled += count
                    if (filled == samples.size) {
                        check(target == null || PlaybackAudioSources.matches(this, target)) { "Selected audio app changed or is unavailable" }
                        check(PublisherClient.nativePcm(grant,samples)) { "Audio source ended or Opus encoder failed" }; filled = 0
                    }
                }
            } catch (e: Exception) {
                if (worker.active.get() && running.get()) { audioAllowed = false; PublisherClient.audioEnabled = false; PublisherClient.status("Sharing video · ${e.message}", text = AppText("Sharing video · %s", listOf(e.message ?: ""))) }
            } finally { worker.active.set(false); runCatching { record?.stop() }; record?.release(); worker.record = null }
        }, "projection-audio")
        worker.thread.start()
    }
    private class PlaybackAudioWorker {
        val active = AtomicBoolean(true)
        @Volatile var record: AudioRecord? = null
        lateinit var thread: Thread
    }
    private fun stopAudio() {
        val worker = audioWorker ?: return
        worker.active.set(false)
        // stop() unblocks READ_BLOCKING; release remains owned by the reader.
        runCatching { worker.record?.stop() }
        try { worker.thread.join(500) } catch (_: InterruptedException) { Thread.currentThread().interrupt() }
        if (!worker.thread.isAlive) audioWorker = null
    }
    override fun onDestroy() {
        languageObserver?.close()
        displayListener?.let { getSystemService(DisplayManager::class.java).unregisterDisplayListener(it) }
        displayListener = null
        running.set(false); worker?.interrupt(); runCatching { projection?.stop() }; projection = null
        super.onDestroy()
    }
}
