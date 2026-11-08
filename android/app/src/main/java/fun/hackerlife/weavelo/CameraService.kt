package `fun`.hackerlife.weavelo

import android.Manifest
import android.app.*
import android.content.Context
import `fun`.hackerlife.weavelo.ui.LanguageNotificationObserver
import `fun`.hackerlife.weavelo.ui.notificationText
import `fun`.hackerlife.weavelo.ui.cameraNameText
import android.content.Intent
import android.content.pm.PackageManager
import android.hardware.camera2.CameraManager
import android.os.Handler
import android.os.HandlerThread
import android.view.Surface
import android.view.WindowManager
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

data class LocalCameraSource(val source: Any, val cameraId: String, val name: String, val facing: String)
data class CameraSharingState(val enabled: Boolean = false, val live: Boolean = false,
    val sources: List<LocalCameraSource> = emptyList(), val activeCameraId: String? = null, val status: String = "Not sharing", val statusText: AppText? = null)

object CameraPublisher {
    private val mutable = MutableStateFlow(CameraSharingState(statusText = AppText("Not sharing")))
    val state = mutable.asStateFlow()
    internal fun update(value: CameraSharingState) { mutable.value = value }
    fun start(context: Context, target: String? = null) {
        context.startForegroundService(Intent(context, CameraService::class.java).putExtra("target", target))
    }
    fun stop(context: Context) { context.stopService(Intent(context, CameraService::class.java)) }
    fun send(target: String, camera: LocalCameraSource) {
        WeaveloClient.refreshDevices()
        val endpoint = resolveScreenDeliveryEndpoint(target, WeaveloClient.devices.value)
        PublisherClient.action(34, JSONObject().put("target", target).put("endpoint", endpoint)
            .put("sources", JSONArray().put(camera.source)).toString())
        PublisherClient.refreshOutgoing()
    }
}

