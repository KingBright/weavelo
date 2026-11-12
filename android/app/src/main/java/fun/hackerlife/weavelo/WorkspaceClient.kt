package `fun`.hackerlife.weavelo

import android.content.Context
import android.content.Intent
import android.os.SystemClock
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.concurrent.Executors

// Each source has its own subscription; audio owners may be shared by same-app windows.
data class WorkspaceSource(val source: Any, val title: String, val width: Int, val height: Int, val application: String = "", val icon: android.graphics.Bitmap? = null, val camera: JSONObject? = null) {
    val isWindow: Boolean get() = (source as? JSONObject)?.has("Window") == true
    val isCamera: Boolean get() = (source as? JSONObject)?.has("Camera") == true
    val isDisplay: Boolean get() = source == "MainDisplay" || (source as? JSONObject)?.has("Display") == true
    val rotation: Int get() = camera?.optInt("rotation", 0) ?: 0
    val group: String get() = if (isCamera) "Cameras" else if (isWindow) "App windows" else "Entire screens"
}
data class WorkspaceAudioOutput(val pendingRevision: Long? = null,
    val requestRevision: Long? = null, val stateGeneration: Long = -1,
    val localPlayback: String = "Unknown", val privateSupported: Boolean? = null,
    val message: String? = null, val messageText: AppText? = null)

data class WorkspacePane(val id: Int, val source: WorkspaceSource, val fps: Int, val kbps: Int,
    val width: Int = 1280, val height: Int = 720, val audioOwner: Int? = null, val input: Boolean = false,
    val accepted: Boolean = false, val appearance: RemoteAppearance? = null, val inputError: String? = null, val videoError: String? = null,
    val audioOutput: WorkspaceAudioOutput = WorkspaceAudioOutput(),
    val maxTouchPoints: Int = 0, val inputErrorText: AppText? = null, val videoErrorText: AppText? = null, val telemetry: TelemetrySnapshot = TelemetrySnapshot(),
    val windowScalePercent: Int = 100, val windowLayoutPreset: String = "original",
    val windowResizeNotice: String? = null, val windowResizeError: Boolean = false)
internal enum class WorkspaceNavigation(val wireValue: String) { BACK("back"), HOME("home") }

internal fun isAndroidProjectionSource(source: String, application: String): Boolean =
    source == "MainDisplay" && application == "MediaProjection"

internal fun primaryScreenSource(sources: List<WorkspaceSource>): WorkspaceSource? {
    val hasConcreteDisplay = sources.any { (it.source as? JSONObject)?.has("Display") == true }
    val screens = sources.filter {
        !it.isWindow && !it.isCamera && !(hasConcreteDisplay && it.source.toString() == "MainDisplay")
    }
    return screens.firstOrNull { it.width > 0 && it.height > 0 } ?: screens.firstOrNull()
}

data class RemoteSharedFile(val id: Long, val name: String, val sizeBytes: Long)
data class WorkspaceState(val deviceId: String = "", val transferring: Boolean = false, val remoteFiles: List<RemoteSharedFile> = emptyList(), val sharedNext: Long? = null, val sharedBusy: Boolean = false, val sessionKey: String = "", val connectionGeneration: Long = 0, val connected: Boolean = false, val clipboard: Boolean = false, val maxSubscriptions: Int = 4,
    val files: Boolean = false, val clipboardAvailable: Boolean = false, val sources: List<WorkspaceSource> = emptyList(),
    val sourcesLoaded: Boolean = false,
    val androidHost: Boolean = false,
    val panes: List<WorkspacePane> = emptyList(), val status: String = "Connecting…", val grid: Boolean = true, val selected: Int = 0,
    val incomingEnded: Boolean = false, val connectionId: Long = 0,
    val statusText: AppText? = AppText("Connecting…"))

object WorkspaceClient {
    private external fun nativeAction(operation: Int, payload: String): String
    external fun nativeMedia(id: Int, audio: Boolean): ByteArray?
    private val dispatcher = Executors.newSingleThreadExecutor().asCoroutineDispatcher()
    private val scope = CoroutineScope(SupervisorJob() + dispatcher)
    private val ownership = WorkspaceSessionOwnership()
    private val initialSource = WorkspaceInitialSource()
    private val pendingStarts = java.util.concurrent.atomic.AtomicInteger()
    val starting: Boolean get() = pendingStarts.get() > 0
    private val mutable = MutableStateFlow(WorkspaceState())
    private val applicationIcons = HashMap<String, android.graphics.Bitmap>()
    val state = mutable.asStateFlow()
    data class EditorFocus(val id: Int, val sequence: String, val binding: JSONObject)
    private val focusEvents = MutableSharedFlow<EditorFocus>(extraBufferCapacity = 16)
    val editorFocus = focusEvents.asSharedFlow()
    private val editorFocusEpoch = java.util.concurrent.atomic.AtomicLong()
    internal fun invalidateEditorFocus() { editorFocusEpoch.incrementAndGet() }
    internal fun editorFocusIntent(): Long = editorFocusEpoch.get()
    @Volatile var active = false
        private set
    @Volatile private var background = false
    private var backgroundAt = 0L
    private var context: Context? = null
    private var endpoint = ""
    private var deviceId = ""
    private var connectionGeneration = 0L
    private val surfaceTokens = java.util.concurrent.atomic.AtomicLong()
    fun nextSurfaceToken(): Long = surfaceTokens.incrementAndGet()
    private var nextId = 256
    private var revision = 0L
    private var shareRequestId = 0L
    private var pendingShareList = 0L
    private var polling: Job? = null
    private var asleep = false
    @Volatile var retainingConnection = false
        private set
    private var transfersActive = false
    private var lastFileAction = 0L
    private var pendingClipboard: JSONObject? = null
    private var incomingInitialized = false
    private var incomingEnded = false
    private val cameraReplacements = HashMap<Int, Pair<Int, Long>>()
    private data class CameraSwitch(val pane: Int, val source: WorkspaceSource, val deadline: Long)
    private val cameraSwitches = HashMap<Long, CameraSwitch>()
    private data class PendingAudioOutput(val pane: Int, val deadline: Long)
    private val pendingAudioOutputs = HashMap<Long, PendingAudioOutput>()
    private var windowResizeSeq = 20_000
    private val pendingWindowResizes = HashMap<Int, Pair<Int, Long>>()

    private fun action(op: Int, payload: String = ""): String {
        val result = nativeAction(op, payload)
        if (result.startsWith("{\"error\"")) throw IllegalStateException(JSONObject(result).getString("error"))
        return result
    }
    private fun command(name: String, value: JSONObject) = action(2, JSONObject().put(name, value).toString())
    private fun request(p: WorkspacePane) = JSONObject().put("id", p.id).put("source", p.source.source)
        .put("width", p.width).put("height", p.height).put("fps", p.fps).put("bitrate_kbps", p.kbps).put("audio", true)
    private fun guarded(block: () -> Unit) = scope.launch {
        runCatching(block).onFailure { mutable.value = mutable.value.copy(status = it.message ?: "Connection failed", statusText = appTextOf(it) ?: if (it.message == null) AppText("Connection failed") else null) }
    }
    // One bounded local record remains available independently of logcat.
    // Never include tickets, addresses, file names, clipboard or typed text.
    fun videoDiagnostic(id: Int, sessionKey: String, generation: Long, token: Long,
        stage: String, received: Long, submitted: Long, decoded: Long) = guarded {
        if (!currentView(id, sessionKey, generation, false)) return@guarded
        val pane = mutable.value.panes.firstOrNull { it.id == id } ?: return@guarded
        val folder = context?.externalCacheDir ?: return@guarded
        val record = JSONObject().put("schema", 1).put("pid", android.os.Process.myPid())
            .put("connection_id", mutable.value.connectionId).put("generation", generation)
            .put("subscription_id", id).put("surface_token", token).put("source", pane.source.source)
            .put("stage", stage).put("received", received).put("submitted", submitted).put("decoded", decoded)
        val file = android.util.AtomicFile(File(folder, "workspace-video.json"))
        var output: java.io.FileOutputStream? = null
        runCatching {
            val stream = file.startWrite(); output = stream
            stream.write(record.toString().toByteArray(Charsets.UTF_8)); file.finishWrite(stream); output = null
        }.onFailure { error ->
            output?.let(file::failWrite)
            android.util.Log.w("WeaveloWorkspace", "Local video diagnostic unavailable", error)
        }
    }

