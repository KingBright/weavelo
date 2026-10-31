package `fun`.hackerlife.weavelo.ui

import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.Image
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import `fun`.hackerlife.weavelo.*
import kotlinx.coroutines.*
import org.json.JSONObject
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicBoolean

@Composable
fun WorkspaceScreen(endpoint: String, sendFileOnConnect: Boolean = false, deviceId: String = "",
    incoming: WorkspaceSessionRequest? = null, onClose: () -> Unit) {
    val context = LocalContext.current
    var sourcePicker by remember { mutableStateOf(false) }
    var actionsOpen by remember { mutableStateOf(false) }
    var pickerGeneration by remember { mutableStateOf(0L) }
    val sessionKey = incoming?.sessionKey ?: rememberSaveable(endpoint, deviceId) { WorkspaceClient.sessionKeyFor(endpoint, deviceId) }
    var requestedFilePicker by rememberSaveable(endpoint, deviceId) { mutableStateOf(false) }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val serviceState by WorkspaceClient.state.collectAsState()
    val state = serviceState.takeIf { it.sessionKey == sessionKey }
        ?: WorkspaceState(sessionKey = sessionKey, status = "Connecting…")
    val devices by WeaveloClient.devices.collectAsState()
    val remoteDevice = devices.firstOrNull { it.id == deviceId }
    var foreground by remember { mutableStateOf(lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) }
    var localError by remember { mutableStateOf<String?>(null) }
    var receivedFiles by remember { mutableStateOf(false) }
    var backgroundSettings by remember { mutableStateOf(false) }
    var shareSelection by rememberSaveable(endpoint, deviceId) { mutableStateOf(false) }
    var pickerSessionKey by rememberSaveable(endpoint, deviceId) { mutableStateOf("") }
    var sharedFilesOpen by remember { mutableStateOf(false) }
    val preferences = remember { context.getSharedPreferences("stream", android.content.Context.MODE_PRIVATE) }
    var keepAlive by remember { mutableStateOf(preferences.getInt("background_keepalive_seconds", 300).toString()) }
    var adaptive by remember { mutableStateOf(preferences.getBoolean("adaptive_background", true)) }
    var keepService by remember { mutableStateOf(WeaveloClient.backgroundServiceEnabled()) }
    val scope = rememberCoroutineScope()
    var selectedSettings by remember { mutableStateOf<WorkspacePane?>(null) }
    val volumes = remember { mutableStateMapOf<Int, Float>() }
    if (sourcePicker) {
        if (state.connectionGeneration != pickerGeneration || state.sessionKey != sessionKey || !state.connected) {
            LaunchedEffect(Unit) { sourcePicker = false }
        } else SourcePicker(state.sources, onClose = { sourcePicker = false }) { source ->
            WorkspaceClient.add(source, sessionKey, pickerGeneration)
            sourcePicker = false
        }
    }
    if (backgroundSettings) AlertDialog(onDismissRequest = { backgroundSettings = false }, title = { Text("Background connection") },
        text = { Column {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(keepService, { keepService = it }); Text("Keep this connection in background until Disconnect")
            }
            Text("This is an explicit foreground service, not a battery or system-stop exemption. The idle timeout below applies when this is off.")
            OutlinedTextField(keepAlive, { keepAlive = it }, label = { Text("Idle seconds (0 = always keep)") },
                isError = keepAlive.toIntOrNull()?.let { it !in 0..86400 } ?: true)
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(adaptive, { adaptive = it }); Text("Adjust for battery and mobile data")
            }
            Text("Media pauses immediately. Idle connections sleep sooner on battery saver (30s), low battery (60s), or metered data (120s). File transfers finish first. Reopen to reconnect.")
        } }, confirmButton = { TextButton(onClick = {
            val seconds = keepAlive.toIntOrNull()
            if (seconds != null && seconds in 0..86400) {
                WeaveloClient.updateBackgroundKeepAlive(seconds)
                WeaveloClient.updateBackgroundServiceEnabled(keepService)
                preferences.edit().putBoolean("adaptive_background", adaptive).apply()
                backgroundSettings = false
            }
        }) { Text("Save") } })
    DisposableEffect(endpoint, deviceId, sessionKey) {
        SessionConnectionService.openWorkspace(context, incoming ?: WorkspaceSessionRequest(endpoint, deviceId, sessionKey))
        val observer = LifecycleEventObserver { _, _ -> foreground = lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED) }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    val filePicker = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        val publishSelection = shareSelection
        val selectedSessionKey = pickerSessionKey
        if (uri != null) scope.launch(Dispatchers.IO) {
            runCatching {
                val name = context.contentResolver.query(uri, arrayOf(android.provider.OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
                    if (it.moveToFirst()) it.getString(0) else null
                }?.substringAfterLast('/')?.substringAfterLast('\\')?.takeIf { it != "." && it != ".." && it.isNotBlank() } ?: "shared-file"
                val directory = File(context.cacheDir, "outgoing/${java.util.UUID.randomUUID()}").apply { mkdirs() }
                val file = File(directory, name)
                checkNotNull(context.contentResolver.openInputStream(uri)).use { input -> file.outputStream().use { output -> input.copyTo(output, 64 * 1024) } }
                if (publishSelection) WorkspaceClient.publishSharedFile(file, selectedSessionKey) else WorkspaceClient.sendFile(file, selectedSessionKey)
            }.onFailure { withContext(Dispatchers.Main) { localError = it.message } }
        }
    }
    LaunchedEffect(sessionKey, foreground, state.connected, state.incomingEnded, remoteDevice?.endpoint, remoteDevice?.canOpenWorkspace) {
        val requested = WorkspaceClient.requestedSession()
        val current = WorkspaceClient.state.value
        if (foreground && remoteDevice?.canOpenWorkspace == true &&
            requested?.sessionKey == sessionKey && requested.deviceId == deviceId &&
            current.sessionKey == sessionKey && !current.connected && !current.incomingEnded && !WorkspaceClient.starting) {
            WorkspaceClient.reconnect()
        }
    }
    LaunchedEffect(state.sessionKey, state.connected, state.files, sendFileOnConnect) {
        // Wait for this connection's capability response, not a previous device's state.
        if (sendFileOnConnect && !requestedFilePicker && state.sessionKey == sessionKey && state.connected && state.files) {
            requestedFilePicker = true
            pickerSessionKey = sessionKey
            shareSelection = false
            filePicker.launch(arrayOf("*/*"))
        }
    }
    WeaveloTheme {
        Column(Modifier.fillMaxSize().background(Color(0xff101318)).safeDrawingPadding().padding(8.dp)) {
            Row(Modifier.fillMaxWidth().heightIn(min = 56.dp), verticalAlignment = Alignment.CenterVertically) {
                MobileAction(MobileSymbol.Back, "Disconnect", onClick = {
                    SessionConnectionService.closeWorkspace(context, sessionKey); onClose()
                })
                Column(Modifier.weight(1f).padding(horizontal = 4.dp)) {
                    Text(remoteDevice?.name ?: "Remote device",
                        color = ColorTextPrimary, fontWeight = androidx.compose.ui.text.font.FontWeight.SemiBold,
                        maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
                    Text(if (state.incomingEnded) "Sharing ended" else if (state.connected) "Connected" else "Connecting…", color = ColorTextSecondary,
                        style = MaterialTheme.typography.labelSmall)
                }
                MobileAction(MobileSymbol.Sources, "Choose screen or app window", state.connected && !state.incomingEnded) {
                    WorkspaceClient.refreshSources(sessionKey, state.connectionGeneration)
                    pickerGeneration = state.connectionGeneration; sourcePicker = true
                }
                MobileAction(MobileSymbol.Refresh, "Refresh screens and windows", state.connected && !state.incomingEnded) {
                    WorkspaceClient.refreshSources(sessionKey, state.connectionGeneration)
                }
                Box {
                    MobileAction(MobileSymbol.More, "Connection actions") { actionsOpen = true }
                    DropdownMenu(expanded = actionsOpen, onDismissRequest = { actionsOpen = false }) {
                if (!state.connected && !state.incomingEnded) TextButton(onClick = { actionsOpen = false;  WorkspaceClient.reconnect() }) { Text("Reconnect") }
                TextButton(onClick = { actionsOpen = false;  WorkspaceClient.layout(!state.grid) }) { Text(if (state.grid) "Use tabs" else "Show together") }
                TextButton(onClick = { actionsOpen = false;  pickerSessionKey = sessionKey; shareSelection = false; filePicker.launch(arrayOf("*/*")) }, enabled = state.connected && state.files) { Text("Send file") }
                TextButton(onClick = { actionsOpen = false;  pickerSessionKey = sessionKey; shareSelection = true; filePicker.launch(arrayOf("*/*")) }, enabled = state.connected) { Text("Share file with network") }
                TextButton(onClick = { actionsOpen = false;  WorkspaceClient.clearSharedFiles() }, enabled = state.connected) { Text("Clear my shared list") }
                TextButton(onClick = { actionsOpen = false;  sharedFilesOpen = true; WorkspaceClient.listSharedFiles() }, enabled = state.connected && state.files && !state.sharedBusy) { Text("Retrieve shared files") }
                TextButton(onClick = { actionsOpen = false;  receivedFiles = true }) { Text("Received files") }
                TextButton(onClick = { actionsOpen = false;  backgroundSettings = true }) { Text("Background") }
                TextButton(onClick = { actionsOpen = false;  WorkspaceClient.enableClipboard(!state.clipboard) }, enabled = state.connected && state.clipboardAvailable) { Text(if (state.clipboard) "Clipboard ✓" else "Clipboard") }
                TextButton(onClick = { actionsOpen = false;
                    // Android only allows reading the clipboard while this app has focus.
                    val clip = context.getSystemService(android.content.ClipboardManager::class.java).primaryClip
                    if (clip != null) scope.launch(Dispatchers.IO) {
                        runCatching {
                            val uris = (0 until clip.itemCount).mapNotNull { clip.getItemAt(it).uri }
                            require(uris.size <= 16) { "Send at most 16 clipboard files at a time" }
                            if (uris.isEmpty()) {
                                val text = clip.getItemAt(0).text?.toString() ?: error("Clipboard has no transferable content")
                                require(text.toByteArray().size <= 1024 * 1024) { "Clipboard text is too large" }
                                WorkspaceClient.sendClipboard(JSONObject().put("text", text))
                            } else {
                                val directory = File(context.cacheDir, "outgoing/${java.util.UUID.randomUUID()}").apply { mkdirs() }
                                val files = uris.take(16).mapIndexed { index, uri ->
                                    val name = context.contentResolver.query(uri, arrayOf(android.provider.OpenableColumns.DISPLAY_NAME), null, null, null)?.use {
                                        if (it.moveToFirst()) it.getString(0) else null
                                    }?.substringAfterLast('/')?.substringAfterLast('\\')?.replace(':', '_')
                                        ?.takeIf { it != "." && it != ".." && it.isNotBlank() } ?: "item-$index"
                                    val folder = File(directory, index.toString()).apply { mkdirs() }
                                    val file = File(folder, name)
                                    checkNotNull(context.contentResolver.openInputStream(uri)).use { input -> file.outputStream().use { input.copyTo(it, 64 * 1024) } }
                                    file
                                }
                                if (uris.size == 1 && context.contentResolver.getType(uris[0])?.startsWith("image/") == true) {
                                    val bounds = android.graphics.BitmapFactory.Options().apply { inJustDecodeBounds = true }
                                    android.graphics.BitmapFactory.decodeFile(files[0].absolutePath, bounds)
                                    require(bounds.outWidth > 0 && bounds.outHeight > 0 && bounds.outWidth.toLong() * bounds.outHeight <= 16_777_216) { "Clipboard image is too large or unsupported" }
                                    val bitmap = checkNotNull(android.graphics.BitmapFactory.decodeFile(files[0].absolutePath))
                                    val png = File(directory, "clipboard.png")
                                    try { png.outputStream().use { check(bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it)) } } finally { bitmap.recycle() }
                                    WorkspaceClient.sendClipboard(JSONObject().put("image", png.absolutePath).put("mime", "image/png"))
                                } else WorkspaceClient.sendClipboard(JSONObject().put("files", org.json.JSONArray(files.map { it.absolutePath })))
                            }
                        }.onFailure { withContext(Dispatchers.Main) { localError = it.message } }
                    }
                }, enabled = state.connected && state.clipboard) { Text("Send clipboard") }
                    }
                }
            }
            (localError ?: state.status.takeUnless { it == "Connected" })?.takeIf { it.isNotBlank() }?.let {
                Text(it, color = Color.LightGray, maxLines = 2)
            }
            if (!state.grid && state.panes.size > 1) Row(Modifier.horizontalScroll(rememberScrollState())) {
                state.panes.forEach { pane -> TextButton(onClick = { WorkspaceClient.layout(false, pane.id) }) { SourceIcon(pane.source); Text(pane.source.title) } }
            }
            val visible = if (state.grid) state.panes else state.panes.filter { it.id == state.selected }
            if (visible.isNotEmpty()) BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
                val columns = workspaceGridColumns(maxWidth.value, maxHeight.value,
                    visible.map { it.source.width.toFloat() / it.source.height.coerceAtLeast(1) })
                val rows = visible.chunked(columns)
                Column(Modifier.fillMaxSize()) {
                    rows.forEach { row ->
                        Row(Modifier.weight(1f).fillMaxWidth()) {
                            row.forEach { pane ->
                                key(pane.id, state.connectionGeneration) {
                                    Column(Modifier.weight(1f).fillMaxHeight().padding(3.dp)) {
                                        Row(Modifier.fillMaxWidth()) {
                                            TextButton(onClick = { selectedSettings = pane }, modifier = Modifier.weight(1f)) { SourceIcon(pane.source); Text(pane.source.title, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis) }
                                            MobileAction(MobileSymbol.Close, "Close ${pane.source.title}") { WorkspaceClient.remove(pane.id) }
                                        }
                                        Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                                            if (foreground && state.connected) WorkspaceVideo(pane, state.sessionKey, state.connectionGeneration, onError = { localError = it })
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if (state.panes.isEmpty()) Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                Column(horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(12.dp), modifier = Modifier.padding(24.dp)) {
                    Icon(MobileSymbol.Sources.image, null, tint = ColorAccentEmerald, modifier = Modifier.size(40.dp))
                    Text(if (state.incomingEnded) "Sharing ended" else "Choose a screen", color = ColorTextPrimary, style = MaterialTheme.typography.titleLarge)
                    Text(if (state.incomingEnded) "You can still exchange files."
                        else if (state.sources.isEmpty()) "This device has not shared a screen yet. You can still exchange files."
                        else "Open a display or application window. File transfers stay independent.",
                        color = ColorTextSecondary, style = MaterialTheme.typography.bodyMedium,
                        textAlign = androidx.compose.ui.text.style.TextAlign.Center)
                    if (!state.incomingEnded) Button(onClick = {
                        WorkspaceClient.refreshSources(sessionKey, state.connectionGeneration)
                        pickerGeneration = state.connectionGeneration; sourcePicker = true
                    }, enabled = state.connected) {
                        Text("Browse screens and windows")
                    }
                }
            }
            if (foreground && state.connected) visible.mapNotNull { it.audioOwner }.distinct().forEach { owner ->
                key(owner) { WorkspaceAudio(owner, volumes[owner] ?: 1f, onError = { localError = it }) }
            }
        }
        selectedSettings?.let { pane ->
            var fps by remember(pane.id) { mutableStateOf(pane.fps.toString()) }
            var kbps by remember(pane.id) { mutableStateOf(pane.kbps.toString()) }
            AlertDialog(onDismissRequest = { selectedSettings = null }, title = { Text("Stream settings") },
                text = { Column {
                    OutlinedTextField(fps, { fps = it }, label = { Text("Frames per second") })
                    OutlinedTextField(kbps, { kbps = it }, label = { Text("Bitrate (kbps)") })
                    pane.audioOwner?.let { owner ->
                        Text("Volume")
                        Slider(value = volumes[owner] ?: 1f, onValueChange = { volumes[owner] = it })
                    }
                } },
                confirmButton = { TextButton(onClick = {
                    val f = fps.toIntOrNull(); val k = kbps.toIntOrNull()
                    if (f != null && k != null && f > 0 && k > 0) { WorkspaceClient.settings(pane.id, state.sessionKey, state.connectionGeneration, fps = f, kbps = k); selectedSettings = null }
                }) { Text("Apply") } })
        }
        if (sharedFilesOpen) AlertDialog(onDismissRequest = { sharedFilesOpen = false }, title = { Text("Remote shared files") },
            text = { Column {
                Text(state.status)
                Text("Only explicitly shared files are listed. No remote filesystem paths are accepted.")
                state.remoteFiles.forEach { file ->
                    TextButton(onClick = { WorkspaceClient.fetchSharedFile(file.id, sessionKey) }, enabled = state.connected && !state.sharedBusy) { Text("Get ${file.name} (${file.sizeBytes} bytes)") }
                }
                Row {
                    TextButton(onClick = { WorkspaceClient.listSharedFiles() }, enabled = state.connected && !state.sharedBusy) { Text("Refresh") }
                    state.sharedNext?.let { next -> TextButton(onClick = { WorkspaceClient.listSharedFiles(next) }, enabled = state.connected && !state.sharedBusy) { Text("Next page") } }
                }
            } }, confirmButton = { TextButton(onClick = { sharedFilesOpen = false }) { Text("Close") } })
        if (receivedFiles) ReceivedFilesDialog { receivedFiles = false }
    }
}

@Composable
private fun WorkspaceAudio(owner: Int, volume: Float, onError: (String) -> Unit) {
    val player = remember(owner) { AudioOpusPlayer() }
    SideEffect { player.setVolume(volume) }
    DisposableEffect(player) { onDispose { player.stop() } }
    LaunchedEffect(owner) {
        withContext(Dispatchers.IO) {
            try {
                while (isActive) {
                    repeat(8) { WorkspaceClient.nativeMedia(owner + 2, true)?.let { player.feed(it) } }
                    player.drain(); delay(5)
                }
            } catch (e: Exception) { if (e !is CancellationException) withContext(Dispatchers.Main) { onError(e.message ?: "Audio failed") } }
            finally { player.stop() }
        }
    }
}

@Composable
private fun WorkspaceVideo(pane: WorkspacePane, sessionKey: String, generation: Long, onError: (String) -> Unit) {
    var dimensions by remember(pane.id, sessionKey, generation) {
        mutableStateOf(VideoDimensions(pane.source.width.coerceAtLeast(1), pane.source.height.coerceAtLeast(1)))
    }
    val transform = remember(pane.id, sessionKey, generation) { mutableStateOf(ViewportTransform()) }
    var measured by remember { mutableStateOf(androidx.compose.ui.unit.IntSize.Zero) }
    var surfaceToken by remember { mutableLongStateOf(0L) }
    var videoStatus by remember(pane.id, sessionKey, generation) { mutableStateOf<String?>("Waiting for video…") }
    val pointers = remember { mutableSetOf<Int>() }
    var touching by remember { mutableStateOf(false) }
    val currentError by rememberUpdatedState(onError)
    LaunchedEffect(measured, pane.accepted) {
        if (!pane.accepted) return@LaunchedEffect
        delay(250)
        // Negotiate only visible video pixels. Letterboxing/IME/toolbars are not video.
        VideoDimensions.fitEncoder(pane.source.width, pane.source.height,
            measured.width.coerceAtMost(4096), measured.height.coerceAtMost(4096), 2, 2)
            ?.takeIf { it.width >= 32 && it.height >= 32 }?.let {
                WorkspaceClient.settings(pane.id, sessionKey, generation, width = it.width, height = it.height)
            }
    }
    LaunchedEffect(touching, surfaceToken) {
        val token = surfaceToken
        if (touching && token != 0L) while (isActive) {
            WorkspaceClient.surface(pane.id, sessionKey, generation, token, "alive")
            delay(100)
        }
    }
    val token = surfaceToken
    var keyboardBinding by remember(pane.id,sessionKey,generation,token) { mutableStateOf<String?>(null) }
    var keyboardVisible by remember(pane.id,sessionKey,generation,token) { mutableStateOf(false) }
    val keyboardOwner = keyboardBinding
    RemoteKeyboard(identity = listOf(pane.id,sessionKey,generation,token,keyboardOwner), visible = keyboardVisible,
        enabled = pane.input && token != 0L && keyboardOwner != null,
        onText = { text -> keyboardOwner?.let { WorkspaceClient.keyboard(pane.id,sessionKey,generation,token,it,text=text) } },
        onKey = { key,pressed -> keyboardOwner?.let { WorkspaceClient.keyboard(pane.id,sessionKey,generation,token,it,key=key,pressed=pressed) } })
    RemoteViewport(
        dimensions = dimensions, transformState = transform,
        enabled = pane.input && token != 0L,
        contentKey = token to dimensions,
        modifier = Modifier.fillMaxSize(),
        onViewportSizeChanged = { measured = it },
        sendTouch = { action, id, x, y, pressure, eventTime ->
            when (action) {
                0 -> pointers.add(id)
                2 -> pointers.remove(id)
                3 -> pointers.clear()
            }
            touching = pointers.isNotEmpty()
            WorkspaceClient.touch(pane.id, sessionKey, generation, token, eventTime,
                JSONObject().put("action", when (action) { 0 -> "Down"; 1 -> "Move"; 2 -> "Up"; else -> "Cancel" })
                    .put("pointer_id", id).put("normalized_x", x).put("normalized_y", y).put("pressure", pressure))
        },
        overlay = {
            videoStatus?.let { message ->
                Text(message, color = ColorTextPrimary, style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.align(Alignment.TopCenter).padding(12.dp)
                        .background(ColorSurfaceCard.copy(alpha = .94f), androidx.compose.foundation.shape.RoundedCornerShape(8.dp))
                        .padding(horizontal = 12.dp, vertical = 8.dp))
            }
            MobileAction(MobileSymbol.Keyboard, if (keyboardVisible) "Hide keyboard" else "Show keyboard",
                enabled = pane.input && token != 0L,
                modifier = Modifier.align(Alignment.BottomEnd).padding(8.dp)
                    .background(ColorSurfaceCard.copy(alpha = .94f), androidx.compose.foundation.shape.RoundedCornerShape(16.dp))) {
                if (keyboardVisible) keyboardVisible = false else {
                    keyboardBinding = WorkspaceClient.keyboardBinding(pane.id, sessionKey, generation, token)
                    keyboardVisible = keyboardBinding != null
                }
            }
            if (transform.value.scale > 1.05f) {
                TextButton(onClick = { transform.value = transform.value.reset() }, modifier = Modifier.align(Alignment.TopEnd)) {
                    Text("Reset zoom")
                }
            }
        },
        factory = { context ->
            SurfaceView(context).apply {
                contentDescription = "Remote screen"
                fun clearTouches() { pointers.clear(); touching = false }
                holder.addCallback(object : SurfaceHolder.Callback {
                    private var active = AtomicBoolean(false)
                    private var worker: Thread? = null
                    override fun surfaceCreated(holder: SurfaceHolder) {
                        val token = WorkspaceClient.nextSurfaceToken()
                        surfaceToken = token
                        videoStatus = "Waiting for video…"
                        clearTouches()
                        WorkspaceClient.surface(pane.id, sessionKey, generation, token, "begin")
                        val running = AtomicBoolean(true)
                        active = running
                        worker = Thread({
                            val codec = MediaCodecPlayer(holder.surface) { decoded -> post {
                                if (running.get() && surfaceToken == token) dimensions = decoded
                            } }
                            try {
                                codec.start(pane.width, pane.height)
                                WorkspaceClient.keyframe(pane.id)
                                var waiting = true
                                var pending: ByteArray? = null
                                var pendingSince = 0L
                                var lastRequest = System.nanoTime()
                                var reportedFirstFrame = false
                                var framesBeforeReset = 0L
                                val started = System.nanoTime()
                                var startupReported = false
                                var receivedPackets = 0L
                                var submittedPackets = 0L
                                while (running.get()) {
                                    if (pending == null) {
                                        pending = WorkspaceClient.nativeMedia(pane.id, false)
                                        if (pending != null) receivedPackets++
                                        pendingSince = System.nanoTime()
                                    }
                                    val bytes = pending
                                    if (bytes != null && bytes.size > 9) {
                                        val keyframe = bytes[0].toInt() != 0
                                        val pts = ByteBuffer.wrap(bytes, 1, 8).order(ByteOrder.LITTLE_ENDIAN).long
                                        if (waiting && !keyframe) pending = null
                                        else if (codec.feedNalu(bytes, keyframe, pts, 9)) { pending = null; waiting = false; submittedPackets++ }
                                    }
                                    if (pending != null && System.nanoTime() - pendingSince > 250_000_000) {
                                        pending = null; waiting = true
                                        WorkspaceClient.surface(pane.id, sessionKey, generation, token, "reset")
                                        codec.flush()
                                        framesBeforeReset = codec.outputFrames
                                        reportedFirstFrame = false
                                        WorkspaceClient.keyframe(pane.id)
                                        lastRequest = System.nanoTime()
                                    }
                                    codec.drain()
                                    if (!reportedFirstFrame && codec.outputFrames > framesBeforeReset) {
                                        post { if (running.get() && surfaceToken == token) videoStatus = null }
                                        WorkspaceClient.surface(pane.id, sessionKey, generation, token, "ready")
                                        android.util.Log.i("WeaveloWorkspace", "First decoded frame for subscription ${pane.id}")
                                        reportedFirstFrame = true
                                    }
                                    if (!startupReported && codec.outputFrames == 0L && System.nanoTime() - started > 10_000_000_000L) {
                                        startupReported = true
                                        val message = when {
                                            receivedPackets == 0L -> "Waiting for video from this device."
                                            submittedPackets == 0L -> "Video arrived, but the decoder is not accepting it."
                                            else -> "Video arrived, but the decoder has not produced a frame."
                                        }
                                        android.util.Log.w("WeaveloWorkspace", "Video startup stalled: subscription=${pane.id}, received=$receivedPackets, submitted=$submittedPackets, surface=${holder.surface.isValid}")
                                        post { if (running.get() && surfaceToken == token) videoStatus = message }
                                    }
                                    if (waiting && System.nanoTime() - lastRequest > 1_000_000_000) { WorkspaceClient.keyframe(pane.id); lastRequest = System.nanoTime() }
                                    Thread.sleep(4)
                                }
                            } catch (e: Exception) { if (running.get()) post { currentError(e.message ?: "Video decoder failed") } }
                            finally {
                                WorkspaceClient.surface(pane.id, sessionKey, generation, token, "end")
                                codec.stop()
                            }
                        }, "workspace-video-${pane.id}").apply { start() }
                    }
                    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
                        clearTouches()
                        WorkspaceClient.surface(pane.id, sessionKey, generation, surfaceToken, "cancel")
                    }
                    override fun surfaceDestroyed(holder: SurfaceHolder) {
                        val token = surfaceToken
                        surfaceToken = 0L
                        clearTouches()
                        WorkspaceClient.surface(pane.id, sessionKey, generation, token, "end")
                        active.set(false); worker?.interrupt(); worker?.join(500); worker = null
                    }
                })
            }
        }
    )
}

@Composable
private fun SourceIcon(source: WorkspaceSource) {
    source.icon?.let { bitmap ->
        Image(bitmap = remember(bitmap) { bitmap.asImageBitmap() }, contentDescription = source.application.ifBlank { source.title }, modifier = Modifier.size(20.dp))
        Spacer(Modifier.width(4.dp))
    }
}

@Composable
private fun SourcePicker(sources: List<WorkspaceSource>, onClose: () -> Unit, onSelect: (WorkspaceSource) -> Unit) {
    var query by remember { mutableStateOf("") }
    val matches = remember(sources, query) { sources.filter { it.title.contains(query, true) || it.application.contains(query, true) } }
    AlertDialog(onDismissRequest = onClose, title = { Text("Screens and app windows") }, text = {
        Column(Modifier.fillMaxWidth()) {
            OutlinedTextField(query, { query = it }, label = { Text("Search apps and windows") }, singleLine = true, modifier = Modifier.fillMaxWidth())
            Spacer(Modifier.height(8.dp))
            LazyColumn(Modifier.fillMaxWidth().heightIn(max = 300.dp)) {
                items(matches, key = { it.source.toString() }) { source ->
                    TextButton(onClick = { onSelect(source) }, modifier = Modifier.fillMaxWidth()) {
                        SourceIcon(source)
                        Column(Modifier.weight(1f)) {
                            Text(source.title, maxLines = 2)
                            if (source.application.isNotBlank()) Text(source.application, style = MaterialTheme.typography.labelMedium)
                        }
                    }
                }
                if (matches.isEmpty()) item { Text("No matching screen or app window") }
            }
        }
    }, confirmButton = { TextButton(onClick = onClose) { Text("Close") } })
}
