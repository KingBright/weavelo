package `fun`.hackerlife.weavelo.ui

import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.systemGestureExclusion
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
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
    var landscapeOverride by rememberSaveable(sessionKey) { mutableStateOf<Boolean?>(null) }
    var portraitHintDismissed by rememberSaveable(sessionKey) { mutableStateOf(false) }
    StreamOrientation(active = state.panes.isNotEmpty(), landscapeOverride = landscapeOverride)
    val imeOpen = WindowInsets.ime.getBottom(LocalDensity.current) > 0
    var imeWasOpen by remember(sessionKey) { mutableStateOf(false) }
    var lastImeClosedAtMs by remember(sessionKey) { mutableLongStateOf(0L) }
    LaunchedEffect(imeOpen) {
        if (imeWasOpen && !imeOpen) {
            lastImeClosedAtMs = android.os.SystemClock.elapsedRealtime()
            actionsOpen = false
        }
        imeWasOpen = imeOpen
    }
    val typing = state.panes.isNotEmpty() && imeOpen
    var lastBackPressMs by remember(sessionKey) { mutableLongStateOf(0L) }
    val exitStreamHint = tr("Press Back again to exit stream")
    val devices by WeaveloClient.devices.collectAsState()
    val remoteDevice = devices.firstOrNull { it.id == deviceId }
    val activePane = state.panes.firstOrNull { it.id == state.selected } ?: state.panes.firstOrNull()
    val defaultDeviceName = tr("Device")
    LaunchedEffect(activePane?.id, activePane?.appearance, activePane?.source?.icon, remoteDevice?.name, defaultDeviceName) {
        (context as? android.app.Activity)?.let { activity ->
            if (activePane == null) `fun`.hackerlife.weavelo.DeviceArtwork.restore(activity)
            else `fun`.hackerlife.weavelo.DeviceArtwork.task(activity, activePane, remoteDevice?.name ?: defaultDeviceName)
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
    var localErrorText by remember { mutableStateOf<`fun`.hackerlife.weavelo.AppText?>(null) }
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
    var settingsGeneration by remember { mutableLongStateOf(0L) }
    BackHandler(enabled = foreground) {
        val now = android.os.SystemClock.elapsedRealtime()
        if (now - lastImeClosedAtMs in 0..400L) return@BackHandler
        val hasOverlay = sourcePicker || actionsOpen || selectedSettings != null || sharedFilesOpen || receivedFiles || backgroundSettings
        when {
            ZeroOcclusionHudPolicy.evaluateBackPress(hasOverlay) == BackPressAction.ToggleControls -> {
                if (state.panes.isEmpty() || now - lastBackPressMs in 1..2000L) {
                    disconnect()
                } else {
                    lastBackPressMs = now
                    android.widget.Toast.makeText(context, exitStreamHint, android.widget.Toast.LENGTH_SHORT).show()
                }
            }
            sourcePicker -> sourcePicker = false
            actionsOpen -> actionsOpen = false
            selectedSettings != null -> selectedSettings = null
            sharedFilesOpen -> sharedFilesOpen = false
            receivedFiles -> receivedFiles = false
            backgroundSettings -> backgroundSettings = false
        }
    }
    val volumes = remember { mutableStateMapOf<Int, Float>() }
    if (sourcePicker) {
        if (state.connectionGeneration != pickerGeneration || state.sessionKey != sessionKey || !state.connected) {
            LaunchedEffect(Unit) { sourcePicker = false }
        } else SourcePicker(state.sources, onClose = { sourcePicker = false }) { source ->
            localError = null; localErrorText = null
            WorkspaceClient.add(source, sessionKey, pickerGeneration)
            sourcePicker = false
        }
    }
    if (backgroundSettings) AlertDialog(onDismissRequest = { backgroundSettings = false }, title = { Text(tr("Background connection")) },
        text = { Column {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(keepService, { keepService = it }); Text(tr("Keep this connection in background until Disconnect"))
            }
            Text(tr("This is an explicit foreground service, not a battery or system-stop exemption. The idle timeout below applies when this is off."))
            OutlinedTextField(keepAlive, { keepAlive = it }, label = { Text(tr("Idle seconds (0 = always keep)")) },
                isError = keepAlive.toIntOrNull()?.let { it !in 0..86400 } ?: true)
            Row(verticalAlignment = Alignment.CenterVertically) {
                Checkbox(adaptive, { adaptive = it }); Text(tr("Adjust for battery and mobile data"))
            }
            Text(tr("Media pauses immediately. Idle connections sleep sooner on battery saver (30s), low battery (60s), or metered data (120s). File transfers finish first. Reopen to reconnect."))
        } }, confirmButton = { TextButton(onClick = {
            val seconds = keepAlive.toIntOrNull()
            if (seconds != null && seconds in 0..86400) {
                WeaveloClient.updateBackgroundKeepAlive(seconds)
                WeaveloClient.updateBackgroundServiceEnabled(keepService)
                preferences.edit().putBoolean("adaptive_background", adaptive).apply()
                backgroundSettings = false
            }
        }) { Text(tr("Save")) } })
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
            }.onFailure { withContext(Dispatchers.Main) { localError = it.message; localErrorText = `fun`.hackerlife.weavelo.appTextOf(it) } }
        }
    }
    val sendClipboardNow: () -> Unit = {
        val clip = context.getSystemService(android.content.ClipboardManager::class.java).primaryClip
        if (clip != null) scope.launch(Dispatchers.IO) {
            runCatching {
                val uris = (0 until clip.itemCount).mapNotNull { clip.getItemAt(it).uri }
                if (uris.size > 16) throw `fun`.hackerlife.weavelo.AppTextArgumentException(`fun`.hackerlife.weavelo.AppText("Send at most 16 clipboard files at a time"))
                if (uris.isEmpty()) {
                    val text = clip.getItemAt(0).text?.toString() ?: throw `fun`.hackerlife.weavelo.AppTextException(`fun`.hackerlife.weavelo.AppText("Clipboard has no transferable content"))
                    if (text.toByteArray().size > 1024 * 1024) throw `fun`.hackerlife.weavelo.AppTextArgumentException(`fun`.hackerlife.weavelo.AppText("Clipboard text is too large"))
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
                        if (bounds.outWidth <= 0 || bounds.outHeight <= 0 || bounds.outWidth.toLong() * bounds.outHeight > 16_777_216) throw `fun`.hackerlife.weavelo.AppTextArgumentException(`fun`.hackerlife.weavelo.AppText("Clipboard image is too large or unsupported"))
                        val bitmap = checkNotNull(android.graphics.BitmapFactory.decodeFile(files[0].absolutePath))
                        val png = File(directory, "clipboard.png")
                        try { png.outputStream().use { check(bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, it)) } } finally { bitmap.recycle() }
                        WorkspaceClient.sendClipboard(JSONObject().put("image", png.absolutePath).put("mime", "image/png"))
                    } else WorkspaceClient.sendClipboard(JSONObject().put("files", org.json.JSONArray(files.map { it.absolutePath })))
                }
            }.onFailure { withContext(Dispatchers.Main) { localError = it.message; localErrorText = `fun`.hackerlife.weavelo.appTextOf(it) } }
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
    LaunchedEffect(state.sessionKey, state.connected, state.connectionGeneration, state.incomingEnded) {
        if (state.sessionKey == sessionKey && state.connected && !state.incomingEnded && state.sources.isEmpty()) {
            WorkspaceClient.refreshSources(sessionKey, state.connectionGeneration)
        }
    }

    val routeBadge = remoteDevice?.scope?.trim()?.takeIf { it.isNotEmpty() }?.let { scopeName ->
        when (scopeName.uppercase(java.util.Locale.ROOT)) {
            "LAN" -> tr("LAN")
            "P2P", "DIRECT" -> tr("P2P")
            "RELAY" -> tr("Relay")
            else -> scopeName
        }
    }

    @Composable
    fun SessionActionsDropdown(expanded: Boolean, onDismiss: () -> Unit) {
        DropdownMenu(expanded = expanded, onDismissRequest = onDismiss) {
            LanguageSelector()
            if (!state.connected && !state.incomingEnded) {
                TextButton(onClick = { onDismiss(); WorkspaceClient.reconnect() }) { Text(tr("Reconnect")) }
            }
            if (state.panes.size > 1) {
                TextButton(onClick = { onDismiss(); WorkspaceClient.layout(!state.grid) }) {
                    Text(if (state.grid) tr("Use tabs") else tr("Show together"))
                }
            }
            TextButton(onClick = { onDismiss(); WorkspaceClient.refreshSources(sessionKey, state.connectionGeneration) },
                enabled = state.connected && !state.incomingEnded) { Text(tr("Refresh screens and windows")) }
            TextButton(onClick = { onDismiss(); pickerSessionKey = sessionKey; shareSelection = false; filePicker.launch(arrayOf("*/*")) },
                enabled = state.connected && state.files) { Text(tr("Send file")) }
            TextButton(onClick = { onDismiss(); pickerSessionKey = sessionKey; shareSelection = true; filePicker.launch(arrayOf("*/*")) },
                enabled = state.connected) { Text(tr("Share file with network")) }
            TextButton(onClick = { onDismiss(); WorkspaceClient.clearSharedFiles() },
                enabled = state.connected) { Text(tr("Clear my shared list")) }
            TextButton(onClick = { onDismiss(); sharedFilesOpen = true; WorkspaceClient.listSharedFiles() },
                enabled = state.connected && state.files && !state.sharedBusy) { Text(tr("Retrieve shared files")) }
            TextButton(onClick = { onDismiss(); receivedFiles = true }) { Text(tr("Received files")) }
            TextButton(onClick = { onDismiss(); backgroundSettings = true }) { Text(tr("Background")) }
            TextButton(onClick = { onDismiss(); WorkspaceClient.enableClipboard(!state.clipboard) },
                enabled = state.connected && state.clipboardAvailable) {
                Text(if (state.clipboard) tr("Clipboard ✓") else tr("Clipboard"))
            }
            TextButton(onClick = { onDismiss(); sendClipboardNow() },
                enabled = state.connected && state.clipboard) { Text(tr("Send clipboard")) }
            TextButton(onClick = { onDismiss(); disconnect() }) { Text(tr("Disconnect")) }
        }
    }

    WeaveloTheme {
        Box(Modifier.fillMaxSize().background(if (state.panes.isEmpty()) ColorBaseBg else Color.Black)) {
            Column(Modifier.fillMaxSize().imePadding()) {
                val visible = if (state.grid) state.panes else state.panes.filter { it.id == state.selected }
                if (visible.isNotEmpty()) BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
                    val portraitViewport = maxHeight > maxWidth
                    val columns = workspaceGridColumns(maxWidth.value, maxHeight.value,
                        visible.map { it.source.width.toFloat() / it.source.height.coerceAtLeast(1) })
                    val rows = visible.chunked(columns)
                    Column(Modifier.fillMaxSize()) {
                        rows.forEach { row ->
                            Row(Modifier.weight(1f).fillMaxWidth()) {
                                row.forEach { pane ->
                                    key(pane.id, state.connectionGeneration) {
                                        Box(Modifier.weight(1f).fillMaxHeight().padding(if (visible.size > 1) 2.dp else 0.dp)) {
                                            if (foreground && state.connected) {
                                                WorkspaceVideo(
                                                    pane = pane,
                                                    allPanes = state.panes,
                                                    gridMode = state.grid,
                                                    selectedPaneId = state.selected,
                                                    deviceName = remoteDevice?.name ?: tr("Remote device"),
                                                    routeBadge = routeBadge,
                                                    sessionKey = state.sessionKey,
                                                    generation = state.connectionGeneration,
                                                    portraitViewport = portraitViewport,
                                                    landscapeOverride = landscapeOverride,
                                                    portraitHintDismissed = portraitHintDismissed,
                                                    onDismissPortraitHint = { portraitHintDismissed = true },
                                                    onToggleLandscape = {
                                                        landscapeOverride = if (landscapeOverride == true) null else true
                                                    },
                                                    onPickSource = {
                                                        WorkspaceClient.refreshSources(sessionKey, state.connectionGeneration)
                                                        pickerGeneration = state.connectionGeneration
                                                        sourcePicker = true
                                                    },
                                                    onOpenSettings = {
                                                        selectedSettings = pane
                                                        settingsGeneration = state.connectionGeneration
                                                    },
                                                    onDisconnect = disconnect,
                                                    mobileTarget = ZeroOcclusionHudPolicy.isMobileTarget(
                                                        state.androidHost, pane.source.isCamera),
                                                    isPhoneScreen = ZeroOcclusionHudPolicy.supportsRemoteHome(
                                                        state.androidHost, pane.source.isCamera, pane.source.source.toString()),
                                                    canRemoteBack = ZeroOcclusionHudPolicy.supportsRemoteBack(
                                                        state.androidHost, pane.source.isCamera,
                                                        pane.source.source.toString(), pane.source.isWindow),
                                                    sessionActionsContent = { open, onDismiss ->
                                                        SessionActionsDropdown(open, onDismiss)
                                                    },
                                                    onError = { message, text ->
                                                        localError = message
                                                        localErrorText = text
                                                    }
                                                )
                                            }
                                            NetworkStatusChip(
                                                if (!state.connected) SessionState.DISCONNECTED else if (pane.accepted) SessionState.STREAMING else SessionState.CONNECTING,
                                                pane.telemetry, route = remoteDevice?.scope,
                                                modifier = Modifier.align(Alignment.BottomEnd).safeDrawingPadding().padding(8.dp)
                                            )
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Only show a compact floating notice if disconnected or an error occurred during streaming.
                    val streamIssue = localError ?: if (!state.connected) state.status else null
                    if (!streamIssue.isNullOrBlank() && !typing) {
                        Surface(
                            modifier = Modifier.align(Alignment.TopCenter).safeDrawingPadding().padding(8.dp),
                            shape = RoundedCornerShape(18.dp),
                            color = ColorSurfaceCard.copy(alpha = .94f),
                            shadowElevation = 4.dp
                        ) {
                            Row(
                                modifier = Modifier.padding(horizontal = 10.dp, vertical = 4.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(6.dp)
                            ) {
                                Text(
                                    text = if (localError != null && localErrorText != null) appText(localErrorText!!)
                                        else if (localError == null && state.statusText != null) appText(state.statusText!!)
                                        else trFormat("Details: %s", streamIssue),
                                    color = ColorTextPrimary,
                                    style = MaterialTheme.typography.labelMedium,
                                    maxLines = 2,
                                    modifier = Modifier.widthIn(max = 240.dp)
                                )
                                if (!state.connected && !state.incomingEnded) {
                                    TextButton(
                                        onClick = { WorkspaceClient.reconnect() },
                                        contentPadding = PaddingValues(horizontal = 6.dp, vertical = 2.dp)
                                    ) { Text(tr("Reconnect"), style = MaterialTheme.typography.labelMedium) }
                                }
                                if (localError != null) {
                                    MobileAction(MobileSymbol.Close, tr("Close"), modifier = Modifier.size(28.dp)) {
                                        localError = null; localErrorText = null
                                    }
                                } else {
                                    MobileAction(MobileSymbol.Close, tr("Exit stream"), modifier = Modifier.size(28.dp), onClick = disconnect)
                                }
                            }
                        }
                    }
                }
                if (state.panes.isEmpty()) Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.Center) {
                    val quickScreen = remember(state.sources) { primaryScreenSource(state.sources) }
                    val loadingCatalog = !state.incomingEnded && (!state.connected || !state.sourcesLoaded)
                    val quickSources = remember(state.sources) {
                        val hasDisplay = state.sources.any { (it.source as? JSONObject)?.has("Display") == true }
                        state.sources.filter { !(hasDisplay && it.source == "MainDisplay") }
                            .sortedBy { if (it.isCamera) 2 else if (it.isWindow) 1 else 0 }
                            .take(4)
                    }
                    Column(horizontalAlignment = Alignment.CenterHorizontally,
                        verticalArrangement = Arrangement.spacedBy(12.dp), modifier = Modifier.padding(24.dp)) {
                        if (loadingCatalog) {
                            CircularProgressIndicator(modifier = Modifier.size(36.dp), strokeWidth = 3.dp, color = ColorAccentCyan)
                        } else {
                            Icon(MobileSymbol.Sources.image, null, tint = ColorAccentEmerald, modifier = Modifier.size(40.dp))
                        }
                        Text(when {
                            state.incomingEnded -> tr("Sharing ended")
                            loadingCatalog -> tr("Connecting…")
                            else -> tr("Choose a screen")
                        }, color = ColorTextPrimary, style = MaterialTheme.typography.titleLarge)
                        Text(when {
                            state.incomingEnded -> tr("You can still exchange files.")
                            loadingCatalog -> tr("Connecting and loading shared screens…")
                            state.sources.isEmpty() -> tr("This device has not shared a screen yet. You can still exchange files.")
                            else -> tr("Open a display or application window. File transfers stay independent.")
                        }, color = ColorTextSecondary, style = MaterialTheme.typography.bodyMedium,
                            textAlign = androidx.compose.ui.text.style.TextAlign.Center)
                        if (!state.incomingEnded && !loadingCatalog) {
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                                if (quickScreen != null) {
                                    Button(onClick = {
                                        localError = null; localErrorText = null
                                        WorkspaceClient.add(quickScreen, sessionKey, state.connectionGeneration)
                                    }, enabled = state.connected) {
                                        Text(tr("Open main screen"))
                                    }
                                }
                                OutlinedButton(onClick = {
                                    WorkspaceClient.refreshSources(sessionKey, state.connectionGeneration)
                                    pickerGeneration = state.connectionGeneration; sourcePicker = true
                                }, enabled = state.connected) {
                                    Text(tr("Browse screens and windows"))
                                }
                            }
                            if (quickSources.isNotEmpty()) {
                                Column(Modifier.widthIn(max = 420.dp).fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                                    quickSources.forEach { source ->
                                        OutlinedButton(onClick = {
                                            localError = null; localErrorText = null
                                            WorkspaceClient.add(source, sessionKey, state.connectionGeneration)
                                        }, enabled = state.connected, modifier = Modifier.fillMaxWidth()) {
                                            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                                                SourceIcon(source)
                                                Text(localizedSourceTitle(source), modifier = Modifier.weight(1f), maxLines = 1,
                                                    overflow = TextOverflow.Ellipsis)
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if (foreground && state.connected) visible.mapNotNull { it.audioOwner }.distinct().forEach { owner ->
                    key(owner) { WorkspaceAudio(owner, volumes[owner] ?: 1f, onError = { message, text -> localError = message; localErrorText = text }) }
                }
            }
            // Top header ONLY when no stream pane is open (pre-stream selection page).
            // During active streaming, zero top bars are ever shown.
            if (state.panes.isEmpty()) {
                Column(
                    Modifier.align(Alignment.TopCenter).fillMaxWidth().safeDrawingPadding().padding(6.dp)
                        .background(ColorSurfaceCard.copy(alpha = .92f), RoundedCornerShape(18.dp))
                ) {
                    Row(Modifier.fillMaxWidth().heightIn(min = 52.dp), verticalAlignment = Alignment.CenterVertically) {
                        MobileAction(MobileSymbol.Back, tr("Exit stream"), onClick = disconnect)
                        Column(Modifier.weight(1f).padding(horizontal = 4.dp)) {
                            Text(remoteDevice?.name ?: tr("Remote device"),
                                color = ColorTextPrimary, fontWeight = FontWeight.SemiBold,
                                maxLines = 1, overflow = TextOverflow.Ellipsis)
                            val subStatus = when {
                                state.incomingEnded -> tr("Sharing ended")
                                state.connected && routeBadge != null -> "${tr("Connected")} · $routeBadge"
                                state.connected -> tr("Connected")
                                else -> tr("Connecting…")
                            }
                            Text(subStatus, color = ColorTextSecondary, style = MaterialTheme.typography.labelSmall)
                        }
                        MobileAction(MobileSymbol.Sources, tr("Choose screen or app window"), state.connected && !state.incomingEnded) {
                            WorkspaceClient.refreshSources(sessionKey, state.connectionGeneration)
                            pickerGeneration = state.connectionGeneration; sourcePicker = true
                        }
                        MobileAction(MobileSymbol.Refresh, tr("Refresh screens and windows"), state.connected && !state.incomingEnded) {
                            WorkspaceClient.refreshSources(sessionKey, state.connectionGeneration)
                        }
                        Box {
                            MobileAction(MobileSymbol.More, tr("Connection actions")) { actionsOpen = true }
                            SessionActionsDropdown(actionsOpen) { actionsOpen = false }
                        }
                    }
                    (localError ?: state.status.takeUnless { it == "Connected" })?.takeIf { it.isNotBlank() }?.let {
                        Text(
                            if (localError != null && localErrorText != null) appText(localErrorText!!)
                            else if (localError == null && state.statusText != null) appText(state.statusText!!)
                            else trFormat("Details: %s", it),
                            color = ColorTextSecondary,
                            maxLines = 2,
                            modifier = Modifier.padding(horizontal = 12.dp, vertical = 4.dp)
                        )
                    }
                }
            }
        }
        selectedSettings?.let { selectedPane ->
            val pane = state.panes.firstOrNull { it.id == selectedPane.id && state.connectionGeneration == settingsGeneration }
                ?: selectedPane
            val settingsCurrent = state.connected && state.connectionGeneration == settingsGeneration &&
                state.panes.any { it.id == pane.id && it.accepted } && (state.grid || state.selected == pane.id)
            var fps by remember(pane.id) { mutableStateOf(pane.fps.toString()) }
            var kbps by remember(pane.id) { mutableStateOf(pane.kbps.toString()) }
            AlertDialog(onDismissRequest = { selectedSettings = null }, title = { Text(tr("Stream settings")) },
                text = { Column(Modifier.verticalScroll(rememberScrollState())) {
                    OutlinedTextField(fps, { fps = it }, label = { Text(tr("Frames per second")) })
                    OutlinedTextField(kbps, { kbps = it }, label = { Text(tr("Bitrate (kbps)")) })
                    pane.audioOwner?.let { owner ->
                        Text(tr("Volume"))
                        Slider(value = volumes[owner] ?: 1f, onValueChange = { volumes[owner] = it })
                    }
                    if (pane.source.isDisplay && !state.androidHost) {
                        val output = pane.audioOutput
                        Text(tr("Sound on controlled device"))
                        Text(tr(when {
                            output.pendingRevision != null -> "Changing host audio output…"
                            output.localPlayback == "Enabled" -> "Host sound is on"
                            output.localPlayback == "Disabled" -> "Host sound is off"
                            output.localPlayback == "Partial" -> "Host sound is partially on"
                            else -> "Host sound state is unknown"
                        }))
                        output.messageText?.let { Text(appText(it)) }
                        output.message?.takeIf { it.isNotBlank() }?.let { Text(tr(it)) }
                        if (output.privateSupported == false) Text(tr("Private output is unavailable on this host"))
                        Row {
                            TextButton(onClick = { WorkspaceClient.setAudioOutput(pane.id, sessionKey, settingsGeneration, "Mirror") },
                                enabled = settingsCurrent && output.pendingRevision == null && output.privateSupported != false) { Text(tr("Play on both")) }
                            TextButton(onClick = { WorkspaceClient.setAudioOutput(pane.id, sessionKey, settingsGeneration, "Private") },
                                enabled = settingsCurrent && output.pendingRevision == null && output.privateSupported != false) { Text(tr("Remote only")) }
                        }
                    }
                } },
                confirmButton = { TextButton(onClick = {
                    val f = fps.toIntOrNull(); val k = kbps.toIntOrNull()
                    if (f != null && k != null && f > 0 && k > 0) { WorkspaceClient.settings(pane.id, sessionKey, settingsGeneration, fps = f, kbps = k); selectedSettings = null }
                }) { Text(tr("Apply")) } })
        }
        if (sharedFilesOpen) AlertDialog(onDismissRequest = { sharedFilesOpen = false }, title = { Text(tr("Remote shared files")) },
            text = { Column {
                Text(state.status)
                Text(tr("Only explicitly shared files are listed. No remote filesystem paths are accepted."))
                state.remoteFiles.forEach { file ->
                    TextButton(onClick = { WorkspaceClient.fetchSharedFile(file.id, sessionKey) }, enabled = state.connected && !state.sharedBusy) { Text(trFormat("Get %s (%d bytes)", file.name, file.sizeBytes)) }
                }
                Row {
                    TextButton(onClick = { WorkspaceClient.listSharedFiles() }, enabled = state.connected && !state.sharedBusy) { Text(tr("Refresh")) }
                    state.sharedNext?.let { next -> TextButton(onClick = { WorkspaceClient.listSharedFiles(next) }, enabled = state.connected && !state.sharedBusy) { Text(tr("Next page")) } }
                }
            } }, confirmButton = { TextButton(onClick = { sharedFilesOpen = false }) { Text(tr("Close")) } })
        if (receivedFiles) ReceivedFilesDialog(onSendToDevice = if (state.connected && state.files) { file ->
            receivedFiles = false
            WorkspaceClient.sendFile(file, sessionKey)
        } else null) { receivedFiles = false }
    }
}

@Composable
private fun WorkspaceAudio(owner: Int, volume: Float, onError: (String, `fun`.hackerlife.weavelo.AppText?) -> Unit) {
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
            } catch (e: Exception) { if (e !is CancellationException) withContext(Dispatchers.Main) { onError(e.message ?: "Audio failed", if (e.message == null) `fun`.hackerlife.weavelo.AppText("Audio failed") else null) } }
            finally { player.stop() }
        }
    }
}

@Composable
private fun WorkspaceVideo(
    pane: WorkspacePane,
    allPanes: List<WorkspacePane>,
    gridMode: Boolean,
    selectedPaneId: Int,
    deviceName: String,
    routeBadge: String?,
    sessionKey: String,
    generation: Long,
    portraitViewport: Boolean,
    landscapeOverride: Boolean?,
    portraitHintDismissed: Boolean,
    onDismissPortraitHint: () -> Unit,
    onToggleLandscape: () -> Unit,
    onPickSource: () -> Unit,
    onOpenSettings: () -> Unit,
    onDisconnect: () -> Unit,
    mobileTarget: Boolean,
    isPhoneScreen: Boolean,
    canRemoteBack: Boolean,
    sessionActionsContent: @Composable (Boolean, () -> Unit) -> Unit,
    onError: (String, `fun`.hackerlife.weavelo.AppText?) -> Unit
) {
    pane.videoError?.let { reason ->
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            Column(Modifier.padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Text(pane.videoErrorText?.let { appText(it) } ?: trFormat("Details: %s", reason), color = ColorTextSecondary, style = MaterialTheme.typography.bodyMedium,
                    textAlign = androidx.compose.ui.text.style.TextAlign.Center)
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    TextButton(onClick = { WorkspaceClient.retry(pane.id, sessionKey, generation) }) { Text(tr("Retry this source")) }
                    TextButton(onClick = onPickSource) { Text(tr("Switch screen")) }
                }
            }
        }
        return
    }
    if (pane.source.isCamera && !pane.accepted) {
        Box(Modifier.fillMaxSize(),contentAlignment=Alignment.Center) { Text(tr("Updating camera…"),color=ColorTextSecondary) }
        return
    }
    var dimensions by remember(pane.id, sessionKey, generation) {
        mutableStateOf(VideoDimensions(pane.source.width.coerceAtLeast(1), pane.source.height.coerceAtLeast(1)))
    }
    val transform = remember(pane.id, sessionKey, generation) { mutableStateOf(ViewportTransform(maxScale = 6.0f)) }
    var baseMeasured by remember { mutableStateOf(androidx.compose.ui.unit.IntSize.Zero) }
    var containerMeasured by remember { mutableStateOf(androidx.compose.ui.unit.IntSize.Zero) }
    var surfaceToken by remember { mutableLongStateOf(0L) }
    val token = surfaceToken
    var keyboardBinding by remember(pane.id,sessionKey,generation,token) { mutableStateOf<String?>(null) }
    var keyboardVisible by remember(pane.id,sessionKey,generation,token) { mutableStateOf(false) }
    val focusGate = remember(pane.id,sessionKey,generation,token) { EditorFocusGate<Pair<JSONObject,Long>>() }
    DisposableEffect(focusGate) { onDispose { focusGate.invalidate(); WorkspaceClient.invalidateEditorFocus() } }

    var handleFraction by rememberSaveable(pane.id, sessionKey, generation) { mutableFloatStateOf(0.5f) }
    var sideDrawerOpen by remember(pane.id, sessionKey, generation) { mutableStateOf(false) }
    var sideMoreOpen by remember(pane.id, sessionKey, generation) { mutableStateOf(false) }
    var sideInteraction by remember(pane.id, sessionKey, generation) { mutableLongStateOf(0L) }

    LaunchedEffect(pane.id, sessionKey, generation, token, pane.input, pane.accepted) {
        WorkspaceClient.editorFocus.collect { result ->
            if (result.id == pane.id && pane.input && pane.accepted && token != 0L) {
                val candidate = focusGate.take(result.sequence, android.os.SystemClock.uptimeMillis())
                if (candidate != null && candidate.second == WorkspaceClient.editorFocusIntent()) {
                    val owner = WorkspaceClient.keyboardBinding(pane.id,sessionKey,generation,token,result.binding,candidate.first)
                    if (owner != null) {
                        sideDrawerOpen = false; sideMoreOpen = false
                        keyboardBinding = owner; keyboardVisible = true
                    }
                }
            }
        }
    }

    val imeVisible = WindowInsets.ime.getBottom(LocalDensity.current) > 0
    var imeWasVisible by remember(pane.id, sessionKey, generation, token) { mutableStateOf(false) }
    LaunchedEffect(imeVisible) {
        if (imeVisible) {
            sideDrawerOpen = false
            sideMoreOpen = false
        } else if (imeWasVisible) {
            // Closing the keyboard returns cleanly to full-screen video without popping any bars.
            focusGate.invalidate()
            WorkspaceClient.invalidateEditorFocus()
            keyboardVisible = false
            sideDrawerOpen = false
            sideMoreOpen = false
        }
        imeWasVisible = imeVisible
    }
    LaunchedEffect(sideDrawerOpen, sideMoreOpen, keyboardVisible, sideInteraction) {
        if (sideDrawerOpen && !sideMoreOpen && !keyboardVisible) {
            kotlinx.coroutines.delay(5000)
            sideDrawerOpen = false
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
            onError("Keyboard control changed. Reopen the keyboard and select shortcuts again.", `fun`.hackerlife.weavelo.AppText("Keyboard control changed. Reopen the keyboard and select shortcuts again."))
        } else if (imeVisible && heldModifiers.isNotEmpty() && pane.input && token != 0L && heldOwner != null) {
            while (isActive) {
                delay(100)
                WorkspaceClient.keyboardAlive(pane.id, sessionKey, generation, token, heldOwner)
            }
        }
    }
    BackHandler(enabled = keyboardVisible || sideMoreOpen || sideDrawerOpen) {
        focusGate.invalidate(); WorkspaceClient.invalidateEditorFocus()
        when {
            sideMoreOpen -> sideMoreOpen = false
            keyboardVisible -> { keyboardVisible = false; sideDrawerOpen = false }
            else -> sideDrawerOpen = false
        }
    }
    var videoStatus by remember(pane.id, sessionKey, generation) { mutableStateOf<String?>("Waiting for video…") }
    val pointers = remember { mutableSetOf<Int>() }
    var touching by remember { mutableStateOf(false) }
    val defaultControlMode = if (pane.maxTouchPoints > 0) TouchMode.NATIVE else TouchMode.DIRECT
    var touchMode by remember(pane.id, sessionKey, generation, pane.maxTouchPoints, pane.input) {
        mutableStateOf(if (!pane.input) TouchMode.VIEW else defaultControlMode)
    }
    var lastControlMode by remember(pane.id, sessionKey, generation, pane.maxTouchPoints) {
        mutableStateOf(defaultControlMode)
    }
    var cursor by remember(pane.id, sessionKey, generation) { mutableStateOf(Offset(.5f, .5f)) }
    val currentError by rememberUpdatedState(onError)
    val nativeViewport = remember(pane.id, sessionKey, generation) { java.util.concurrent.atomic.AtomicReference<GpuVideoSurface?>() }
    LaunchedEffect(baseMeasured, dimensions, pane.accepted, keyboardVisible, imeVisible) {
        if (!pane.accepted || keyboardVisible || imeVisible) return@LaunchedEffect
        delay(250)
        val quarterTurn = pane.source.rotation % 180 != 0
        (if (!pane.source.isCamera) MediaCodecPlayer.screenSize(dimensions.width,dimensions.height) else VideoDimensions.fitEncoder(dimensions.width, dimensions.height,
            if (pane.source.isCamera) (if (quarterTurn) baseMeasured.height else baseMeasured.width).coerceAtMost(4096) else 4096,
            if (pane.source.isCamera) (if (quarterTurn) baseMeasured.width else baseMeasured.height).coerceAtMost(4096) else 4096, 2, 2))
            ?.takeIf { it.width >= 32 && it.height >= 32 }?.let {
                WorkspaceClient.settings(pane.id, sessionKey, generation, width = it.width, height = it.height)
            }
    }
    LaunchedEffect(pane.id, sessionKey, generation, pane.accepted) {
        if (!pane.accepted || pane.source.isCamera) return@LaunchedEffect
        var previous = emptyList<Int>()
        while (isActive) {
            val base = baseMeasured
            val container = if (containerMeasured.width >= 16 && containerMeasured.height >= 16) containerMeasured else base
            if (base.width >= 16 && base.height >= 16) {
                val (visW, visH) = transform.value.expandedViewportSize(
                    base.width.toFloat(), base.height.toFloat(),
                    container.width.toFloat(), container.height.toFloat()
                )
                val rect = transform.value.visibleRect(
                    base.width.toFloat(), base.height.toFloat(),
                    container.width.toFloat(), container.height.toFloat()
                )
                val reqW = visW.toInt().coerceIn(16, 8192)
                val reqH = visH.toInt().coerceIn(16, 8192)
                val next = rect.toList() + listOf(reqW, reqH)
                if (next != previous) {
                    WorkspaceClient.viewport(pane.id, sessionKey, generation, rect, reqW, reqH)
                    previous = next
                }
            }
            delay(50)
        }
    }
    LaunchedEffect(touching, surfaceToken) {
        val token = surfaceToken
        if (touching && token != 0L) while (isActive) {
            WorkspaceClient.surface(pane.id, sessionKey, generation, token, "alive")
            delay(100)
        }
    }
    LaunchedEffect(baseMeasured, dimensions, sideDrawerOpen, sideMoreOpen, touchMode) { focusGate.invalidate() }
    val keyboardOwner = keyboardBinding
    RemoteKeyboard(identity = listOf(pane.id,sessionKey,generation,token,keyboardOwner), visible = keyboardVisible,
        enabled = pane.input && token != 0L && keyboardOwner != null,
        onText = { text -> keyboardOwner?.let { WorkspaceClient.keyboard(pane.id,sessionKey,generation,token,it,text=text) } },
        onKey = { key,pressed -> keyboardOwner?.let { WorkspaceClient.keyboard(pane.id,sessionKey,generation,token,it,key=key,pressed=pressed) } })
    Column(Modifier.fillMaxSize()) {
      BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
        val viewportHeightPx = with(LocalDensity.current) { maxHeight.toPx().coerceAtLeast(1f) }
        LaunchedEffect(maxWidth, maxHeight) {
            sideMoreOpen = false
            sideDrawerOpen = false
        }
        RemoteViewport(
            dimensions = dimensions, transformState = transform, description = tr("Remote screen"),
            rotation = pane.source.rotation,
            expandToContainer = true,
            enabled = pane.input && token != 0L,
            touchMode = touchMode,
            contentKey = token to dimensions,
            modifier = Modifier.fillMaxSize(),
            onViewportSizeChanged = { baseMeasured = it },
            onContainerSizeChanged = { containerMeasured = it },
            sendTouch = { action, id, x, y, pressure, eventTime ->
                if (action == 0) {
                    focusGate.invalidate(); WorkspaceClient.invalidateEditorFocus()
                    if (sideDrawerOpen && !sideMoreOpen) sideDrawerOpen = false
                }
                val gestureIntent = focusGate.intent()
                val globalIntent = WorkspaceClient.editorFocusIntent()
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
                        .put("pointer_id", id).put("normalized_x", x).put("normalized_y", y).put("pressure", pressure),geometry,
                    onProbe = { sequence, geometry -> if (surfaceToken == token) focusGate.record(gestureIntent,sequence,eventTime,geometry to globalIntent) }) { cx, cy ->
                    if (surfaceToken == token) {
                        val nextCursor = Offset(cx, cy)
                        val cursorMoved = (nextCursor - cursor).getDistance() > 0.0005f
                        cursor = nextCursor
                        if (cursorMoved && touchMode == TouchMode.TRACKPAD && transform.value.scale > 1f) {
                            val bw = baseMeasured.width.coerceAtLeast(1).toFloat()
                            val bh = baseMeasured.height.coerceAtLeast(1).toFloat()
                            val cw = containerMeasured.width.coerceAtLeast(baseMeasured.width).coerceAtLeast(1).toFloat()
                            val ch = containerMeasured.height.coerceAtLeast(baseMeasured.height).coerceAtLeast(1).toFloat()
                            transform.value = transform.value.reveal(cx, cy, bw, bh, cw, ch)
                        }
                    }
                }
                }
            },
            sendTouchFrame = { sample ->
                if (sideDrawerOpen && !sideMoreOpen) sideDrawerOpen = false
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
                    Canvas(Modifier.fillMaxSize()) {
                        val bw = baseMeasured.width.coerceAtLeast(1).toFloat()
                        val bh = baseMeasured.height.coerceAtLeast(1).toFloat()
                        val (x, y) = transform.value.normalizedToScreen(
                            cursor.x, cursor.y, bw, bh, size.width, size.height
                        )
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
                (videoStatus ?: pane.inputError)?.let { message ->
                    Text(if (videoStatus != null) tr(message) else pane.inputErrorText?.let { appText(it) } ?: trFormat("Details: %s", message), color = ColorTextPrimary, style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.align(Alignment.TopCenter).padding(12.dp)
                            .background(ColorSurfaceCard.copy(alpha = .94f), RoundedCornerShape(8.dp))
                            .padding(horizontal = 12.dp, vertical = 8.dp))
                }
            },
            factory = { context ->
                SurfaceView(context).apply {
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
                                    val size=if (target != null) MediaCodecPlayer.screenSize(pane.source.width,pane.source.height) ?: `fun`.hackerlife.weavelo.appError("No supported native screen decoder size")
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
                                } catch (e: Exception) { if (running.get()) post { currentError(e.message ?: "Video decoder failed", `fun`.hackerlife.weavelo.appTextOf(e) ?: if (e.message == null) `fun`.hackerlife.weavelo.AppText("Video decoder failed") else null) } }
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

        // Portrait quick helper at 1.0x: 1-tap Fill Height (纵向铺满) or Rotate to Landscape
        if (portraitViewport && landscapeOverride == null && !portraitHintDismissed && !imeVisible &&
            !mobileTarget && pane.source.width > pane.source.height && transform.value.scale <= 1.05f) {
            Surface(
                modifier = Modifier.align(Alignment.BottomStart).safeDrawingPadding().padding(8.dp),
                shape = RoundedCornerShape(50),
                color = ColorSurfaceCard.copy(alpha = .9f),
                shadowElevation = 4.dp
            ) {
                Row(
                    modifier = Modifier.padding(start = 6.dp, end = 2.dp, top = 2.dp, bottom = 2.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    TextButton(
                        onClick = {
                            val bw = baseMeasured.width.coerceAtLeast(1).toFloat()
                            val bh = baseMeasured.height.coerceAtLeast(1).toFloat()
                            val cw = containerMeasured.width.coerceAtLeast(baseMeasured.width).coerceAtLeast(1).toFloat()
                            val ch = containerMeasured.height.coerceAtLeast(baseMeasured.height).coerceAtLeast(1).toFloat()
                            transform.value = transform.value.fillHeight(bw, bh, cw, ch)
                        },
                        contentPadding = PaddingValues(horizontal = 6.dp, vertical = 2.dp)
                    ) {
                        Icon(MobileSymbol.ExpandVertical.image, contentDescription = null, modifier = Modifier.size(15.dp))
                        Spacer(Modifier.width(4.dp))
                        Text(tr("Fill height"), style = MaterialTheme.typography.labelMedium)
                    }
                    TextButton(
                        onClick = onToggleLandscape,
                        contentPadding = PaddingValues(horizontal = 6.dp, vertical = 2.dp)
                    ) {
                        Icon(MobileSymbol.Rotate.image, contentDescription = null, modifier = Modifier.size(15.dp))
                        Spacer(Modifier.width(4.dp))
                        Text(tr("Landscape view"), style = MaterialTheme.typography.labelMedium)
                    }
                    MobileAction(MobileSymbol.Close, tr("Close"), modifier = Modifier.size(28.dp), onClick = onDismissPortraitHint)
                }
            }
        }

        // Zoomed floating pill (> 1.05x): 1-tap Pan Canvas toggle or Reset Zoom
        if (!imeVisible && transform.value.scale > 1.05f) {
            ZoomedCanvasChip(
                scale = transform.value.scale,
                panActive = touchMode == TouchMode.VIEW,
                canTogglePan = pane.input && token != 0L,
                modifier = Modifier.align(Alignment.TopStart).safeDrawingPadding().padding(8.dp),
                onTogglePan = {
                    focusGate.invalidate(); WorkspaceClient.invalidateEditorFocus()
                    WorkspaceClient.surface(pane.id, sessionKey, generation, token, "cancel")
                    pointers.clear(); touching = false
                    if (touchMode == TouchMode.VIEW) {
                        touchMode = lastControlMode
                    } else {
                        lastControlMode = touchMode
                        touchMode = TouchMode.VIEW
                    }
                },
                onResetZoom = {
                    WorkspaceClient.surface(pane.id, sessionKey, generation, token, "cancel")
                    pointers.clear(); touching = false
                    transform.value = transform.value.reset()
                    if (pane.input && touchMode == TouchMode.VIEW) touchMode = lastControlMode
                }
            )
        }

        // Subtle right-edge handle + deliberate-intent compact right-side drawer (zero top bars!)
        if (!imeVisible) {
            val pillAlpha = ZeroOcclusionHudPolicy.edgePillAlpha(touching = touching, imeVisible = imeVisible)
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
                            pointers.clear(); touching = false
                            sideDrawerOpen = true; sideInteraction++
                        }
                    }) { change, drag ->
                        change.consume(); horizontal += drag.x
                        handleFraction = ZeroOcclusionHudPolicy.dragEdgePillFraction(handleFraction, drag.y, viewportHeightPx)
                    }
                }
                .clickable(onClickLabel = tr("Show remote controls")) {
                    focusGate.invalidate(); WorkspaceClient.invalidateEditorFocus()
                    WorkspaceClient.surface(pane.id, sessionKey, generation, token, "cancel")
                    pointers.clear(); touching = false
                    sideDrawerOpen = !sideDrawerOpen
                    if (!sideDrawerOpen) sideMoreOpen = false else sideInteraction++
                }
                .padding(start = 36.dp, end = 2.dp, top = 10.dp, bottom = 10.dp)
                .background(ColorAccentCyan.copy(alpha = pillAlpha), RoundedCornerShape(6.dp)))

            if (sideDrawerOpen) {
                val paneTitle = localizedSourceTitle(pane.source)
                val inputReady = pane.input && token != 0L
                Surface(
                    modifier = Modifier
                        .align(Alignment.CenterEnd)
                        .safeDrawingPadding()
                        .padding(end = 42.dp, top = 8.dp, bottom = 8.dp)
                        .widthIn(min = 220.dp, max = 276.dp)
                        .pointerInput(pane.id, sessionKey, generation) {
                            awaitPointerEventScope {
                                while (true) {
                                    if (awaitPointerEvent(PointerEventPass.Initial).type == PointerEventType.Press) {
                                        sideInteraction++
                                    }
                                }
                            }
                        },
                    shape = RoundedCornerShape(20.dp),
                    color = ColorSurfaceCard.copy(alpha = .95f),
                    shadowElevation = 8.dp
                ) {
                    Column(
                        modifier = Modifier.padding(horizontal = 10.dp, vertical = 8.dp),
                        verticalArrangement = Arrangement.spacedBy(6.dp)
                    ) {
                        // Row 1: Compact device header + more session menu + exit stream
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Column(Modifier.weight(1f)) {
                                Text(
                                    text = deviceName,
                                    color = ColorTextPrimary,
                                    style = MaterialTheme.typography.labelLarge,
                                    fontWeight = FontWeight.SemiBold,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis
                                )
                                if (routeBadge != null) {
                                    Text(
                                        text = routeBadge,
                                        color = ColorAccentEmerald,
                                        style = MaterialTheme.typography.labelSmall,
                                        maxLines = 1
                                    )
                                }
                            }
                            Box {
                                MobileAction(
                                    symbol = MobileSymbol.More,
                                    label = tr("Connection actions"),
                                    modifier = Modifier.size(36.dp)
                                ) { sideMoreOpen = true }
                                sessionActionsContent(sideMoreOpen) { sideMoreOpen = false }
                            }
                            MobileAction(
                                symbol = MobileSymbol.Back,
                                label = tr("Exit stream"),
                                modifier = Modifier.size(36.dp),
                                onClick = onDisconnect
                            )
                        }

                        // Row 2: Current source / window selector & stream settings (tucked inside side drawer)
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .clip(RoundedCornerShape(12.dp))
                                .background(Color(0xFFF1F5F9))
                                .padding(horizontal = 6.dp, vertical = 2.dp),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            TextButton(
                                onClick = { sideDrawerOpen = false; onOpenSettings() },
                                contentPadding = PaddingValues(horizontal = 4.dp, vertical = 2.dp),
                                modifier = Modifier.weight(1f)
                            ) {
                                SourceIcon(pane.source.copy(icon = pane.appearance?.application ?: pane.source.icon))
                                Text(
                                    text = paneTitle,
                                    color = ColorTextPrimary,
                                    style = MaterialTheme.typography.labelMedium,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis
                                )
                            }
                            if (WorkspaceClient.cameraSwitchTarget(pane) != null) {
                                MobileAction(
                                    symbol = MobileSymbol.CameraSwitch,
                                    label = tr("Switch camera"),
                                    modifier = Modifier.size(32.dp)
                                ) {
                                    WorkspaceClient.switchCamera(pane.id, sessionKey, generation)
                                }
                            }
                            MobileAction(
                                symbol = MobileSymbol.Sources,
                                label = tr("Choose screen or app window"),
                                modifier = Modifier.size(32.dp)
                            ) {
                                sideDrawerOpen = false
                                onPickSource()
                            }
                            if (allPanes.size > 1) {
                                MobileAction(
                                    symbol = MobileSymbol.Close,
                                    label = trFormat("Close %s", paneTitle),
                                    modifier = Modifier.size(32.dp)
                                ) {
                                    WorkspaceClient.remove(pane.id)
                                }
                            }
                        }

                        if (!gridMode && allPanes.size > 1) {
                            Row(
                                modifier = Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
                                horizontalArrangement = Arrangement.spacedBy(4.dp)
                            ) {
                                allPanes.forEach { candidate ->
                                    val active = candidate.id == selectedPaneId
                                    TextButton(
                                        onClick = { WorkspaceClient.layout(false, candidate.id) },
                                        contentPadding = PaddingValues(horizontal = 8.dp, vertical = 2.dp),
                                        modifier = Modifier
                                            .heightIn(min = 30.dp)
                                            .background(
                                                if (active) ColorAccentCyan.copy(alpha = .12f) else Color.Transparent,
                                                RoundedCornerShape(10.dp)
                                            )
                                    ) {
                                        Text(
                                            localizedSourceTitle(candidate.source),
                                            color = if (active) ColorAccentCyan else ColorTextSecondary,
                                            style = MaterialTheme.typography.labelSmall,
                                            maxLines = 1
                                        )
                                    }
                                }
                            }
                        }

                        // Row 3: Thumb-friendly quick mode & view buttons
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            if (mobileTarget) {
                                MobileAction(
                                    symbol = MobileSymbol.Back,
                                    label = tr("Remote Back"),
                                    enabled = inputReady && pane.accepted && canRemoteBack,
                                    modifier = Modifier.size(40.dp)
                                ) {
                                    WorkspaceClient.keyboardBinding(pane.id, sessionKey, generation, token)?.let {
                                        WorkspaceClient.navigation(pane.id, sessionKey, generation, token, it, WorkspaceNavigation.BACK)
                                    }
                                }
                                MobileAction(
                                    symbol = MobileSymbol.Home,
                                    label = tr("Remote Home"),
                                    enabled = inputReady && pane.accepted && isPhoneScreen,
                                    modifier = Modifier.size(40.dp)
                                ) {
                                    WorkspaceClient.keyboardBinding(pane.id, sessionKey, generation, token)?.let {
                                        WorkspaceClient.navigation(pane.id, sessionKey, generation, token, it, WorkspaceNavigation.HOME)
                                    }
                                }
                            } else {
                                MobileAction(
                                    symbol = MobileSymbol.Trackpad,
                                    label = tr("Trackpad"),
                                    enabled = inputReady,
                                    selected = touchMode == TouchMode.TRACKPAD,
                                    modifier = Modifier.size(40.dp)
                                ) {
                                    WorkspaceClient.surface(pane.id, sessionKey, generation, token, "cancel")
                                    pointers.clear(); touching = false
                                    lastControlMode = TouchMode.TRACKPAD; touchMode = TouchMode.TRACKPAD
                                }
                                MobileAction(
                                    symbol = MobileSymbol.Touch,
                                    label = tr("Direct touch"),
                                    enabled = inputReady,
                                    selected = touchMode == TouchMode.DIRECT || touchMode == TouchMode.NATIVE,
                                    modifier = Modifier.size(40.dp)
                                ) {
                                    WorkspaceClient.surface(pane.id, sessionKey, generation, token, "cancel")
                                    pointers.clear(); touching = false
                                    val nextMode = if (pane.maxTouchPoints > 0) TouchMode.NATIVE else TouchMode.DIRECT
                                    lastControlMode = nextMode; touchMode = nextMode
                                }
                            }
                            MobileAction(
                                symbol = MobileSymbol.Pan,
                                label = tr("Pan canvas"),
                                selected = touchMode == TouchMode.VIEW,
                                modifier = Modifier.size(40.dp)
                            ) {
                                WorkspaceClient.surface(pane.id, sessionKey, generation, token, "cancel")
                                pointers.clear(); touching = false
                                if (touchMode == TouchMode.VIEW && pane.input) {
                                    touchMode = lastControlMode
                                } else {
                                    if (touchMode != TouchMode.VIEW) lastControlMode = touchMode
                                    touchMode = TouchMode.VIEW
                                }
                            }
                            MobileAction(
                                symbol = MobileSymbol.Keyboard,
                                label = tr("Show keyboard"),
                                enabled = inputReady,
                                modifier = Modifier.size(40.dp)
                            ) {
                                focusGate.invalidate(); WorkspaceClient.invalidateEditorFocus()
                                sideDrawerOpen = false; sideMoreOpen = false
                                if (keyboardVisible) {
                                    keyboardVisible = false
                                } else {
                                    keyboardBinding = WorkspaceClient.keyboardBinding(pane.id, sessionKey, generation, token)
                                    keyboardVisible = keyboardBinding != null
                                }
                            }
                            MobileAction(
                                symbol = MobileSymbol.Rotate,
                                label = if (landscapeOverride == true) tr("Unlock rotation") else tr("Rotate to landscape"),
                                selected = landscapeOverride == true,
                                modifier = Modifier.size(40.dp),
                                onClick = onToggleLandscape
                            )
                            MobileAction(
                                symbol = MobileSymbol.Refresh,
                                label = tr("Reset zoom"),
                                modifier = Modifier.size(40.dp)
                            ) {
                                transform.value = transform.value.reset()
                                if (pane.input && touchMode == TouchMode.VIEW) touchMode = lastControlMode
                            }
                        }
                    }
                }
            }
        }
      }
      if (imeVisible && pane.input) {
        Row(
            Modifier.fillMaxWidth().background(ColorSurfaceCard).padding(horizontal = 4.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Row(
                Modifier.weight(1f).horizontalScroll(rememberScrollState()),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
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
            MobileAction(
                symbol = MobileSymbol.Collapse,
                label = tr("Hide keyboard"),
                modifier = Modifier.size(40.dp)
            ) {
                focusGate.invalidate()
                WorkspaceClient.invalidateEditorFocus()
                keyboardVisible = false
                sideDrawerOpen = false
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
private fun localizedSourceTitle(source: WorkspaceSource): String {
    if (source.isWindow && !(source.application.trim().isEmpty() && source.title == "Shared window")) {
        return source.title
    }
    val raw = source.title.trim()
    return when (raw) {
        "Desktop" -> tr("Desktop")
        "Shared display" -> tr("Shared display")
        "Shared window" -> tr("Shared window")
        "Android selected screen or app" -> tr("Android selected screen or app")
        "Built-in Display" -> tr("Built-in Display")
        "Main Display" -> tr("Main Display")
        else -> {
            Regex("^Built-in Display \\((\\d+×\\d+)\\)$").matchEntire(raw)?.let { return trFormat("Built-in Display (%s)", it.groupValues[1]) }
            Regex("^Main Display \\((\\d+×\\d+)\\)$").matchEntire(raw)?.let { return trFormat("Main Display (%s)", it.groupValues[1]) }
            Regex("^Main Display · (.+) \\((\\d+×\\d+)\\)$").matchEntire(raw)?.let { return trFormat("Main Display · %s (%s)", it.groupValues[1], it.groupValues[2]) }
            Regex("^Display (\\d+) \\((\\d+×\\d+)\\)$").matchEntire(raw)?.let { return trFormat("Display %s (%s)", it.groupValues[1], it.groupValues[2]) }
            Regex("^Display (\\d+) · (.+) \\((\\d+×\\d+)\\)$").matchEntire(raw)?.let { return trFormat("Display %s · %s (%s)", it.groupValues[1], it.groupValues[2], it.groupValues[3]) }
            raw.ifEmpty { source.title }
        }
    }
}

@Composable
private fun SourcePicker(sources: List<WorkspaceSource>, onClose: () -> Unit, onSelect: (WorkspaceSource) -> Unit) {
    var query by remember { mutableStateOf("") }
    val quickScreen = remember(sources) { primaryScreenSource(sources) }
    val matches = remember(sources, query) {
        val hasDisplay = sources.any { (it.source as? JSONObject)?.has("Display") == true }
        sources.filter { !(hasDisplay && it.source == "MainDisplay") }
            .filter { it.title.contains(query.trim(), true) || it.application.contains(query.trim(), true) }
            .sortedBy { if (it.isCamera) 2 else if (it.isWindow) 1 else 0 }
    }
    AlertDialog(onDismissRequest = onClose,
        modifier = Modifier.imePadding().navigationBarsPadding().widthIn(max = 560.dp).fillMaxWidth(.9f),
        properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false),
        title = {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
                Text(tr("Sources"), maxLines = 1)
                if (quickScreen != null) {
                    TextButton(onClick = { onSelect(quickScreen) }) { Text(tr("Open main screen")) }
                }
            }
        }, text = {
        Column(Modifier.fillMaxWidth()) {
            OutlinedTextField(query, { query = it }, label = { Text(tr("Search apps and windows")) }, singleLine = true, modifier = Modifier.fillMaxWidth())
            Spacer(Modifier.height(8.dp))
            LazyColumn(Modifier.weight(1f, fill = false).fillMaxWidth().heightIn(max = 300.dp)) {
                itemsIndexed(matches, key = { _, source -> source.source.toString() }) { index, source ->
                  Column(Modifier.fillMaxWidth()) {
                    if (index == 0 || matches[index - 1].group != source.group) {
                        Text(tr(source.group),
                            style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary,
                            modifier = Modifier.padding(top = 8.dp, bottom = 4.dp))
                    }
                    val app=source.application.trim().ifBlank { "Other applications" }
                    if (source.isWindow && (index==0 || !matches[index-1].isWindow ||
                            !matches[index-1].application.trim().ifBlank { "Other applications" }.equals(app,true))) {
                        Text(if (source.application.trim().isBlank()) tr("Other applications") else app,style=MaterialTheme.typography.labelMedium,color=MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier=Modifier.padding(start=8.dp,top=8.dp,bottom=2.dp))
                    }
                    TextButton(onClick = { onSelect(source) }, modifier = Modifier.fillMaxWidth(),
                        contentPadding=PaddingValues(start=if (source.isWindow) 16.dp else 8.dp,end=8.dp,top=6.dp,bottom=6.dp)) {
                        Row(Modifier.fillMaxWidth(),verticalAlignment=Alignment.CenterVertically) {
                            SourceIcon(source)
                            Text(localizedSourceTitle(source),modifier=Modifier.weight(1f),maxLines=2,
                                textAlign=androidx.compose.ui.text.style.TextAlign.Start,
                                overflow=TextOverflow.Ellipsis)
                        }
                    }
                  }
                }
                if (matches.isEmpty()) item {
                    Row(Modifier.fillMaxWidth(),horizontalArrangement=Arrangement.SpaceBetween,verticalAlignment=Alignment.CenterVertically) {
                        Text(tr("No matching sources"),style=MaterialTheme.typography.bodySmall)
                        TextButton(onClick={query=""}) { Text(tr("Clear search")) }
                    }
                }
            }
        }
    }, confirmButton = { TextButton(onClick = onClose) { Text(tr("Close")) } })
}
