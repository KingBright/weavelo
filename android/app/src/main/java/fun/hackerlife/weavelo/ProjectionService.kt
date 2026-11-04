package `fun`.hackerlife.weavelo

import android.app.*
import android.content.Intent
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.*
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.*
import org.json.JSONObject
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean

/** Owns one OS authorization and one VirtualDisplay for its entire lifetime. */
class ProjectionService : Service() {
    private val running = AtomicBoolean(false)
    private var worker: Thread? = null
    private var projection: MediaProjection? = null
    @Volatile private var captureSize = 1080 to 1920
    @Volatile private var audioAllowed = false
    @Volatile private var controlAllowed = false
    private var sendTarget: String? = null
    @Volatile private var captureGrant = 0L
    private val audioRunning = AtomicBoolean(false)
    private var audioWorker: Thread? = null

    override fun onBind(intent: Intent?) = null
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == "stop") { stopSelf(); return START_NOT_STICKY }
        if (running.get()) return START_NOT_STICKY
        val consent = if (Build.VERSION.SDK_INT >= 33) intent?.getParcelableExtra("consent", Intent::class.java) else @Suppress("DEPRECATION") intent?.getParcelableExtra("consent")
        if (consent == null) { stopSelf(); return START_NOT_STICKY }
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel("projection", "Screen sharing", NotificationManager.IMPORTANCE_LOW))
        val stop = PendingIntent.getService(this, 2, Intent(this, ProjectionService::class.java).setAction("stop"), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val open = PendingIntent.getActivity(this, 3, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        startForeground(8124, Notification.Builder(this, "projection").setSmallIcon(android.R.drawable.stat_sys_upload)
            .setContentTitle("Weavelo screen sharing").setContentText("Sharing with your devices. Tap to return.")
            .setContentIntent(open).setOngoing(true).addAction(Notification.Action.Builder(null, "Stop sharing", stop).build()).build())
        try {
            val metrics = resources.displayMetrics
            captureSize = metrics.widthPixels to metrics.heightPixels
            controlAllowed = intent?.getBooleanExtra("control", false) == true
            sendTarget = intent?.getStringExtra("send-target")
            audioAllowed = intent?.getBooleanExtra("audio", false) == true && Build.VERSION.SDK_INT >= 29
            val capture = checkNotNull(getSystemService(MediaProjectionManager::class.java).getMediaProjection(intent?.getIntExtra("result", Activity.RESULT_CANCELED) ?: Activity.RESULT_CANCELED, consent))
            projection = capture
            capture.registerCallback(object : MediaProjection.Callback() {
                override fun onStop() { PublisherClient.controlSize = null; runCatching { PublisherClient.action(1) }; running.set(false); stopSelf() }
                override fun onCapturedContentResize(width: Int, height: Int) { if (width > 1 && height > 1 && captureSize != (width to height)) {
                    PublisherClient.controlSize = null
                    captureSize = width to height
                } }
            }, Handler(Looper.getMainLooper()))
            running.set(true)
            worker = Thread({ publish(capture, metrics.densityDpi) }, "remote-projection").apply { start() }
        } catch (e: Exception) { PublisherClient.status(e.message ?: "Screen sharing failed", false); stopSelf() }
        return START_NOT_STICKY
    }

    private fun publish(capture: MediaProjection, density: Int) {
        var display: VirtualDisplay? = null
        var publisher: GpuScreenPublisher? = null
        var sourceSize = 0 to 0
        var demand = JSONObject()
        var nextDemand = 0L
        try {
            val directory = File(filesDir, "received").apply { mkdirs() }
            PublisherClient.action(0, JSONObject().put("bind", "0.0.0.0:5001").put("directory", directory.absolutePath).put("audio", audioAllowed).put("control",controlAllowed).toString())
            captureGrant = JSONObject(PublisherClient.action(2)).getLong("grant")
            val initialSize=captureSize
            // Android 14+ authorizes exactly one VirtualDisplay. Pause and
            // rotation replace its Surface, never create another with this token.
            display = capture.createVirtualDisplay("Weavelo", initialSize.first, initialSize.second, density,
                DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, null, null, Handler(Looper.getMainLooper()))
            val addresses = java.util.Collections.list(java.net.NetworkInterface.getNetworkInterfaces()).filter { it.isUp && !it.isLoopback }
                .flatMap { java.util.Collections.list(it.inetAddresses) }.filterIsInstance<java.net.Inet4Address>().joinToString { "${it.hostAddress}:5001" }
            PublisherClient.status("Sharing enabled · $addresses", true)
            sendTarget?.let { target ->
                runCatching { PublisherClient.sendScreen(target) }
                    .onFailure { PublisherClient.status(it.message ?: "Could not send screen", true) }
                sendTarget = null
            }
            while (running.get()) {
                if (SystemClock.elapsedRealtime() >= nextDemand) {
                    demand = JSONObject(PublisherClient.action(2)); nextDemand = SystemClock.elapsedRealtime() + 50
                }
                val size=captureSize
                if (sourceSize != size) {
                    PublisherClient.controlSize = null
                    display.surface = null
                    publisher?.finish();publisher?.close();publisher=null
                    display.resize(size.first,size.second,density)
                    PublisherClient.action(3, JSONObject().put("grant",captureGrant).put("width",size.first).put("height",size.second).toString())
                    sourceSize=size
                    if (captureSize == size && controlAllowed) PublisherClient.controlSize=size
                    demand = JSONObject(PublisherClient.action(2))
                }
                val views=demand.optJSONArray("views") ?: org.json.JSONArray()
                if (views.length()>0) {
                    if (publisher == null) {
                        publisher=GpuScreenPublisher(size.first,size.second)
                        display.surface=publisher.input
                    }
                    if (!publisher.update(views)) nextDemand=0
                } else if (publisher != null) {
                    display.surface=null;publisher.close();publisher=null
                }
                val wantsAudio = audioAllowed && demand.optBoolean("audio")
                if (wantsAudio && !audioRunning.get()) startAudio(capture,captureGrant)
                if (!wantsAudio && audioRunning.get()) stopAudio()
                val waitMs=(nextDemand-SystemClock.elapsedRealtime()).coerceIn(1,50)
                val gpu=publisher
                if (gpu != null) gpu.awaitFrame(if (gpu.pendingFrames) 1 else waitMs) else Thread.sleep(waitMs)
            }
        } catch (e: Exception) {
            if (running.get()) { PublisherClient.status(e.message ?: "Projection stopped", false); runCatching { PublisherClient.action(4, e.message ?: "Projection stopped") } }
        } finally {
            PublisherClient.controlSize = null
            running.set(false); stopAudio()
            display?.surface = null; display?.release()
            publisher?.close()
            runCatching { PublisherClient.action(1) }
            if (PublisherClient.active) PublisherClient.status("Screen sharing stopped", false)
            stopSelf()
        }
    }

    @Suppress("MissingPermission")
    private fun startAudio(capture: MediaProjection, grant: Long) {
        if (Build.VERSION.SDK_INT < 29) return
        audioRunning.set(true)
        audioWorker = Thread({
            var record: AudioRecord? = null
            try {
                val config = AudioPlaybackCaptureConfiguration.Builder(capture)
                    .addMatchingUsage(AudioAttributes.USAGE_MEDIA).addMatchingUsage(AudioAttributes.USAGE_GAME)
                    .excludeUid(android.os.Process.myUid()).build()
                val format = AudioFormat.Builder().setSampleRate(48000).setEncoding(AudioFormat.ENCODING_PCM_16BIT).setChannelMask(AudioFormat.CHANNEL_IN_STEREO).build()
                record = AudioRecord.Builder().setAudioPlaybackCaptureConfig(config).setAudioFormat(format)
                    .setBufferSizeInBytes(maxOf(7680, AudioRecord.getMinBufferSize(48000, AudioFormat.CHANNEL_IN_STEREO, AudioFormat.ENCODING_PCM_16BIT))).build()
                check(record.state == AudioRecord.STATE_INITIALIZED) { "Playback audio capture is unavailable" }
                record.startRecording()
                val samples = ShortArray(1920)
                var filled = 0
                while (audioRunning.get() && running.get()) {
                    val count = record.read(samples, filled, samples.size - filled, AudioRecord.READ_BLOCKING)
                    check(count >= 0) { "Audio capture failed: $count" }; filled += count
                    if (filled == samples.size) { check(PublisherClient.nativePcm(grant,samples)) { "Audio source ended or Opus encoder failed" }; filled = 0 }
                }
            } catch (e: Exception) {
                if (audioRunning.get()) { audioAllowed = false; PublisherClient.status("Sharing video · ${e.message}") }
            } finally { runCatching { record?.stop() }; record?.release(); audioRunning.set(false) }
        }, "projection-audio").apply { start() }
    }
    private fun stopAudio() { audioRunning.set(false); audioWorker?.join(500); audioWorker = null }
    override fun onDestroy() {
        running.set(false); worker?.interrupt(); runCatching { projection?.stop() }; projection = null
        super.onDestroy()
    }
}
