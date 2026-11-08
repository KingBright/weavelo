package `fun`.hackerlife.weavelo.ui

import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.Image
import androidx.compose.foundation.Canvas
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.systemGestureExclusion
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.unit.IntOffset
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.DialogProperties
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
    incoming: WorkspaceSessionRequest? = null, fileToSend: File? = null,
    onFileQueued: () -> Unit = {}, openMainDisplay: Boolean = false, onClose: () -> Unit) {
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
    StreamOrientation(active = state.panes.isNotEmpty())
    val typing = state.panes.isNotEmpty() && WindowInsets.ime.getBottom(LocalDensity.current) > 0
    var hudVisible by remember(sessionKey) { mutableStateOf(false) }
    var hudInteraction by remember(sessionKey) { mutableLongStateOf(0L) }
    val devices by WeaveloClient.devices.collectAsState()
    val remoteDevice = devices.firstOrNull { it.id == deviceId }
    val activePane = state.panes.firstOrNull { it.id == state.selected } ?: state.panes.firstOrNull()
    LaunchedEffect(activePane?.id, activePane?.appearance, activePane?.source?.icon, remoteDevice?.name) {
        (context as? android.app.Activity)?.let { activity ->
            if (activePane == null) `fun`.hackerlife.weavelo.DeviceArtwork.restore(activity)
            else `fun`.hackerlife.weavelo.DeviceArtwork.task(activity, activePane, remoteDevice?.name ?: "Device")
        }
    }
    DisposableEffect(context) {
        onDispose { (context as? android.app.Activity)?.let { `fun`.hackerlife.weavelo.DeviceArtwork.restore(it) } }
    }
    var foreground by remember { mutableStateOf(lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) }
    val disconnect: () -> Unit = {
        SessionConnectionService.closeWorkspace(context, sessionKey)
        onClose()
    }
    LaunchedEffect(state.sessionKey, state.incomingEnded, state.connected) {
        // Keep the current file context accessible until its jobs have drained.
        // Retiring the idle incoming connection then leaves this ended page.
        if (state.sessionKey == sessionKey && state.incomingEnded && !state.connected) onClose()
    }
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
    BackHandler(enabled = foreground) {
        val hasOverlay = sourcePicker || actionsOpen || selectedSettings != null || sharedFilesOpen || receivedFiles || backgroundSettings
        when {
            ZeroOcclusionHudPolicy.evaluateBackPress(hasOverlay) == BackPressAction.ToggleControls -> { hudVisible = !hudVisible; hudInteraction++ }
            sourcePicker -> sourcePicker = false
            actionsOpen -> actionsOpen = false
            selectedSettings != null -> selectedSettings = null
            sharedFilesOpen -> sharedFilesOpen = false
            receivedFiles -> receivedFiles = false
            backgroundSettings -> backgroundSettings = false
        }
    }
    LaunchedEffect(hudVisible, hudInteraction, actionsOpen, sourcePicker, selectedSettings, typing, state.panes.isEmpty(), state.connected) {
        if (!state.connected) hudVisible = true
        if (state.connected && hudVisible && state.panes.isNotEmpty() && !actionsOpen && !sourcePicker && selectedSettings == null && !typing) {
            delay(3500)
            hudVisible = false
        }
    }
    val volumes = remember { mutableStateMapOf<Int, Float>() }
    if (sourcePicker) {
        if (state.connectionGeneration != pickerGeneration || state.sessionKey != sessionKey || !state.connected) {
            LaunchedEffect(Unit) { sourcePicker = false }
        } else SourcePicker(state.sources, onClose = { sourcePicker = false }) { source ->
            localError = null
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
        SessionConnectionService.openWorkspace(context, incoming ?: WorkspaceSessionRequest(endpoint, deviceId, sessionKey, openMainDisplay = openMainDisplay))
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
    LaunchedEffect(sessionKey, state.sessionKey, state.connected, state.files, fileToSend) {
        if (fileToSend != null && state.sessionKey == sessionKey && state.connected && state.files) {
            onFileQueued()
            WorkspaceClient.sendFile(fileToSend, sessionKey)
        }
    }
    WeaveloTheme {
        Box(Modifier.fillMaxSize().background(if (state.panes.isEmpty()) ColorBaseBg else Color.Black)) {
            Column(Modifier.fillMaxSize().imePadding()) {
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
                                    Box(Modifier.weight(1f).fillMaxHeight().padding(if (visible.size > 1) 2.dp else 0.dp)) {
                                        if (foreground && state.connected) WorkspaceVideo(pane, state.sessionKey, state.connectionGeneration,
                                            mobileTarget = ZeroOcclusionHudPolicy.isMobileTarget(
                                                state.androidHost, pane.source.isCamera),
                                            isPhoneScreen = ZeroOcclusionHudPolicy.supportsRemoteHome(
                                                state.androidHost, pane.source.isCamera, pane.source.source.toString()),
                                            canRemoteBack = ZeroOcclusionHudPolicy.supportsRemoteBack(
                                                state.androidHost, pane.source.isCamera,
                                                pane.source.source.toString(), pane.source.isWindow),
                                            onMore = { hudVisible = true; hudInteraction++; actionsOpen = true }, onError = { localError = it; hudVisible = true; hudInteraction++ })
                                        if (hudVisible && !typing) Row(Modifier.align(Alignment.TopStart).fillMaxWidth()
                                            .safeDrawingPadding().padding(top = 56.dp)
                                            .background(ColorSurfaceCard.copy(alpha = .82f), androidx.compose.foundation.shape.RoundedCornerShape(14.dp))) {
                                            TextButton(onClick = { selectedSettings = pane }, modifier = Modifier.weight(1f)) { SourceIcon(pane.source.copy(icon = pane.appearance?.application ?: pane.source.icon)); Text(pane.source.title, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis) }
                                            if (WorkspaceClient.cameraSwitchTarget(pane) != null) {
                                                MobileAction(MobileSymbol.CameraSwitch, "Switch camera", state.connected) {
                                                    WorkspaceClient.switchCamera(pane.id, state.sessionKey, state.connectionGeneration)
                                                }
                                            }
                                            MobileAction(MobileSymbol.Close, "Close ${pane.source.title}") { WorkspaceClient.remove(pane.id) }
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
            if ((hudVisible || state.panes.isEmpty()) && !typing) Column(
                Modifier.align(Alignment.TopCenter).fillMaxWidth().safeDrawingPadding().padding(6.dp)
                    .background(ColorSurfaceCard.copy(alpha = .9f), androidx.compose.foundation.shape.RoundedCornerShape(18.dp))
                    .pointerInput(sessionKey) {
                        awaitPointerEventScope { while (true) if (awaitPointerEvent(PointerEventPass.Initial).type == PointerEventType.Press) hudInteraction++ }
                    }) {
            if (!typing) Row(Modifier.fillMaxWidth().heightIn(min = 56.dp), verticalAlignment = Alignment.CenterVertically) {
                MobileAction(MobileSymbol.Back, "Hide controls", onClick = { hudVisible = false })
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
                TextButton(onClick = { actionsOpen = false; disconnect() }) { Text("Disconnect") }
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
                Text(it, color = ColorTextSecondary, maxLines = 2)
            }
            if (!typing && !state.grid && state.panes.size > 1) Row(Modifier.horizontalScroll(rememberScrollState())) {
                state.panes.forEach { pane -> TextButton(onClick = { WorkspaceClient.layout(false, pane.id) }) { SourceIcon(pane.source.copy(icon = pane.appearance?.application ?: pane.source.icon)); Text(pane.source.title) } }
            }
            }
            if (!hudVisible && state.panes.isNotEmpty() && !typing && state.panes.any {
                !ZeroOcclusionHudPolicy.isMobileTarget(state.androidHost, it.source.isCamera)
            }) MobileAction(MobileSymbol.More, "Show stream controls",
                modifier = Modifier.align(Alignment.TopEnd).safeDrawingPadding().padding(8.dp).size(36.dp)
                    .background(ColorSurfaceCard.copy(alpha = .6f), androidx.compose.foundation.shape.RoundedCornerShape(18.dp))) {
                hudVisible = true; hudInteraction++
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
        if (receivedFiles) ReceivedFilesDialog(onSendToDevice = if (state.connected && state.files) { file ->
            receivedFiles = false
            WorkspaceClient.sendFile(file, sessionKey)
        } else null) { receivedFiles = false }
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
private fun WorkspaceVideo(pane: WorkspacePane, sessionKey: String, generation: Long,
    mobileTarget: Boolean, isPhoneScreen: Boolean, canRemoteBack: Boolean,
    onMore: () -> Unit, onError: (String) -> Unit) {
    pane.videoError?.let { reason ->
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            Column(Modifier.padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Text(reason, color = ColorTextSecondary, style = MaterialTheme.typography.bodyMedium,
                    textAlign = androidx.compose.ui.text.style.TextAlign.Center)
                TextButton(onClick = { WorkspaceClient.retry(pane.id, sessionKey, generation) }) { Text("Retry this source") }
            }
        }
        return
    }
    if (pane.source.isCamera && !pane.accepted) {
        Box(Modifier.fillMaxSize(),contentAlignment=Alignment.Center) { Text("Updating camera…",color=ColorTextSecondary) }
        return
    }
    var dimensions by remember(pane.id, sessionKey, generation) {
        mutableStateOf(VideoDimensions(pane.source.width.coerceAtLeast(1), pane.source.height.coerceAtLeast(1)))
    }
    val transform = remember(pane.id, sessionKey, generation) { mutableStateOf(ViewportTransform()) }
    var measured by remember { mutableStateOf(androidx.compose.ui.unit.IntSize.Zero) }
    var surfaceToken by remember { mutableLongStateOf(0L) }
    val token = surfaceToken
    var keyboardBinding by remember(pane.id,sessionKey,generation,token) { mutableStateOf<String?>(null) }
    var keyboardVisible by remember(pane.id,sessionKey,generation,token) { mutableStateOf(false) }
    val imeVisible = WindowInsets.ime.getBottom(LocalDensity.current) > 0
    var imeWasVisible by remember(pane.id, sessionKey, generation, token) { mutableStateOf(false) }
    LaunchedEffect(imeVisible) {
        if (imeWasVisible && !imeVisible) keyboardVisible = false
        imeWasVisible = imeVisible
    }
    var handleFraction by rememberSaveable(pane.id, sessionKey, generation) { mutableFloatStateOf(0.5f) }
    var mobileControls by remember(pane.id, sessionKey, generation) { mutableStateOf(false) }
    var mobileMore by remember(pane.id, sessionKey, generation) { mutableStateOf(false) }
    LaunchedEffect(mobileControls, mobileMore, keyboardVisible) {
        if (mobileControls && !mobileMore && !keyboardVisible) {
            kotlinx.coroutines.delay(4000)
            mobileControls = false
        }
    }

    var heldModifiers by remember(pane.id, sessionKey, generation, token) { mutableStateOf(emptySet<String>()) }
    var modifierBinding by remember(pane.id, sessionKey, generation, token) { mutableStateOf<String?>(null) }
    val virtualKeys = remember(pane.id, sessionKey, generation, token) {
        StickyKeyController { key, pressed ->
            modifierBinding?.let { owner ->
                WorkspaceClient.keyboard(pane.id, sessionKey, generation, token, owner, key = key, pressed = pressed)
            }
        }
    }
    DisposableEffect(virtualKeys) { onDispose { virtualKeys.releaseAll() } }
    LaunchedEffect(pane.input, token, imeVisible) {
        if (!pane.input || token == 0L || !imeVisible) {
            virtualKeys.releaseAll(); heldModifiers = emptySet(); modifierBinding = null
        }
    }
    val heldOwner = modifierBinding
    LaunchedEffect(imeVisible, heldModifiers, pane.input, token, heldOwner, pane.inputError) {
        if (heldModifiers.isNotEmpty() && pane.inputError != null) {
            virtualKeys.forgetAll(); heldModifiers = emptySet(); modifierBinding = null
            keyboardVisible = false; keyboardBinding = null
            onError("Keyboard control changed. Reopen the keyboard and select shortcuts again.")
        } else if (imeVisible && heldModifiers.isNotEmpty() && pane.input && token != 0L && heldOwner != null) {
            while (isActive) {
                delay(100)
                WorkspaceClient.keyboardAlive(pane.id, sessionKey, generation, token, heldOwner)
            }
        }
    }
    BackHandler(enabled = keyboardVisible || mobileMore || mobileControls) {
        when { mobileMore -> mobileMore = false; keyboardVisible -> keyboardVisible = false; else -> mobileControls = false }
    }
    var videoStatus by remember(pane.id, sessionKey, generation) { mutableStateOf<String?>("Waiting for video…") }
    val pointers = remember { mutableSetOf<Int>() }
    var touching by remember { mutableStateOf(false) }
    var touchMode by remember(pane.id, sessionKey, generation, pane.maxTouchPoints, pane.input) {
        mutableStateOf(if (!pane.input) TouchMode.VIEW else if (pane.maxTouchPoints > 0) TouchMode.NATIVE else TouchMode.DIRECT)
    }
    var cursor by remember(pane.id, sessionKey, generation) { mutableStateOf(Offset(.5f, .5f)) }
    val currentError by rememberUpdatedState(onError)
    val nativeViewport = remember(pane.id, sessionKey, generation) { java.util.concurrent.atomic.AtomicReference<GpuVideoSurface?>() }
    LaunchedEffect(measured, dimensions, pane.accepted, keyboardVisible, imeVisible) {
        // IME insets resize presentation and its inverse touch transform. Keep
        // the encoder/decoder and composition owner stable while typing.
        if (!pane.accepted || keyboardVisible || imeVisible) return@LaunchedEffect
        delay(250)
        // Negotiate only visible video pixels. Letterboxing/IME/toolbars are not video.
        val quarterTurn = pane.source.rotation % 180 != 0
        (if (!pane.source.isCamera) MediaCodecPlayer.screenSize(dimensions.width,dimensions.height) else VideoDimensions.fitEncoder(dimensions.width, dimensions.height,
            if (pane.source.isCamera) (if (quarterTurn) measured.height else measured.width).coerceAtMost(4096) else 4096,
            if (pane.source.isCamera) (if (quarterTurn) measured.width else measured.height).coerceAtMost(4096) else 4096, 2, 2))
            ?.takeIf { it.width >= 32 && it.height >= 32 }?.let {
                WorkspaceClient.settings(pane.id, sessionKey, generation, width = it.width, height = it.height)
            }
    }
    LaunchedEffect(pane.id, sessionKey, generation, pane.accepted) {
        if (!pane.accepted || pane.source.isCamera) return@LaunchedEffect
        var previous = emptyList<Int>()
        while (isActive) {
            val size = measured
            if (size.width >= 16 && size.height >= 16) {
                val rect = transform.value.visibleRect(size.width.toFloat(),size.height.toFloat())
                val next = rect.toList() + listOf(size.width,size.height)
                if (next != previous) {
                    WorkspaceClient.viewport(pane.id,sessionKey,generation,rect,size.width,size.height)
                    previous=next
                }
            }
            delay(50) // latest state only; continuous gestures cannot starve updates
        }
    }
    LaunchedEffect(touching, surfaceToken) {
        val token = surfaceToken
        if (touching && token != 0L) while (isActive) {
            WorkspaceClient.surface(pane.id, sessionKey, generation, token, "alive")
            delay(100)
        }
    }
    val keyboardOwner = keyboardBinding
    RemoteKeyboard(identity = listOf(pane.id,sessionKey,generation,token,keyboardOwner), visible = keyboardVisible,
        enabled = pane.input && token != 0L && keyboardOwner != null,
        onText = { text -> keyboardOwner?.let { WorkspaceClient.keyboard(pane.id,sessionKey,generation,token,it,text=text) } },
        onKey = { key,pressed -> keyboardOwner?.let { WorkspaceClient.keyboard(pane.id,sessionKey,generation,token,it,key=key,pressed=pressed) } })
    Column(Modifier.fillMaxSize()) {
      BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
        val viewportHeightPx = with(LocalDensity.current) { maxHeight.toPx().coerceAtLeast(1f) }
        LaunchedEffect(maxWidth, maxHeight, imeVisible) {
            mobileMore = false
            mobileControls = false
        }
        RemoteViewport(
            dimensions = dimensions, transformState = transform,
            rotation = pane.source.rotation,
            enabled = pane.input && token != 0L,
            touchMode = touchMode,
            contentKey = token to dimensions,
            // Controls overlay the viewport; opening them never changes input geometry.
            modifier = Modifier.fillMaxSize(),
            onViewportSizeChanged = { measured = it },
            sendTouch = { action, id, x, y, pressure, eventTime ->
                val presentation = nativeViewport.get()?.presentation
                val geometry = presentation?.geometry
                if (action != 3 && (presentation?.ready != true || geometry != null &&
                    (geometry.sourceRevision != 0 || geometry.sourceWidth != dimensions.width || geometry.sourceHeight != dimensions.height ||
                        touchMode == TouchMode.DIRECT && !geometry.contains(x,y)))) {
                    pointers.clear(); touching=false
                    WorkspaceClient.surface(pane.id,sessionKey,generation,token,"cancel")
                } else {
                when (action) {
                    0 -> pointers.add(id)
                    2 -> pointers.remove(id)
                    3 -> pointers.clear()
                }
                touching = pointers.isNotEmpty()
                WorkspaceClient.touch(pane.id, sessionKey, generation, token, eventTime,
                    touchMode, geometry?.sourceWidth ?: pane.source.width, geometry?.sourceHeight ?: pane.source.height,
                    JSONObject().put("action", when (action) { 0 -> "Down"; 1 -> "Move"; 2 -> "Up"; else -> "Cancel" })
                        .put("pointer_id", id).put("normalized_x", x).put("normalized_y", y).put("pressure", pressure),geometry) { cx, cy ->
                    if (surfaceToken == token) cursor = Offset(cx, cy)
                if (surfaceToken == token && touchMode == TouchMode.TRACKPAD && transform.value.scale>1f) {
                    transform.value=transform.value.reveal(cx,cy,measured.width.toFloat(),measured.height.toFloat())
                }
                }
                }
            },
            sendTouchFrame = { sample ->
                val presentation=nativeViewport.get()?.presentation
                val geometry=presentation?.geometry
                val invalid=sample.size>pane.maxTouchPoints || presentation?.ready!=true || geometry==null ||
                    geometry.sourceRevision!=0 || geometry.sourceWidth!=dimensions.width || geometry.sourceHeight!=dimensions.height ||
                    sample.any { it.action!=3 && !geometry.contains(it.x,it.y) }
                if(invalid) {
                    pointers.clear();touching=false
                    WorkspaceClient.surface(pane.id,sessionKey,generation,token,"cancel")
                } else {
                    val contacts=org.json.JSONArray()
                    sample.forEach { c ->
                        when(c.action) {0->pointers.add(c.pointerId);2,3->pointers.remove(c.pointerId)}
                        contacts.put(JSONObject().put("action",when(c.action){0->"Down";1->"Move";2->"Up";else->"Cancel"})
                            .put("pointer_id",Integer.toUnsignedLong(c.pointerId)).put("normalized_x",c.x)
                            .put("normalized_y",c.y).put("pressure",c.pressure))
                    }
                    touching=pointers.isNotEmpty()
                    WorkspaceClient.touch(pane.id,sessionKey,generation,token,sample.minOf {it.time},TouchMode.NATIVE,
                        geometry.sourceWidth,geometry.sourceHeight,JSONObject(),geometry,contacts)
                }
            },
            overlay = {
                if (pane.input && pane.accepted && touchMode == TouchMode.TRACKPAD) {
                    // This layer shares the video's forward transform and never
                    // consumes gestures or reserves space around the picture.
                    Canvas(Modifier.fillMaxSize()) {
                        val (x, y) = transform.value.normalizedToScreen(cursor.x, cursor.y, size.width, size.height)
                        val unit = 1.dp.toPx()
                        val arrow = Path().apply {
                            moveTo(0f, 0f); lineTo(0f, 20f * unit)
                            lineTo(5f * unit, 15f * unit); lineTo(9f * unit, 23f * unit)
                            lineTo(12f * unit, 21f * unit); lineTo(8f * unit, 13f * unit)
                            lineTo(15f * unit, 13f * unit); close()
                        }
                        translate(x, y) {
                            drawPath(arrow, Color.Black, style = Stroke(3f * unit))
                            drawPath(arrow, Color.White)
                        }
                    }
                }
                if (imeVisible && pane.input) MobileAction(MobileSymbol.Keyboard, "Hide keyboard",
                    modifier = Modifier.align(Alignment.TopEnd).size(36.dp)
                        .background(ColorSurfaceCard.copy(alpha = .85f), androidx.compose.foundation.shape.RoundedCornerShape(8.dp))) {
                    keyboardVisible = false
                }
                (videoStatus ?: pane.inputError)?.let { message ->
                    Text(message, color = ColorTextPrimary, style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.align(Alignment.TopCenter).padding(12.dp)
                            .background(ColorSurfaceCard.copy(alpha = .94f), androidx.compose.foundation.shape.RoundedCornerShape(8.dp))
                            .padding(horizontal = 12.dp, vertical = 8.dp))
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
                            WorkspaceClient.videoDiagnostic(pane.id,sessionKey,generation,token,"decoder-start",0,0,0)
                            val running = AtomicBoolean(true)
                            active = running
                            worker = Thread({
                                var target: GpuVideoSurface? = null
                                var codec: MediaCodecPlayer? = null
                                var reportedGeometry: VideoFrameGeometry? = null
                                try {
                                    if (!pane.source.isCamera) {
                                        target=GpuVideoSurface(holder.surface,pane.source.width,pane.source.height) { geometry ->
                                            if (geometry != null && geometry != reportedGeometry) {
                                                reportedGeometry = geometry
                                                WorkspaceClient.presented(pane.id,sessionKey,generation,token,geometry)
                                            }
                                            if (geometry != null) post {
                                                if (running.get() && surfaceToken == token) dimensions=VideoDimensions(geometry.sourceWidth,geometry.sourceHeight)
                                            }
                                        }
                                        nativeViewport.set(target)
                                    }
                                    val player=MediaCodecPlayer(target?.input ?: holder.surface,target) { decoded -> post {
                                        if (running.get() && surfaceToken == token && nativeViewport.get()?.presentation?.geometry == null) dimensions=decoded
                                    } }
                                    codec=player
                                    val size=if (target != null) checkNotNull(MediaCodecPlayer.screenSize(pane.source.width,pane.source.height)) { "No supported native screen decoder size" }
                                        else VideoDimensions(pane.width,pane.height)
                                    player.start(size.width,size.height)
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
                                            else if (player.feedNalu(bytes, keyframe, pts, 9)) { pending = null; waiting = false; submittedPackets++ }
                                        }
                                        if (pending != null && System.nanoTime() - pendingSince > 250_000_000) {
                                            pending = null; waiting = true
                                            WorkspaceClient.surface(pane.id, sessionKey, generation, token, "reset")
                                            reportedGeometry = null
                                            player.flush()
                                            framesBeforeReset = player.outputFrames
                                            reportedFirstFrame = false
                                            WorkspaceClient.keyframe(pane.id)
                                            lastRequest = System.nanoTime()
                                        }
                                        player.drain()
                                        if (!reportedFirstFrame && player.outputFrames > framesBeforeReset) {
                                            post { if (running.get() && surfaceToken == token) videoStatus = null }
                                            WorkspaceClient.surface(pane.id, sessionKey, generation, token, "ready")
                                            android.util.Log.i("WeaveloWorkspace", "First decoded frame for subscription ${pane.id}")
                                            WorkspaceClient.videoDiagnostic(pane.id,sessionKey,generation,token,"first-decoder-output",
                                                receivedPackets,submittedPackets,player.outputFrames)
                                            reportedFirstFrame = true
                                        }
                                        if (!startupReported && player.outputFrames == 0L && System.nanoTime() - started > 10_000_000_000L) {
                                            startupReported = true
                                            val message = when {
                                                receivedPackets == 0L -> "Waiting for video from this device."
                                                submittedPackets == 0L -> "Video arrived, but the decoder is not accepting it."
                                                else -> "Video arrived, but the decoder has not produced a frame."
                                            }
                                            android.util.Log.w("WeaveloWorkspace", "Video startup stalled: subscription=${pane.id}, received=$receivedPackets, submitted=$submittedPackets, surface=${holder.surface.isValid}")
                                            WorkspaceClient.videoDiagnostic(pane.id,sessionKey,generation,token,"startup-stalled",
                                                receivedPackets,submittedPackets,player.outputFrames)
                                            post { if (running.get() && surfaceToken == token) videoStatus = message }
                                        }
                                        if (waiting && System.nanoTime() - lastRequest > 1_000_000_000) { WorkspaceClient.keyframe(pane.id); lastRequest = System.nanoTime() }
                                        Thread.sleep(4)
                                    }
                                } catch (e: Exception) { if (running.get()) post { currentError(e.message ?: "Video decoder failed") } }
                                finally {
                                    WorkspaceClient.surface(pane.id, sessionKey, generation, token, "end")
                                    codec?.stop()
                                    nativeViewport.compareAndSet(target,null)
                                    target?.close()
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
        if (!imeVisible) {
            Box(Modifier.align(Alignment.CenterEnd)
                .offset { IntOffset(0, ((handleFraction - .5f) * viewportHeightPx).toInt()) }
                .size(width = 44.dp, height = 64.dp)
                .systemGestureExclusion { coordinates ->
                    Rect(0f, 0f, coordinates.size.width.toFloat(), coordinates.size.height.toFloat())
                }
                .pointerInput(pane.id, sessionKey, generation, token, viewportHeightPx) {
                    var horizontal = 0f
                    detectDragGestures(onDragStart = { horizontal = 0f }, onDragEnd = {
                        if (ZeroOcclusionHudPolicy.revealsControls(horizontal, 24.dp.toPx())) {
                            WorkspaceClient.surface(pane.id, sessionKey, generation, token, "cancel")
                            pointers.clear(); touching = false; mobileControls = true
                        }
                    }) { change, drag ->
                        change.consume(); horizontal += drag.x
                        handleFraction = ZeroOcclusionHudPolicy.dragEdgePillFraction(handleFraction, drag.y, viewportHeightPx)
                    }
                }
                .clickable(onClickLabel = "Show remote controls") {
                    WorkspaceClient.surface(pane.id, sessionKey, generation, token, "cancel")
                    pointers.clear(); touching = false; mobileControls = !mobileControls
                }
                .padding(start = 39.dp, top = 12.dp, bottom = 12.dp)
                .background(ColorAccentCyan, RoundedCornerShape(4.dp)))
            if (mobileControls && mobileTarget) Row(Modifier.align(Alignment.CenterEnd).padding(end = 48.dp)
                .background(ColorSurfaceCard, RoundedCornerShape(18.dp)).padding(4.dp),
                verticalAlignment = Alignment.CenterVertically) {
                MobileAction(MobileSymbol.Back, "Remote Back", enabled = pane.input && token != 0L && pane.accepted && canRemoteBack, onClick = {
                    val owner = WorkspaceClient.keyboardBinding(pane.id, sessionKey, generation, token)
                    owner?.let { WorkspaceClient.navigation(pane.id, sessionKey, generation, token, it, WorkspaceNavigation.BACK) }
                })
                MobileAction(MobileSymbol.Home, "Remote Home", enabled = pane.input && token != 0L && pane.accepted && isPhoneScreen, onClick = {
                    val owner = WorkspaceClient.keyboardBinding(pane.id, sessionKey, generation, token)
                    owner?.let { WorkspaceClient.navigation(pane.id, sessionKey, generation, token, it, WorkspaceNavigation.HOME) }
                })
                MobileAction(MobileSymbol.Keyboard, "Keyboard", enabled = pane.input && token != 0L, onClick = {
                    if (keyboardVisible) keyboardVisible = false else {
                        keyboardBinding = WorkspaceClient.keyboardBinding(pane.id, sessionKey, generation, token)
                        keyboardVisible = keyboardBinding != null
                    }
                })
                Box {
                    MobileAction(MobileSymbol.More, "More remote actions", onClick = { mobileMore = true })
                    DropdownMenu(expanded = mobileMore, onDismissRequest = { mobileMore = false }) {
                        DropdownMenuItem(enabled = pane.input, text = { Text(if (touchMode == TouchMode.VIEW) "Control" else "Adjust view") }, onClick = {
                            mobileMore = false
                            WorkspaceClient.surface(pane.id, sessionKey, generation, token, "cancel")
                            pointers.clear(); touching = false
                            touchMode = if (touchMode == TouchMode.VIEW) {
                                if (pane.maxTouchPoints > 0) TouchMode.NATIVE else TouchMode.DIRECT
                            } else TouchMode.VIEW
                        })
                        DropdownMenuItem(enabled = pane.input, text = { Text(if (touchMode == TouchMode.TRACKPAD) {
                            if (pane.maxTouchPoints > 0) "Use multi-touch" else "Use direct touch"
                        } else "Use trackpad") }, onClick = {
                            mobileMore = false
                            WorkspaceClient.surface(pane.id, sessionKey, generation, token, "cancel")
                            pointers.clear(); touching = false
                            touchMode = if (touchMode == TouchMode.TRACKPAD) {
                                if (pane.maxTouchPoints > 0) TouchMode.NATIVE else TouchMode.DIRECT
                            } else TouchMode.TRACKPAD
                        })
                        if (!pane.input) DropdownMenuItem(
                            text = { Text("Remote control is unavailable; you can adjust the view") },
                            enabled = false, onClick = {})
                        if (!canRemoteBack) DropdownMenuItem(
                            text = { Text("Back is available for an Android screen or app window") },
                            enabled = false, onClick = {})
                        if (!isPhoneScreen) DropdownMenuItem(
                            text = { Text("Home is available for the shared Android screen") },
                            enabled = false, onClick = {})
                        DropdownMenuItem(text = { Text("Reset zoom") }, onClick = { mobileMore = false; transform.value = transform.value.reset() })
                        DropdownMenuItem(text = { Text("Connection actions") }, onClick = { mobileMore = false; onMore() })
                    }
                }
            }
        }
      if (!mobileTarget && !imeVisible && mobileControls) {
        Box(Modifier.align(Alignment.BottomCenter)) {
        MobileInputControls(trackpad = touchMode == TouchMode.TRACKPAD,
            direct = touchMode == TouchMode.DIRECT || touchMode == TouchMode.NATIVE,
            inputEnabled = pane.input && token != 0L,
            onTrackpad = {
                WorkspaceClient.surface(pane.id, sessionKey, generation, token, "cancel")
                pointers.clear(); touching = false; touchMode = TouchMode.TRACKPAD
            }, onDirect = {
                WorkspaceClient.surface(pane.id, sessionKey, generation, token, "cancel")
                pointers.clear(); touching = false
                touchMode = if (pane.maxTouchPoints > 0) TouchMode.NATIVE else TouchMode.DIRECT
            }, onKeyboard = {
                if (keyboardVisible) keyboardVisible = false else {
                    keyboardBinding = WorkspaceClient.keyboardBinding(pane.id, sessionKey, generation, token)
                    keyboardVisible = keyboardBinding != null
                }
            }, onReset = { transform.value = transform.value.reset() })
      }
      }
      }
      if (imeVisible && pane.input) {
        Row(Modifier.fillMaxWidth().background(ColorSurfaceCard).horizontalScroll(rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(4.dp), verticalAlignment = Alignment.CenterVertically) {
            listOf("Esc", "Ctrl", "Alt", "Win", "Shift", "Tab", "F5", "F11", "▲", "▼", "◄", "►").forEach { key ->
                VirtualKeyPill(name = key, active = key.lowercase() in heldModifiers, enabled = token != 0L) {
                    val owner = if (heldModifiers.isNotEmpty()) modifierBinding else keyboardBinding
                    if (owner != null) {
                        modifierBinding = owner
                        virtualKeys.tap(key, token != 0L)
                        heldModifiers = virtualKeys.selected()
                        if (heldModifiers.isEmpty()) modifierBinding = null
                    }
                }
            }
        }
      }
    }
}

@Composable
private fun SourceIcon(source: WorkspaceSource) {
    Box(Modifier.size(24.dp),contentAlignment=Alignment.Center) {
        val bitmap=source.icon
        if (bitmap!=null) Image(bitmap = remember(bitmap) { bitmap.asImageBitmap() }, contentDescription = source.application.ifBlank { source.title }, modifier = Modifier.size(20.dp))
        else Icon((if (source.isCamera) MobileSymbol.Camera else if (source.isWindow) MobileSymbol.Sources else MobileSymbol.Devices).image,
            contentDescription=null,modifier=Modifier.size(20.dp))
    }
    Spacer(Modifier.width(4.dp))
}

@Composable
private fun SourcePicker(sources: List<WorkspaceSource>, onClose: () -> Unit, onSelect: (WorkspaceSource) -> Unit) {
    var query by remember { mutableStateOf("") }
    val matches = remember(sources, query) {
        val hasDisplay = sources.any { (it.source as? JSONObject)?.has("Display") == true }
        sources.filter { !(hasDisplay && it.source == "MainDisplay") }
            .filter { it.title.contains(query.trim(), true) || it.application.contains(query.trim(), true) }
            .sortedBy { if (it.isCamera) 2 else if (it.isWindow) 1 else 0 }
    }
    AlertDialog(onDismissRequest = onClose,
        modifier = Modifier.imePadding().navigationBarsPadding().widthIn(max = 560.dp).fillMaxWidth(.9f),
        properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false),
        title = { Text("Sources", maxLines = 1) }, text = {
        Column(Modifier.fillMaxWidth()) {
            OutlinedTextField(query, { query = it }, label = { Text("Search apps and windows") }, singleLine = true, modifier = Modifier.fillMaxWidth())
            Spacer(Modifier.height(8.dp))
            LazyColumn(Modifier.weight(1f, fill = false).fillMaxWidth().heightIn(max = 300.dp)) {
                itemsIndexed(matches, key = { _, source -> source.source.toString() }) { index, source ->
                  Column(Modifier.fillMaxWidth()) {
                    if (index == 0 || matches[index - 1].group != source.group) {
                        Text(source.group,
                            style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary,
                            modifier = Modifier.padding(top = 8.dp, bottom = 4.dp))
                    }
                    val app=source.application.trim().ifBlank { "Other applications" }
                    if (source.isWindow && (index==0 || !matches[index-1].isWindow ||
                            !matches[index-1].application.trim().ifBlank { "Other applications" }.equals(app,true))) {
                        Text(app,style=MaterialTheme.typography.labelMedium,color=MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier=Modifier.padding(start=8.dp,top=8.dp,bottom=2.dp))
                    }
                    TextButton(onClick = { onSelect(source) }, modifier = Modifier.fillMaxWidth(),
                        contentPadding=PaddingValues(start=if (source.isWindow) 16.dp else 8.dp,end=8.dp,top=6.dp,bottom=6.dp)) {
                        Row(Modifier.fillMaxWidth(),verticalAlignment=Alignment.CenterVertically) {
                            SourceIcon(source)
                            Text(source.title,modifier=Modifier.weight(1f),maxLines=2,
                                textAlign=androidx.compose.ui.text.style.TextAlign.Start,
                                overflow=androidx.compose.ui.text.style.TextOverflow.Ellipsis)
                        }
                    }
                  }
                }
                if (matches.isEmpty()) item {
                    Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.SpaceBetween,verticalAlignment=Alignment.CenterVertically) {
                        Text("No matching sources",style=MaterialTheme.typography.bodySmall)
                        TextButton(onClick={query=""}) { Text("Clear search") }
                    }
                }
            }
        }
    }, confirmButton = { TextButton(onClick = onClose) { Text("Close") } })
}