/** One foreground camera owner; the existing Rust endpoint owns sessions, auth and encoded fanout. */
class CameraService : Service() {
    private var languageObserver: LanguageNotificationObserver? = null
    private val running = AtomicBoolean(false)
    private val capture = AtomicReference<CameraCapture?>(null)
    private val target = AtomicReference<String?>(null)
    private var worker: Thread? = null
    override fun onBind(intent: Intent?) = null
    override fun dump(fd: java.io.FileDescriptor, writer: java.io.PrintWriter, args: Array<out String>?) {
        writer.println("Weavelo camera delivery diagnostics")
        // Scalar native JSON only; preserve u64 IDs and exclude media, tickets and pairing secrets.
        writer.println(runCatching { PublisherClient.action(23) }.getOrDefault("Publisher diagnostics unavailable"))
    }
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == "stop") { stopSelf(); return START_NOT_STICKY }
        if (checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED) {
            CameraPublisher.update(CameraSharingState(status = "Allow camera access on this phone", statusText = AppText("Allow camera access on this phone"))); stopSelf(); return START_NOT_STICKY
        }
        target.set(intent?.getStringExtra("target"))
        if (!running.compareAndSet(false, true)) return START_NOT_STICKY
        startForeground(8125, notification())
        languageObserver = LanguageNotificationObserver(this) { getSystemService(NotificationManager::class.java).notify(8125, notification()) }
        worker = Thread({ publish() }, "weavelo-camera").apply { start() }
        return START_NOT_STICKY
    }
    private fun notification(): Notification {
        getSystemService(NotificationManager::class.java).createNotificationChannel(NotificationChannel("camera", notificationText("Camera sharing"), NotificationManager.IMPORTANCE_LOW))
        val stop = PendingIntent.getService(this, 8126, Intent(this, CameraService::class.java).setAction("stop"), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val open = PendingIntent.getActivity(this, 8127, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        return Notification.Builder(this, "camera").setSmallIcon(android.R.drawable.ic_menu_camera)
            .setContentTitle(notificationText("Weavelo camera sharing")).setContentText(notificationText("Ready for your paired devices"))
            .setContentIntent(open).setOngoing(true).addAction(Notification.Action.Builder(null, notificationText("Stop sharing"), stop).build()).build()
    }
    @Suppress("DEPRECATION")
    private fun displayDegrees(): Int = when (getSystemService(WindowManager::class.java).defaultDisplay.rotation) {
        Surface.ROTATION_90 -> 90; Surface.ROTATION_180 -> 180; Surface.ROTATION_270 -> 270; else -> 0
    }
    private fun publish() {
        val callbacks = HandlerThread("weavelo-camera-sdk").apply { start() }
        val handler = Handler(callbacks.looper)
        var activeKey = ""
        var delayedClose: CameraCapture? = null
        try {
            val descriptions = CameraDescription.enumerate(getSystemService(CameraManager::class.java))
            appCheck(descriptions.isNotEmpty(), "No video camera is available on this phone")
            val directory = File(filesDir, "received").apply { mkdirs() }
            PublisherClient.action(30, JSONObject().put("bind", "0.0.0.0:5001").put("directory", directory.absolutePath).toString())
            val registered = JSONArray(PublisherClient.action(32, JSONArray().apply {
                descriptions.forEach { put(it.json(displayDegrees())) }
            }.toString()))
            val sources = (0 until registered.length()).map { index ->
                val pair = registered.getJSONArray(index); val cameraId = pair.getString(1)
                val description = descriptions.first { it.id == cameraId }
                LocalCameraSource(pair.get(0), cameraId, description.name, description.facing)
            }
            CameraPublisher.update(CameraSharingState(enabled = true, sources = sources, status = "Ready for your devices", statusText = AppText("Ready for your devices")))
            while (running.get()) {
                appCheck(checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED, "Camera permission was revoked")
                target.getAndSet(null)?.let { device ->
                    val source = sources.firstOrNull { it.facing == "Back" } ?: sources.first()
                    runCatching { CameraPublisher.send(device, source) }.onFailure {
                        CameraPublisher.update(CameraPublisher.state.value.copy(status = it.message ?: "Camera delivery failed", statusText = appTextOf(it) ?: if (it.message == null) AppText("Camera delivery failed") else null))
                    }
                }
                val all = JSONArray(PublisherClient.action(31))
                val wanted = (0 until all.length()).map { all.getJSONObject(it) }
                    .firstOrNull { it.getJSONObject("demand").optBoolean("active") }
                if (wanted == null) {
                    capture.getAndSet(null)?.let { old -> delayedClose = old; old.close(); delayedClose = null }
                    activeKey = ""
                    val state = CameraPublisher.state.value
                    CameraPublisher.update(state.copy(live = false, activeCameraId = null,
                        status = if (state.live) "Ready for your devices" else state.status, statusText = if (state.live) AppText("Ready for your devices") else state.statusText))
                    Thread.sleep(100); continue
                }
                val source = wanted.get("source")
                val demand = wanted.getJSONObject("demand")
                val description = descriptions.first { it.id == wanted.getString("camera_id") }
                val rotation = description.rotation(displayDegrees())
                var epoch = demand.getLong("epoch")
                val grant = demand.getLong("grant")
                try {
                    // Epoch changes already cover every encoding-demand change. Camera
                    // characteristics and codec capabilities are not per-frame work.
                    fun key(value: Long) = "$source:$value:$rotation"
                    if (activeKey != key(epoch)) {
                        val plan = description.configure(demand)
                        capture.getAndSet(null)?.let { old -> delayedClose = old; old.close(); delayedClose = null }
                        val configured = JSONObject(PublisherClient.action(3, JSONObject().put("source", source)
                            .put("epoch", epoch).put("grant", grant).put("width", plan.encoder.width)
                            .put("height", plan.encoder.height).put("rotation", rotation).toString()))
                        epoch = configured.getLong("epoch")
                        val next = CameraCapture(description, plan, handler)
                        capture.set(next); next.start()
                        activeKey = key(epoch)
                        CameraPublisher.update(CameraPublisher.state.value.copy(enabled = true, live = true,
                            activeCameraId = description.id, status = "Sharing ${description.name.lowercase()}", statusText = AppText("Sharing %s", listOf(cameraNameText(description.facing)))))
                    }
                    capture.get()?.failure?.get()?.let { throw it }
                    val frameEpoch = epoch
                    capture.get()?.drain(demand.getLong("keyframe")) { bytes, pts ->
                        PublisherClient.nativeFrame(source.toString(), frameEpoch, bytes, pts)
                    }
                } catch (error: Throwable) {
                    capture.getAndSet(null)?.let { old -> delayedClose = old; runCatching { old.close() }.onSuccess { delayedClose = null } }
                    activeKey = ""
                    // A newer subscription/configuration can supersede a native open in progress.
                    val latest = JSONArray(PublisherClient.action(31))
                    val current = (0 until latest.length()).map { latest.getJSONObject(it) }.firstOrNull { it.get("source").toString() == source.toString() }
                    if (current?.getJSONObject("demand")?.optLong("epoch") == epoch && running.get()) {
                        PublisherClient.action(35, JSONObject().put("source", source).put("epoch", epoch)
                            .put("reason", error.cause?.message ?: error.message ?: "Camera capture failed").toString())
                        CameraPublisher.update(CameraPublisher.state.value.copy(live = false, activeCameraId = null,
                            status = error.cause?.message ?: error.message ?: "Camera capture failed", statusText = appTextOf(error) ?: if (error.cause?.message == null && error.message == null) AppText("Camera capture failed") else null))
                    }
                }
            }
        } catch (error: Throwable) {
            CameraPublisher.update(CameraSharingState(status = error.cause?.message ?: error.message ?: "Camera sharing stopped", statusText = appTextOf(error) ?: if (error.cause?.message == null && error.message == null) AppText("Camera sharing stopped") else null))
        } finally {
            running.set(false)
            runCatching { PublisherClient.action(33) }
            capture.getAndSet(null)?.let { old -> delayedClose = old; runCatching { old.close() }.onSuccess { delayedClose = null } }
            val pending = delayedClose
            if (pending == null) callbacks.quitSafely()
            else pending.closed.whenComplete { _, _ -> callbacks.quitSafely() }
            stopSelf()
        }
    }
    override fun onDestroy() {
        languageObserver?.close()
        running.set(false)
        runCatching { PublisherClient.action(33) }
        capture.get()?.requestClose()
        val state = CameraPublisher.state.value
        CameraPublisher.update(CameraSharingState(status = if (state.enabled) "Not sharing" else state.status, statusText = if (state.enabled) AppText("Not sharing") else state.statusText))
        super.onDestroy()
    }
}