    fun requestedSession(): WorkspaceSessionRequest? = ownership.request()
    fun visibleSession(): WorkspaceSessionRequest? = ownership.visibleRequest()
    fun sessionKeyFor(endpoint: String, deviceId: String): String = ownership.request()
        ?.takeIf { it.endpoint == endpoint && it.deviceId == deviceId }?.sessionKey
        ?: java.util.UUID.randomUUID().toString()

    fun resumeOnForeground(context: Context) {
        ownership.request()?.takeIf(::canOpenRequest)?.let { SessionConnectionService.openWorkspace(context, it) }
    }

    private fun canOpenRequest(request: WorkspaceSessionRequest): Boolean = request.incomingId?.let {
        DirectedDelivery.alive(it) && !(mutable.value.sessionKey == request.sessionKey && mutable.value.incomingEnded)
    } ?: true

    // Called only by the foreground Service. All native work remains serialized;
    // the polling Job is a child of the Service rather than the UI/global scope.
    fun openFromService(context: Context, ownerScope: CoroutineScope, owner: Any, request: WorkspaceSessionRequest, forceReconnect: Boolean = false) {
        pendingStarts.incrementAndGet()
        guarded {
            try {
                if (!ownerScope.isActive || !canOpenRequest(request)) return@guarded
                val previous = ownership.request()
                val sameSession = previous == request && mutable.value.sessionKey == request.sessionKey
                initialSource.bind(request, sameSession)
                val reuse = ownership.attach(owner, request, active && !asleep && !forceReconnect)
                if (!reuse) {
                    closeNative()
                    this.context = context.applicationContext
                    endpoint = request.endpoint; deviceId = request.deviceId
                    mutable.value = if (sameSession) mutable.value.copy(connected = false, sourcesLoaded = false, androidHost = false, status = "Connecting…", statusText = AppText("Connecting…"))
                        else WorkspaceState(sessionKey = request.sessionKey, deviceId = request.deviceId).also { applicationIcons.clear() }
                    if (!sameSession) nextId = 256
                    revision = 0; asleep = false
                    background = WeaveloClient.backgrounded; transfersActive = false
                    if (background) backgroundAt = SystemClock.elapsedRealtime()
                    connectNative()
                    active = true
                    if (sameSession) {
                        mutable.value.panes.forEach { command("Subscribe", this.request(it)) }
                        if (mutable.value.clipboard) command("SetClipboard", JSONObject().put("enabled", true))
                        activity()
                    }
                }
                polling?.cancel()
                polling = ownerScope.launch(dispatcher) {
                    while (isActive && active) {
                        runCatching {
                            if (!incomingEnded && ownership.request()?.incomingId?.let { !DirectedDelivery.alive(it) } == true) endIncoming("Sending stopped", reasonText = AppText("Sending stopped"))
                            if (!asleep) drainEvents()
                            retireEndedIncoming()
                        }.onFailure { mutable.value = mutable.value.copy(status = it.message ?: "Connection error", statusText = appTextOf(it) ?: if (it.message == null) AppText("Connection error") else null) }
                        if (background && !asleep) {
                            val seconds = BackgroundConnectionPolicy.currentSeconds(context)
                            if (seconds > 0 && !transfersActive && SystemClock.elapsedRealtime() - lastFileAction >= 5_000 && SystemClock.elapsedRealtime() - backgroundAt >= seconds * 1000L) {
                                action(1); asleep = true; retainingConnection = false
                                mutable.value = mutable.value.copy(connected = false, androidHost = false, status = "Connection sleeping to save power", statusText = AppText("Connection sleeping to save power"))
                            }
                        }
                        delay(if (background) 1000 else 50)
                    }
                }
            } catch (error: Exception) {
                request.incomingId?.let { DirectedDelivery.finish(it, "Could not open the offered sources", rejected = true) }
                ownership.release(owner, preserveForForeground = true)
                throw error
            } finally { pendingStarts.decrementAndGet() }
        }
    }

    fun disconnectSession(sessionKey: String) = guarded {
        val incoming = ownership.request()?.takeIf { it.sessionKey == sessionKey }?.incomingId
        if (!ownership.disconnect(sessionKey)) return@guarded
        initialSource.cancel(sessionKey)
        incoming?.let { DirectedDelivery.finish(it) }
        closeNative()
        applicationIcons.clear()
        mutable.value = WorkspaceState(status = "Disconnected", statusText = AppText("Disconnected"))
    }

    fun closeFromService(owner: Any, preserveForForeground: Boolean) = guarded {
        if (!ownership.release(owner, preserveForForeground)) return@guarded
        closeNative()
        mutable.value = if (preserveForForeground) mutable.value.copy(connected = false, androidHost = false,
            panes = mutable.value.panes.map { it.copy(input = false, accepted = false) },
            status = if (mutable.value.connected) "Connection sleeping; reopen to reconnect" else mutable.value.status,
            statusText = if (mutable.value.connected) AppText("Connection sleeping; reopen to reconnect") else mutable.value.statusText)
            else WorkspaceState(status = "Disconnected", statusText = AppText("Disconnected"))
    }
    private fun connectNative() {
        val directory = File(checkNotNull(context).filesDir, "received").apply { mkdirs() }
        WeaveloClient.refreshDevices()
        val target = resolveNetworkEndpoint(deviceId, endpoint, WeaveloClient.devices.value)
        val payload = JSONObject().put("endpoint", target).put("directory", directory.absolutePath)
        ownership.request()?.incomingId?.let { payload.put("claim", DirectedDelivery.claim(it)) }
        val capabilities = JSONObject(action(0, payload.toString()))
        incomingInitialized = false; incomingEnded = false
        retainingConnection = true
        pendingShareList = 0L
        mutable.value = mutable.value.copy(connected = true, sourcesLoaded = false, androidHost = false, incomingEnded = false, maxSubscriptions = capabilities.getInt("max_subscriptions"),
            files = capabilities.getBoolean("files"), clipboardAvailable = capabilities.getBoolean("clipboard"),
            connectionGeneration = ++connectionGeneration, connectionId = capabilities.getLong("connection_id"), panes = mutable.value.panes.map { it.copy(input = false, accepted = false, inputError = null, videoError = null, inputErrorText = null, videoErrorText = null, audioOutput = WorkspaceAudioOutput(), telemetry = TelemetrySnapshot()) }, remoteFiles = emptyList(), sharedNext = null, sharedBusy = false, status = "Connected", statusText = AppText("Connected"))
        android.util.Log.i("WeaveloWorkspace", "Connected: connection=${mutable.value.connectionId}, generation=${mutable.value.connectionGeneration}")
    }
    private fun closeNative() { polling?.cancel(); polling = null; cameraReplacements.clear(); cameraSwitches.clear(); pendingAudioOutputs.clear(); if (active) action(1); active = false; retainingConnection = false }
    fun reconnect() {
        ownership.request()?.takeIf(::canOpenRequest)?.let { SessionConnectionService.openWorkspace(checkNotNull(context), it, forceReconnect = true) }
    }
    fun backgrounded(value: Boolean) = guarded {
        if (!active || background == value) return@guarded
        background = value
        if (value) backgroundAt = SystemClock.elapsedRealtime()
        if (!value && asleep) {
            ownership.request()?.takeIf(::canOpenRequest)?.let { SessionConnectionService.openWorkspace(checkNotNull(context), it) }
        }
        if (!value) pendingClipboard?.let { applyClipboard(it); pendingClipboard = null }
        activity()
    }
    fun refreshSources(sessionKey: String, generation: Long) = guarded {
        val current = mutable.value
        if (!active || !current.connected || current.sessionKey != sessionKey || current.connectionGeneration != generation) return@guarded
        command("ListSources", JSONObject().put("request_id", 1).put("known_icons", JSONArray()))
    }
    fun add(source: WorkspaceSource, sessionKey: String, generation: Long) = guarded {
        val current = mutable.value
        if (!active || !current.connected || current.sessionKey != sessionKey || current.connectionGeneration != generation) return@guarded
        if (source !in current.sources) { mutable.value = current.copy(status = "Source list changed; choose the window again", statusText = AppText("Source list changed; choose the window again")); return@guarded }
        initialSource.cancel(sessionKey)
        addNative(source)
    }
    private fun windowOverrideKey(source: WorkspaceSource): String? {
        if (!source.isWindow) return null
        val dev = deviceId.trim()
        val app = source.application.trim().ifEmpty { source.title.trim() }
        return if (dev.isNotEmpty() && app.isNotEmpty()) "$dev::$app" else null
    }
    fun fitRemoteWindowViewport(viewW: Int, viewH: Int): Pair<Int, Int> {
        if (viewW < 64 || viewH < 64) return 1280 to 800
        val shortEdge = minOf(viewW, viewH).toDouble()
        val upscale = if (shortEdge < 680.0) 720.0 / shortEdge else 1.0
        var w = viewW * upscale
        var h = viewH * upscale
        val downscale = minOf(2560.0 / w, 2560.0 / h, 1.0)
        w *= downscale
        h *= downscale
        return ((w.toInt() and -2).coerceIn(320, 2560)) to ((h.toInt() and -2).coerceIn(320, 2560))
    }
    fun presetWindowTargetSize(preset: String, viewW: Int = 0, viewH: Int = 0): Pair<Int, Int>? = when (preset) {
        "fit_viewport" -> if (viewW >= 64 && viewH >= 64) fitRemoteWindowViewport(viewW, viewH) else null
        "1280x800" -> 1280 to 800
        "1440x900" -> 1440 to 900
        "1600x1000" -> 1600 to 1000
        "900x1440" -> 900 to 1440
        else -> null
    }
    fun scaledEncodeSize(srcWidth: Int, srcHeight: Int, scalePercent: Int): Pair<Int, Int> {
        val sw = if (srcWidth >= 2) srcWidth else 1920
        val sh = if (srcHeight >= 2) srcHeight else 1080
        val pct = scalePercent.coerceIn(25, 100) / 100.0
        val rawW = (sw * pct).toInt().coerceAtLeast(32)
        val rawH = (sh * pct).toInt().coerceAtLeast(32)
        val fitted = runCatching { MediaCodecPlayer.screenSize(rawW, rawH) }.getOrNull()
        if (fitted != null && fitted.width >= 32 && fitted.height >= 32) {
            return fitted.width to fitted.height
        }
        val edgeScale = minOf(4096.0 / rawW, 4096.0 / rawH, 1.0)
        val areaScale = minOf(kotlin.math.sqrt(8_294_400.0 / (rawW.toDouble() * rawH.toDouble())), 1.0)
        val s = minOf(edgeScale, areaScale)
        return (((rawW * s).toInt() and -2).coerceIn(32, 4096)) to (((rawH * s).toInt() and -2).coerceIn(32, 4096))
    }
    private fun addNative(source: WorkspaceSource) {
        val incoming = ownership.request()?.incomingId
        if (incoming != null && source.source.toString() !in DirectedDelivery.sources(incoming)) {
            mutable.value = mutable.value.copy(status = "This view is bound to the offered sources", statusText = AppText("This view is bound to the offered sources")); return
        }
        val existing = mutable.value.panes.firstOrNull { it.source.source.toString() == source.source.toString() }
        if (existing != null) { mutable.value = mutable.value.copy(selected = existing.id); activity(); return }
        // Conservative default for mobile hardware; source count is distinct from visible decoder count.
        if (mutable.value.panes.size >= minOf(4, mutable.value.maxSubscriptions)) { mutable.value = mutable.value.copy(status = "This connection supports ${minOf(4, mutable.value.maxSubscriptions)} window(s)", statusText = AppText("This connection supports %d window(s)", listOf(minOf(4, mutable.value.maxSubscriptions)))); return }
        val ctx = checkNotNull(context)
        val prefs = ctx.getSharedPreferences("stream", Context.MODE_PRIVATE)
        val defaultFps = prefs.getInt("fps", 60)
        val defaultKbps = prefs.getInt("bitrate_kbps", 20_000)
        var scalePercent = 100
        var layoutPreset = "original"
        var paneFps = defaultFps
        var paneKbps = defaultKbps
        windowOverrideKey(source)?.let { key ->
            ctx.getSharedPreferences("window_stream_overrides", Context.MODE_PRIVATE).getString(key, null)?.let { raw ->
                runCatching {
                    val obj = JSONObject(raw)
                    scalePercent = obj.optInt("scale", 100).coerceIn(35, 100)
                    layoutPreset = obj.optString("layout", "original")
                    paneFps = obj.optInt("fps", defaultFps).coerceAtLeast(1)
                    paneKbps = obj.optInt("kbps", defaultKbps).coerceAtLeast(500)
                }
            }
        }
        val (initW, initH) = if (source.isWindow) scaledEncodeSize(source.width, source.height, scalePercent) else 1280 to 720
        val pane = WorkspacePane(nextId, source, paneFps, paneKbps, width = initW, height = initH,
            windowScalePercent = scalePercent, windowLayoutPreset = layoutPreset)
        nextId += 256
        mutable.value = mutable.value.copy(panes = mutable.value.panes + pane, selected = pane.id, status = "Connected", statusText = AppText("Connected"))
        command("Subscribe", request(pane)); activity()
    }
    fun remove(id: Int) = guarded {
        cameraSwitches.entries.removeAll { it.value.pane == id }
        pendingWindowResizes.entries.removeAll { it.value.first == id }
        cameraReplacements.entries.removeAll { it.value.first == id }
        command("Unsubscribe", JSONObject().put("id", id))
        removePane(id, "Receiving stopped", reasonText = AppText("Receiving stopped"))
    }
    fun retry(id: Int, sessionKey: String, generation: Long) = guarded {
        if (!currentView(id, sessionKey, generation, false)) return@guarded
        val pane = mutable.value.panes.firstOrNull { it.id == id && it.videoError != null && !it.accepted } ?: return@guarded
        val next = pane.copy(id = nextId, audioOwner = null, input = false, inputError = null, videoError = null, inputErrorText = null, videoErrorText = null)
        nextId += 256
        mutable.value = mutable.value.copy(panes = mutable.value.panes.map { if (it.id == id) next else it },
            selected = if (mutable.value.selected == id) next.id else mutable.value.selected)
        runCatching { command("Subscribe", request(next)); activity() }.onFailure { error ->
            mutable.value = mutable.value.copy(panes = mutable.value.panes.map {
                if (it.id == next.id) it.copy(videoError = error.message ?: "Could not open this source", videoErrorText = appTextOf(error) ?: if (error.message == null) AppText("Could not open this source") else null) else it
            })
        }
    }
    fun cameraSwitchTarget(pane: WorkspacePane): WorkspaceSource? {
        if (!pane.source.isCamera || !pane.accepted) return null
        if (ownership.request()?.incomingId?.let { DirectedDelivery.requestId(it) == null } == true) return null
        val group = pane.source.camera?.optLong("switch_group") ?: 0
        if (group == 0L) return null
        val wanted = if (pane.source.camera?.optString("facing") == "Front") "Back" else "Front"
        return mutable.value.sources.firstOrNull { it.isCamera && it.camera?.optLong("switch_group") == group &&
            it.camera?.optString("facing") == wanted && it.source.toString() != pane.source.source.toString() }
    }
    fun switchCamera(id: Int, sessionKey: String, generation: Long) = guarded {
        if (!currentView(id, sessionKey, generation, false) || cameraSwitches.values.any { it.pane == id }) return@guarded
        val pane = mutable.value.panes.firstOrNull { it.id == id } ?: return@guarded
        val target = cameraSwitchTarget(pane) ?: return@guarded
        val deliveryRequestId = ownership.request()?.incomingId?.let { DirectedDelivery.requestId(it) ?: return@guarded }
        val request = ++revision
        command("RequestCameraSwitch", JSONObject().put("id", id).put("request_id", request)
            .put("source_revision", 0).put("from", pane.source.source).put("target", target.source)
            .put("delivery_request_id", deliveryRequestId ?: JSONObject.NULL))
        cameraSwitches[request] = CameraSwitch(id, target, SystemClock.elapsedRealtime() + 10_000)
    }
    private fun replaceCameraView(pane: WorkspacePane, source: WorkspaceSource) {
        if (cameraReplacements.values.any { it.first==pane.id }) return
        val next=pane.copy(id=nextId,source=source,accepted=false,input=false,audioOwner=null)
        nextId+=256
        command("Unsubscribe",JSONObject().put("id",pane.id))
        cameraReplacements[pane.id]=next.id to (SystemClock.elapsedRealtime()+10_000)
        mutable.value=mutable.value.copy(panes=mutable.value.panes.map { if(it.id==pane.id) next else it },
            selected=if(mutable.value.selected==pane.id) next.id else mutable.value.selected)
    }
    private fun removePane(id: Int, reason: String, rejected: Boolean = false, reasonText: AppText? = null) {
        val previous = mutable.value
        if (previous.panes.none { it.id == id }) return
        val panes = previous.panes.filter { it.id != id }
        mutable.value = previous.copy(panes = panes,
            selected = previous.selected.takeIf { selected -> panes.any { it.id == selected } } ?: panes.firstOrNull()?.id ?: 0)
        if (panes.isEmpty() && ownership.request()?.incomingId != null && !incomingEnded) endIncoming(reason, rejected, reasonText)
        else activity()
    }
    fun layout(grid: Boolean, selected: Int = mutable.value.selected) = guarded {
        mutable.value = mutable.value.copy(grid = grid, selected = selected); activity()
    }
    private fun activity() {
        if (asleep) return
        val s = mutable.value
        s.panes.forEach { pane ->
            val visible = !background && (s.grid || s.selected == pane.id)
            command("SetActivity", JSONObject().put("id", pane.id).put("revision", ++revision).put("video", visible).put("audio", visible))
        }
    }
    fun setAudioOutput(id: Int, sessionKey: String, generation: Long, mode: String) = guarded {
        if ((mode != "Mirror" && mode != "Private") || mutable.value.androidHost || !currentView(id, sessionKey, generation, true)) return@guarded
        val pane = mutable.value.panes.firstOrNull { it.id == id && it.accepted && it.source.isDisplay } ?: return@guarded
        if (pane.audioOutput.pendingRevision != null) return@guarded
        val requestRevision = ++revision
        val pending = PendingAudioOutput(id, SystemClock.elapsedRealtime() + 10_000)
        pendingAudioOutputs[requestRevision] = pending
        updateAudioOutput(id) { it.copy(pendingRevision = requestRevision, requestRevision = requestRevision,
            stateGeneration = -1, message = null, messageText = null) }
        runCatching {
            command("SetAudioOutput", JSONObject().put("id", id).put("source_revision", 0)
                .put("revision", requestRevision).put("mode", mode))
        }.onFailure {
            pendingAudioOutputs.remove(requestRevision)
            updateAudioOutput(id) { output -> output.copy(pendingRevision = null,
                localPlayback = "Unknown", message = null, messageText = AppText("Could not request host audio output")) }
        }
    }
    private fun updateAudioOutput(id: Int, update: (WorkspaceAudioOutput) -> WorkspaceAudioOutput) {
        mutable.value = mutable.value.copy(panes = mutable.value.panes.map { pane ->
            if (pane.id == id) pane.copy(audioOutput = update(pane.audioOutput)) else pane
        })
    }
    private fun saveWindowOverride(pane: WorkspacePane) {
        val ctx = context ?: return
        val key = windowOverrideKey(pane.source) ?: return
        val basePrefs = ctx.getSharedPreferences("stream", Context.MODE_PRIVATE)
        val defaultFps = basePrefs.getInt("fps", 60)
        val defaultKbps = basePrefs.getInt("bitrate_kbps", 20_000)
        val winPrefs = ctx.getSharedPreferences("window_stream_overrides", Context.MODE_PRIVATE)
        if (pane.windowScalePercent == 100 && pane.windowLayoutPreset == "original" && pane.fps == defaultFps && pane.kbps == defaultKbps) {
            winPrefs.edit().remove(key).apply()
        } else {
            val json = JSONObject()
                .put("scale", pane.windowScalePercent)
                .put("layout", pane.windowLayoutPreset)
                .put("fps", pane.fps)
                .put("kbps", pane.kbps)
            winPrefs.edit().putString(key, json.toString()).apply()
        }
    }
    fun settings(id: Int, sessionKey: String, generation: Long, width: Int? = null, height: Int? = null, fps: Int? = null, kbps: Int? = null, scalePercent: Int? = null) = guarded {
        if (!currentView(id, sessionKey, generation, false)) return@guarded
        val pane = mutable.value.panes.find { it.id == id } ?: return@guarded
        if (!pane.accepted) return@guarded
        val updated = pane.copy(
            width = width ?: pane.width,
            height = height ?: pane.height,
            fps = fps ?: pane.fps,
            kbps = kbps ?: pane.kbps,
            windowScalePercent = scalePercent ?: pane.windowScalePercent
        )
        appRequire(updated.fps > 0 && updated.kbps > 0, "Frame rate and bitrate must be positive")
        if (pane.source.isWindow && (fps != null || kbps != null || scalePercent != null)) {
            saveWindowOverride(updated)
        }
        if (updated == pane) return@guarded
        action(3, request(updated).toString())
        mutable.value = mutable.value.copy(panes = mutable.value.panes.map { if (it.id == id) updated else it })
    }
    fun resizeWindow(id: Int, sessionKey: String, generation: Long, preset: String, targetWidth: Int? = null, targetHeight: Int? = null) = guarded {
        if (!currentView(id, sessionKey, generation, false)) return@guarded
        val pane = mutable.value.panes.find { it.id == id && it.accepted && it.source.isWindow } ?: return@guarded
        val updated = pane.copy(windowLayoutPreset = preset, windowResizeNotice = null, windowResizeError = false)
        saveWindowOverride(updated)
        val reqId = ++windowResizeSeq
        pendingWindowResizes[reqId] = id to (SystemClock.elapsedRealtime() + 8_500L)
        val targetJson = if (targetWidth != null && targetHeight != null) {
            JSONArray().put(targetWidth).put(targetHeight)
        } else JSONObject.NULL
        mutable.value = mutable.value.copy(panes = mutable.value.panes.map { if (it.id == id) updated else it })
        command("ResizeWindow", JSONObject().put("id", pane.id).put("source_revision", 0)
            .put("request_id", reqId).put("target_size", targetJson))
    }
    fun resetWindowOverride(id: Int, sessionKey: String, generation: Long, srcWidth: Int, srcHeight: Int) = guarded {
        if (!currentView(id, sessionKey, generation, false)) return@guarded
        val pane = mutable.value.panes.find { it.id == id && it.accepted && it.source.isWindow } ?: return@guarded
        val ctx = checkNotNull(context)
        val basePrefs = ctx.getSharedPreferences("stream", Context.MODE_PRIVATE)
        val defaultFps = basePrefs.getInt("fps", 60)
        val defaultKbps = basePrefs.getInt("bitrate_kbps", 20_000)
        val (capW, capH) = scaledEncodeSize(srcWidth, srcHeight, 100)
        val updated = pane.copy(
            width = capW,
            height = capH,
            fps = defaultFps,
            kbps = defaultKbps,
            windowScalePercent = 100,
            windowLayoutPreset = "original",
            windowResizeNotice = null,
            windowResizeError = false
        )
        saveWindowOverride(updated)
        mutable.value = mutable.value.copy(panes = mutable.value.panes.map { if (it.id == id) updated else it })
        action(3, request(updated).toString())
        val reqId = ++windowResizeSeq
        pendingWindowResizes[reqId] = id to (SystemClock.elapsedRealtime() + 8_500L)
        command("ResizeWindow", JSONObject().put("id", pane.id).put("source_revision", 0)
            .put("request_id", reqId).put("target_size", JSONObject.NULL))
    }
    fun viewport(id: Int,sessionKey: String,generation: Long,visible: IntArray,width: Int,height: Int) = guarded {
        if (!currentView(id,sessionKey,generation,false)) return@guarded
        val pane=mutable.value.panes.firstOrNull { it.id==id && it.accepted && !it.source.isCamera } ?: return@guarded
        require(visible.size==4 && width in 16..8192 && height in 16..8192)
        command("Viewport",JSONObject().put("id",pane.id).put("source_revision",0).put("revision",++revision)
            .put("width",width).put("height",height).put("visible",JSONObject().put("x",visible[0]).put("y",visible[1])
                .put("width",visible[2]).put("height",visible[3])))
    }
    fun sendFile(file: File, expectedSessionKey: String) = guarded {
        appCheck(active && mutable.value.connected && mutable.value.sessionKey == expectedSessionKey, "The destination changed or disconnected. Select the file again for the intended device.")
        lastFileAction = SystemClock.elapsedRealtime()
        action(4, file.absolutePath)
    }
    fun publishSharedFile(file: File, expectedSessionKey: String) = guarded {
        appCheck(active && mutable.value.connected && mutable.value.sessionKey == expectedSessionKey, "The selected device session changed")
        action(8, file.absolutePath)
        mutable.value = mutable.value.copy(status = "File shared with this network until cleared, network change, or app exit. Enable file reception on this phone for new incoming connections.", statusText = AppText("File shared with this network until cleared, network change, or app exit. Enable file reception on this phone for new incoming connections."))
    }
    fun clearSharedFiles() = guarded {
        action(10)
        mutable.value = mutable.value.copy(status = "Shared list cleared. Already accepted transfers may finish.", statusText = AppText("Shared list cleared. Already accepted transfers may finish."))
    }
    fun listSharedFiles(afterId: Long = 0L) = guarded {
        appCheck(active && mutable.value.connected && mutable.value.files, "File connection is unavailable")
        appCheck(!mutable.value.sharedBusy, "A shared-file list request is already pending")
        val id = ++shareRequestId
        action(9, JSONObject().put("request_id", id).put("request", JSONObject().put("List", JSONObject().put("after_id", afterId))).toString())
        pendingShareList = id
        lastFileAction = SystemClock.elapsedRealtime()
        mutable.value = mutable.value.copy(sharedBusy = true, status = "Requesting shared files...", statusText = AppText("Requesting shared files..."))
    }
    fun fetchSharedFile(fileId: Long, expectedSessionKey: String) = guarded {
        appCheck(active && mutable.value.connected && mutable.value.sessionKey == expectedSessionKey, "The selected device session changed")
        appCheck(mutable.value.remoteFiles.any { it.id == fileId }, "Refresh the remote shared list first")
        action(9, JSONObject().put("request_id", ++shareRequestId).put("request", JSONObject().put("Fetch", JSONObject().put("file_id", fileId))).toString())
        lastFileAction = SystemClock.elapsedRealtime()
        mutable.value = mutable.value.copy(status = "File requested; waiting for transfer confirmation", statusText = AppText("File requested; waiting for transfer confirmation"))
    }
    fun enableClipboard(enabled: Boolean) = guarded {
        command("SetClipboard", JSONObject().put("enabled", enabled))
        mutable.value = mutable.value.copy(clipboard = enabled)
        if (!enabled) pendingClipboard = null
    }
    fun sendClipboard(value: JSONObject) = guarded { lastFileAction = SystemClock.elapsedRealtime(); action(7, value.toString()) }
    private fun applyClipboard(value: JSONObject) {
        if (!mutable.value.clipboard) return
        if (background) { pendingClipboard = value; return }
        val ctx = checkNotNull(context)
        android.os.Handler(android.os.Looper.getMainLooper()).post {
            if (background || !mutable.value.clipboard) return@post
            runCatching {
                val clipboard = ctx.getSystemService(android.content.ClipboardManager::class.java)
                val clip = when {
                    value.has("text") -> android.content.ClipData.newPlainText("Weavelo", value.getString("text"))
                    value.has("image") -> android.content.ClipData.newUri(ctx.contentResolver, "Weavelo image", androidx.core.content.FileProvider.getUriForFile(ctx, "${ctx.packageName}.files", File(value.getString("image"))))
                    value.has("clipboard_files") -> {
                        val files = value.getJSONArray("clipboard_files")
                        if (files.length() == 0) return@post
                        fun uri(index: Int) = androidx.core.content.FileProvider.getUriForFile(ctx, "${ctx.packageName}.files", File(files.getString(index)))
                        android.content.ClipData.newUri(ctx.contentResolver, "Weavelo files", uri(0)).apply { for (i in 1 until files.length()) addItem(android.content.ClipData.Item(uri(i))) }
                    }
                    else -> return@post
                }
                clipboard.setPrimaryClip(clip)
            }.onFailure { mutable.value = mutable.value.copy(status = it.message ?: "Clipboard unavailable", statusText = appTextOf(it) ?: if (it.message == null) AppText("Clipboard unavailable") else null) }
        }
    }
    fun keyframe(id: Int) = guarded { action(6, id.toString()) }
    private fun currentView(id: Int, sessionKey: String, generation: Long, visible: Boolean): Boolean {
        val s = mutable.value
        return active && s.connected && s.sessionKey == sessionKey && s.connectionGeneration == generation &&
            s.panes.any { it.id == id } && (!visible || (!background && (s.grid || s.selected == id)))
    }
    private fun inputIssue(id: Int, message: String?, text: AppText? = null) {
        val current = mutable.value
        if (current.panes.any { it.id == id && (it.inputError != message || it.inputErrorText != text) }) {
            mutable.value = current.copy(panes = current.panes.map { if (it.id == id) it.copy(inputError = message, inputErrorText = text) else it })
        }
    }
    private fun inputViewIssue(id: Int, sessionKey: String, generation: Long) {
        val current = mutable.value
        if (current.sessionKey != sessionKey || current.connectionGeneration != generation || current.panes.none { it.id == id }) return
        val message = when {
            !active || !current.connected -> "Input couldn't reach this device connection."
            background -> "Input was ignored while the app was in the background."
            !current.grid && current.selected != id -> "Input was ignored while this stream was hidden."
            else -> "Control is not ready yet."
        }
        inputIssue(id, message, AppText(message))
    }
    fun keyboardBinding(id: Int, sessionKey: String, generation: Long, token: Long, focusBinding: JSONObject? = null, focusGeometry: JSONObject? = null): String? = synchronized(this) {
        if (!currentView(id,sessionKey,generation,true)) {
            guarded { inputViewIssue(id,sessionKey,generation) }
            return@synchronized null
        }
        val result = runCatching { action(13,JSONObject().put("id",id).put("token",token).put("focus_binding",focusBinding ?: JSONObject.NULL).put("focus_geometry",focusGeometry ?: JSONObject.NULL).toString()).takeIf { JSONObject(it).has("generation") } }
        if (focusBinding == null) guarded {
            if (currentView(id,sessionKey,generation,false)) {
                val message = if (result.isFailure) result.exceptionOrNull()?.message ?: "Keyboard is unavailable"
                    else if (result.getOrNull() == null) "Control is not ready yet." else null
                val local = if (result.isFailure && result.exceptionOrNull()?.message != null) null else message?.let { AppText(it) }
                inputIssue(id, message, local)
            }
        }
        result.getOrNull()
    }
    fun keyboard(id: Int, sessionKey: String, generation: Long, token: Long, binding: String,
        text: String? = null, key: String? = null, pressed: Boolean = false, eventTime: Long = SystemClock.uptimeMillis()) = guarded {
        if (!currentView(id,sessionKey,generation,true) || SystemClock.uptimeMillis()-eventTime !in 0L..250L) return@guarded
        val event=JSONObject().put("id",id).put("token",token).put("binding",JSONObject(binding)).put("age_ms",SystemClock.uptimeMillis()-eventTime)
        if (text!=null) event.put("text",text) else event.put("key",key).put("pressed",pressed)
        runCatching { action(14,event.toString()) }.onFailure {
            // A rejected commit can invalidate its editor before queued IME
            // callbacks arrive. Keep the original, actionable host reason.
            if (mutable.value.panes.firstOrNull { pane -> pane.id == id }?.inputError == null)
                inputIssue(id, it.message ?: "Keyboard is unavailable", if (it.message == null) AppText("Keyboard is unavailable") else null)
        }
    }
    internal fun keyboardAlive(id: Int, sessionKey: String, generation: Long, token: Long, binding: String,
        eventTime: Long = SystemClock.uptimeMillis()) = guarded {
        if (!currentView(id, sessionKey, generation, true) ||
            mutable.value.panes.none { it.id == id && it.input && it.accepted }) return@guarded
        val age = SystemClock.uptimeMillis() - eventTime
        if (age !in 0L..250L) {
            inputIssue(id, "Keyboard shortcuts expired. Reopen the keyboard and select them again.", AppText("Keyboard shortcuts expired. Reopen the keyboard and select them again."))
            return@guarded
        }
        val event = JSONObject().put("id", id).put("token", token).put("binding", JSONObject(binding))
            .put("age_ms", age).put("alive", true)
        runCatching { action(14, event.toString()) }.onFailure {
            inputIssue(id, "Keyboard shortcuts expired. Reopen the keyboard and select them again.", AppText("Keyboard shortcuts expired. Reopen the keyboard and select them again."))
        }
    }
    internal fun navigation(id: Int, sessionKey: String, generation: Long, token: Long, binding: String,
        navigation: WorkspaceNavigation, eventTime: Long = SystemClock.uptimeMillis()) = guarded {
        if (!currentView(id, sessionKey, generation, true) || SystemClock.uptimeMillis() - eventTime !in 0L..250L) return@guarded
        val state = mutable.value
        val pane = state.panes.firstOrNull { it.id == id } ?: return@guarded
        val mainDisplay = pane.source.source == "MainDisplay"
        val supportedSource = when (navigation) {
            WorkspaceNavigation.BACK -> mainDisplay || pane.source.isWindow
            WorkspaceNavigation.HOME -> mainDisplay
        }
        if (!state.androidHost || !pane.accepted || !pane.input || pane.source.isCamera || !supportedSource) {
            inputIssue(id, "Remote navigation is unavailable for this source", AppText("Remote navigation is unavailable for this source"))
            return@guarded
        }
        val event = JSONObject().put("id", id).put("token", token).put("binding", JSONObject(binding))
            .put("age_ms", SystemClock.uptimeMillis() - eventTime).put("navigation", navigation.wireValue)
        runCatching { action(14, event.toString()) }.onFailure {
            if (mutable.value.panes.firstOrNull { pane -> pane.id == id }?.inputError == null)
                inputIssue(id, it.message ?: "Remote navigation is unavailable", if (it.message == null) AppText("Remote navigation is unavailable") else null)
        }
    }
    fun surface(id: Int, sessionKey: String, generation: Long, token: Long, phase: String, eventTime: Long = SystemClock.uptimeMillis()) = guarded {
        if (!currentView(id, sessionKey, generation, phase == "begin" || phase == "ready" || phase == "alive")) return@guarded
        val currentPhase = if (phase == "alive" && SystemClock.uptimeMillis() - eventTime !in 0L..250L) "cancel" else phase
        action(11, JSONObject().put("id", id).put("token", token).put("phase", currentPhase).toString())
        if (currentPhase == "begin" || currentPhase == "ready") inputIssue(id, null)
        if (currentPhase == "ready") ownership.request()?.incomingId?.let {
            DirectedDelivery.rendered(it, generation, id, mutable.value.panes)
        }
    }
    internal fun presented(id: Int, sessionKey: String, generation: Long, token: Long, geometry: VideoFrameGeometry) = guarded {
        if (currentView(id, sessionKey, generation, true)) {
            action(15, JSONObject().put("id", id).put("token", token).put("geometry", geometry.toJson()).toString())
        }
    }
    internal fun touch(id: Int, sessionKey: String, generation: Long, token: Long, eventTime: Long,
        mode: TouchMode, width: Int, height: Int, event: JSONObject,
        geometry: VideoFrameGeometry? = null,
        contacts: JSONArray? = null,
        onProbe: (String, JSONObject) -> Unit = { _, _ -> },
        onCursor: (Float, Float) -> Unit = { _, _ -> }) = guarded {
        if (!currentView(id, sessionKey, generation, true)) {
            inputViewIssue(id,sessionKey,generation)
            return@guarded
        }
        // Include time waiting on this serial executor; do not replay an old Android event as fresh input.
        if (SystemClock.uptimeMillis() - eventTime !in 0L..250L) {
            action(11, JSONObject().put("id", id).put("token", token).put("phase", "cancel").toString())
            if (event.optString("action") == "Down") inputIssue(id, "Input arrived too late. Try again.", AppText("Input arrived too late. Try again."))
            return@guarded
        }
        val presented = geometry?.let { g -> JSONObject().put("source_revision",Integer.toUnsignedLong(g.sourceRevision))
            .put("demand_revision",java.math.BigInteger(java.lang.Long.toUnsignedString(g.demandRevision)))
            .put("revision",java.math.BigInteger(java.lang.Long.toUnsignedString(g.revision)))
            .put("source_width",g.sourceWidth).put("source_height",g.sourceHeight).put("width",g.width).put("height",g.height)
            .put("crop",JSONObject().put("x",g.x).put("y",g.y).put("width",g.cropWidth).put("height",g.cropHeight)) }
        runCatching { action(12, JSONObject().put("id", id).put("token", token)
            .put("mode", mode.code).put("width", width).put("height", height)
            .put("natural_scroll", WeaveloClient.naturalScroll)
            .put("geometry",presented ?: JSONObject.NULL)
            .put("event", if(contacts==null) JSONObject().put("Touch",event) else JSONObject().put("TouchFrame",contacts)).toString()) }
            .onFailure { inputIssue(id, it.message ?: "Control is unavailable", if (it.message == null) AppText("Control is unavailable") else null) }
            .onSuccess { result ->
                if (event.optString("action") == "Down") inputIssue(id, null)
                if (result != "null") {
                    val response = JSONObject(result)
                    if (!response.isNull("focus_sequence") && presented != null && currentView(id, sessionKey, generation, true))
                        onProbe(response.getString("focus_sequence"), presented)
                    val cursor = response.optJSONArray("cursor") ?: return@onSuccess
                    // Compose snapshot state accepts this serialized update;
                    // no extra UI queue may accumulate old pointer positions.
                    if (currentView(id, sessionKey, generation, true))
                        onCursor(cursor.getDouble(0).toFloat(), cursor.getDouble(1).toFloat())
                }
            }
    }
    private fun endIncoming(reason: String, rejected: Boolean = false, reasonText: AppText? = null) {
        ownership.endIncoming(mutable.value.sessionKey)
        incomingEnded = true
        // A lost unsubscribe acknowledgment must not leave a completed offer
        // visible/busy forever. The retained connection closes after files drain.
        mutable.value.panes.forEach { pane -> runCatching { command("Unsubscribe", JSONObject().put("id", pane.id)) } }
        mutable.value = mutable.value.copy(panes = emptyList(), sources = emptyList(), androidHost = false, incomingEnded = true, status = reason, statusText = reasonText)
        ownership.request()?.incomingId?.let { DirectedDelivery.finish(it, reason, rejected) }
    }
    private fun retireEndedIncoming() {
        if (!incomingEnded || !ownership.retireEndedIncoming(transfersActive ||
                SystemClock.elapsedRealtime() - lastFileAction < 5_000)) return
        closeNative()
        applicationIcons.clear()
        mutable.value = mutable.value.copy(connected = false, androidHost = false, files = false, clipboard = false,
            clipboardAvailable = false, sharedBusy = false, remoteFiles = emptyList(), sharedNext = null)
    }
    private fun drainEvents() {
        pendingAudioOutputs.entries.toList().forEach { (requestRevision, pending) ->
            if (mutable.value.panes.none { it.id == pending.pane }) pendingAudioOutputs.remove(requestRevision)
            else if (pending.deadline < SystemClock.elapsedRealtime()) {
                pendingAudioOutputs.remove(requestRevision)
                updateAudioOutput(pending.pane) { it.copy(pendingRevision = null,
                    localPlayback = "Unknown", message = null, messageText = AppText("Host audio request timed out; state unknown")) }
            }
        }
        pendingWindowResizes.entries.removeAll { (_, pending) ->
            val (paneId, deadline) = pending
            if (deadline < SystemClock.elapsedRealtime()) {
                mutable.value = mutable.value.copy(panes = mutable.value.panes.map { p ->
                    if (p.id == paneId) p.copy(windowResizeNotice = "Host window resize timed out", windowResizeError = true) else p
                })
                true
            } else mutable.value.panes.none { it.id == paneId }
        }
        cameraSwitches.entries.removeAll { (_, pending) ->
            if (pending.deadline < SystemClock.elapsedRealtime()) {
                mutable.value = mutable.value.copy(status = "Camera switch timed out; retry", statusText = AppText("Camera switch timed out; retry"))
                true
            } else mutable.value.panes.none { it.id == pending.pane }
        }
        cameraReplacements.entries.filter { it.value.second<SystemClock.elapsedRealtime() }.toList().forEach { (old,waiting)->
            cameraReplacements.remove(old)
            removePane(waiting.first,"Camera did not finish closing; retry this source",rejected=true, reasonText = AppText("Camera did not finish closing; retry this source"))
        }
        val events = JSONArray(action(5))
        for (i in 0 until events.length()) {
            val event = events.getJSONObject(i)
            if (event.has("transfers_active")) {
                transfersActive = event.getBoolean("transfers_active")
                mutable.value = mutable.value.copy(transferring = transfersActive)
            }
            event.optJSONArray("telemetry")?.let { samples ->
                val byId = (0 until samples.length()).map { samples.getJSONObject(it) }.associateBy { it.getInt("id") }
                val now = SystemClock.elapsedRealtime()
                mutable.value = mutable.value.copy(panes = mutable.value.panes.map { pane ->
                    val sample = byId[pane.id]
                    pane.copy(telemetry = if (sample == null) TelemetrySnapshot() else TelemetrySnapshot(
                        networkRttMs = nullableTelemetryNumber(sample.opt("network_rtt_ms")),
                        packetLossPercent = nullableTelemetryNumber(sample.opt("packet_loss_percent"))?.toFloat() ?: Float.NaN,
                        videoBitrateKbps = if (sample.isNull("video_bitrate_kbps")) -1 else sample.optInt("video_bitrate_kbps", -1),
                        observedAtElapsedMs = now))
                })
            }
            val control = event.optJSONObject("control")
            control?.optJSONObject("AudioOutputState")?.let { reply ->
                val request = reply.getJSONObject("request")
                val requestRevision = request.getLong("revision")
                val pane = mutable.value.panes.firstOrNull { it.id == request.getInt("id") && it.accepted }
                val resultGeneration = reply.getLong("generation")
                if (pane != null && pane.audioOutput.requestRevision == requestRevision && request.getInt("source_revision") == 0 &&
                    resultGeneration > pane.audioOutput.stateGeneration) {
                    val status = reply.getJSONObject("status")
                    pendingAudioOutputs.remove(requestRevision)
                    updateAudioOutput(pane.id) { it.copy(pendingRevision = null, stateGeneration = resultGeneration,
                        localPlayback = status.getString("local_playback"),
                        privateSupported = status.getBoolean("private_supported"),
                        message = if (status.isNull("message")) null else status.getString("message"), messageText = null) }
                }
            }
            control?.optJSONObject("CameraSwitchResult")?.let { reply ->
                val request = reply.getJSONObject("request")
                cameraSwitches.remove(request.getLong("request_id"))?.let { pending ->
                    val pane = mutable.value.panes.firstOrNull { it.id == pending.pane }
                    val incomingId = ownership.request()?.incomingId
                    val expectedDeliveryRequest = incomingId?.let { DirectedDelivery.requestId(it) }
                    val deliveryMatches = if (incomingId == null) request.isNull("delivery_request_id")
                    else expectedDeliveryRequest != null && !request.isNull("delivery_request_id") && request.getString("delivery_request_id") == expectedDeliveryRequest
                    if (pane != null && request.getInt("id") == pane.id && request.getInt("source_revision") == 0 &&
                        deliveryMatches && request.get("from").toString() == pane.source.source.toString() &&
                        request.get("target").toString() == pending.source.source.toString()) {
                        val ready = reply.getJSONObject("ready")
                        if (ready.has("Ok")) {
                            if (incomingId != null) DirectedDelivery.replaceSource(incomingId, pane.source.source.toString(), pending.source.source)
                            replaceCameraView(pane, pending.source)
                        } else mutable.value = mutable.value.copy(status = ready.optString("Err", "Camera switch was rejected"), statusText = if (ready.has("Err")) null else AppText("Camera switch was rejected"))
                    }
                }
            }
            control?.optJSONObject("WindowResized")?.let { reply ->
                val reqId = reply.getInt("request_id")
                val paneId = reply.getInt("id")
                if (pendingWindowResizes.remove(reqId)?.first == paneId && reply.getInt("source_revision") == 0) {
                    val applied = reply.getJSONObject("applied_size")
                    val okArr = applied.optJSONArray("Ok")
                    val isErr = okArr == null
                    val notice = if (okArr != null && okArr.length() == 2) {
                        "${okArr.getInt(0)} × ${okArr.getInt(1)}"
                    } else {
                        applied.optString("Err", "Window resize unavailable")
                    }
                    mutable.value = mutable.value.copy(panes = mutable.value.panes.map { p ->
                        if (p.id == paneId) {
                            p.copy(
                                windowResizeNotice = if (p.windowLayoutPreset == "original" && !isErr) null else notice,
                                windowResizeError = isErr
                            )
                        } else p
                    })
                }
            }
            control?.optJSONObject("EditorFocus")?.let { reply ->
                val binding = reply.getJSONObject("binding")
                val id = binding.getInt("subscription_id")
                if (reply.optBoolean("editable") && currentView(id, mutable.value.sessionKey, mutable.value.connectionGeneration, true))
                    focusEvents.tryEmit(EditorFocus(id, reply.getString("sequence"), binding))
            }
            control?.optJSONObject("Opened")?.let { mutable.value = mutable.value.copy(connectionId = it.getLong("connection_id"), maxSubscriptions = it.getInt("max_subscriptions"), files = it.getBoolean("files"), clipboardAvailable = it.getBoolean("clipboard")) }
            control?.optJSONObject("Closed")?.let { action(1); asleep = true; retainingConnection = false; mutable.value = mutable.value.copy(connected = false, androidHost = false, status = it.getString("reason"), statusText = null) }
            control?.optJSONObject("TextRejected")?.let {
                val id = it.getJSONObject("binding").getInt("subscription_id")
                if (mutable.value.panes.firstOrNull { pane -> pane.id == id }?.inputError == null)
                    inputIssue(id, it.getString("reason"))
            }
            control?.optJSONObject("ClipboardState")?.let { mutable.value = mutable.value.copy(clipboard = it.getBoolean("enabled")) }
            event.optJSONObject("clipboard")?.let { applyClipboard(it) }
            event.optJSONObject("detail")?.takeIf { it.has("clipboard_files") }?.let { applyClipboard(it) }
            control?.optJSONObject("SourceAppearance")?.let { response ->
                val id = response.getInt("id")
                val source = response.get("source").toString()
                response.getJSONObject("appearance").optJSONObject("Ok")?.let { value ->
                    val appearance = DeviceArtwork.decode(value)
                    mutable.value = mutable.value.copy(panes = mutable.value.panes.map { pane ->
                        if (pane.id == id && pane.source.source.toString() == source) pane.copy(appearance = appearance) else pane
                    })
                }
            }
            control?.optJSONObject("Sources")?.let { response ->
                val assets = response.getJSONArray("icons")
                for (index in 0 until assets.length()) {
                    val asset = assets.getJSONObject(index)
                    val id = asset.getJSONArray("id").toString()
                    if (applicationIcons.containsKey(id)) continue
                    val png = asset.getJSONArray("png")
                    // The native connection has already authenticated, hashed and decoded these bounds.
                    if (png.length() !in 1..16384) continue
                    val bytes = ByteArray(png.length()) { png.getInt(it).toByte() }
                    val bitmap = android.graphics.BitmapFactory.decodeByteArray(bytes, 0, bytes.size) ?: continue
                    if (bitmap.width !in 1..64 || bitmap.height !in 1..64) { bitmap.recycle(); continue }
                    applicationIcons[id] = bitmap
                }
                val bindings = response.getJSONArray("source_icons")
                val artwork = HashMap<String, android.graphics.Bitmap?>()
                val referenced = HashSet<String>()
                for (index in 0 until bindings.length()) {
                    val binding = bindings.getJSONObject(index)
                    val id = binding.getJSONArray("icon_id").toString()
                    referenced.add(id)
                    artwork[binding.get("source").toString()] = applicationIcons[id]
                }
                applicationIcons.keys.retainAll(referenced)
                val sources = response.getJSONArray("sources")
                val directory = (0 until sources.length()).map { index ->
                    val source = sources.getJSONObject(index)
                    WorkspaceSource(source.get("source"), source.getString("title"), source.getInt("width"), source.getInt("height"),
                        source.getString("application"), artwork[source.get("source").toString()], source.optJSONObject("camera"))
                }
                val androidHost = directory.any { isAndroidProjectionSource(it.source.toString(), it.application) }
                val incoming = ownership.request()?.incomingId
                val expected = incoming?.let { DirectedDelivery.sources(it) }
                val initialVisible = if (expected == null) directory else directory.filter { it.source.toString() in expected }
                val switchGroups = initialVisible.mapNotNull { it.camera?.optLong("switch_group")?.takeIf { g -> g != 0L } }.toSet()
                val visible = if (expected == null) directory else directory.filter {
                    it.source.toString() in expected || (it.isCamera && (it.camera?.optLong("switch_group") ?: 0L) in switchGroups)
                }
                val rotated=mutable.value.panes.mapNotNull { pane ->
                    directory.firstOrNull { it.source.toString()==pane.source.source.toString() }
                        ?.takeIf { pane.accepted && it.isCamera && it.rotation!=pane.source.rotation }?.let { pane to it }
                }
                mutable.value = mutable.value.copy(sources = visible, sourcesLoaded = true, androidHost = androidHost, panes = mutable.value.panes.map { pane ->
                    directory.firstOrNull { it.source.toString() == pane.source.source.toString() }?.let { pane.copy(source = it) } ?: pane
                })
                rotated.forEach { (pane,source)->replaceCameraView(pane,source) }
                val mainDisplay = primaryScreenSource(visible)
                if (initialSource.takeMainDisplay(mutable.value.sessionKey, mainDisplay != null)) {
                    addNative(checkNotNull(mainDisplay))
                }
                if (incoming != null && !incomingInitialized && !incomingEnded) {
                    incomingInitialized = true
                    if (expected.isNullOrEmpty() || expected.size > minOf(4, mutable.value.maxSubscriptions) || initialVisible.size != expected.size) {
                        endIncoming("An offered source is no longer available", rejected = true, reasonText = AppText("An offered source is no longer available"))
                    } else initialVisible.forEach { addNative(it) }
                }
            }
            control?.optJSONObject("Unsubscribed")?.let { ended ->
                val id=ended.getInt("id")
                val next=cameraReplacements.remove(id)?.first
                if(next!=null) mutable.value.panes.firstOrNull { it.id==next }?.let { command("Subscribe",request(it)) }
                else removePane(id, "The source stopped sharing", reasonText = AppText("The source stopped sharing"))
            }
            control?.optJSONObject("Subscribed")?.let { subscribed ->
                val subId = subscribed.getInt("id")
                mutable.value = mutable.value.copy(panes = mutable.value.panes.map {
                    if (it.id == subId) it.copy(audioOwner = if (subscribed.isNull("audio_owner")) null else subscribed.getInt("audio_owner"), input = subscribed.getBoolean("supports_input"),
                        maxTouchPoints = subscribed.getInt("max_touch_points").coerceIn(0,10), accepted = true, videoError = null, videoErrorText = null) else it
                })
                mutable.value.panes.firstOrNull { it.id == subId && it.source.isWindow && it.windowLayoutPreset != "original" }?.let { winPane ->
                    presetWindowTargetSize(winPane.windowLayoutPreset)?.let { (tw, th) ->
                        val reqId = ++windowResizeSeq
                        pendingWindowResizes[reqId] = subId to (SystemClock.elapsedRealtime() + 8_500L)
                        runCatching {
                            command("ResizeWindow", JSONObject().put("id", subId).put("source_revision", 0)
                                .put("request_id", reqId).put("target_size", JSONArray().put(tw).put(th)))
                        }
                    }
                }
                activity()
            }
            control?.optJSONObject("Error")?.let {
                val id = it.getInt("request_id")
                if (ownership.request()?.incomingId != null && mutable.value.panes.any { pane -> pane.id == id && !pane.accepted }) {
                    command("Unsubscribe", JSONObject().put("id", id))
                    mutable.value = mutable.value.copy(status = it.getString("reason"), statusText = null)
                    removePane(id, it.getString("reason"), rejected = true)
                } else if (mutable.value.panes.any { pane -> pane.id == id && !pane.accepted }) {
                    mutable.value = mutable.value.copy(panes = mutable.value.panes.map { pane ->
                        if (pane.id == id) pane.copy(videoError = it.getString("reason"), videoErrorText = null, input = false) else pane
                    })
                }
                android.util.Log.w("WeaveloWorkspace", "Request $id failed: ${it.getString("reason")}")
                // A late settings/unsubscribe reply for a closed pane must not
                // replace the status of the current screen or its keyboard.
                if (it.getString("reason").startsWith("INPUT_UNAVAILABLE:") && mutable.value.panes.any { pane -> pane.id == id && pane.accepted })
                    inputIssue(id, it.getString("reason"))
                else if (id in 0..1 || mutable.value.panes.any { pane -> pane.id == id && pane.accepted })
                    mutable.value = mutable.value.copy(status = it.getString("reason"), statusText = null)
            }
            if (event.has("file")) mutable.value = mutable.value.copy(status = event.getString("file").take(220), statusText = null)
            event.optJSONObject("detail")?.let { detail ->
                val response = detail.optJSONObject("shared_response")
                val id = detail.optLong("shared_request_id")
                response?.optJSONObject("Page")?.takeIf { id == pendingShareList }?.let { page ->
                    val entries = page.getJSONArray("entries")
                    val files = (0 until entries.length()).map { index ->
                        val item = entries.getJSONObject(index)
                        RemoteSharedFile(item.getLong("id"), item.getString("name"), item.getLong("size_bytes"))
                    }
                    pendingShareList = 0L
                    mutable.value = mutable.value.copy(remoteFiles = files, sharedNext = if (page.isNull("next_after_id")) null else page.getLong("next_after_id"), sharedBusy = false, status = if (files.isEmpty()) "No files explicitly shared by this device" else "Choose a shared file to retrieve", statusText = AppText(if (files.isEmpty()) "No files explicitly shared by this device" else "Choose a shared file to retrieve"))
                }
                response?.optJSONObject("Queued")?.let { mutable.value = mutable.value.copy(status = "Request accepted; delivery is not complete yet", statusText = AppText("Request accepted; delivery is not complete yet")) }
                response?.optJSONObject("Rejected")?.let { rejected ->
                    if (id == pendingShareList) pendingShareList = 0L
                    mutable.value = mutable.value.copy(sharedBusy = pendingShareList != 0L, status = rejected.getString("message"), statusText = null)
                }
            }
        }
    }
}
