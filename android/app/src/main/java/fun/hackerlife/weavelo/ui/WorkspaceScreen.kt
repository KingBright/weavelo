package `fun`.hackerlife.weavelo.ui

import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.BorderStroke
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
import androidx.compose.foundation.text.KeyboardOptions
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
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.input.pointer.*
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
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

    WeaveloTheme {
        val p = WeaveloTokens.colors

        @Composable
        fun SessionActionsDropdown(expanded: Boolean, onDismiss: () -> Unit) {
            DropdownMenu(
                expanded = expanded,
                onDismissRequest = onDismiss,
            ) {
                Column(
                    modifier = Modifier
                        .widthIn(min = 248.dp, max = 300.dp)
                        .padding(horizontal = 8.dp, vertical = 4.dp),
                    verticalArrangement = Arrangement.spacedBy(2.dp)
                ) {
                    LanguageSelector(modifier = Modifier.fillMaxWidth().padding(bottom = 4.dp))
                    HorizontalDivider(color = p.borderMuted)
                    if (!state.connected && !state.incomingEnded) {
                        PrecisionMenuItem(
                            text = tr("Reconnect"),
                            symbol = MobileSymbol.Refresh,
                            onClick = { onDismiss(); WorkspaceClient.reconnect() }
                        )
                    }
                    if (state.panes.size > 1) {
                        PrecisionMenuItem(
                            text = if (state.grid) tr("Use tabs") else tr("Show together"),
                            symbol = MobileSymbol.Grid,
                            onClick = { onDismiss(); WorkspaceClient.layout(!state.grid) }
                        )
                    }
                    PrecisionMenuItem(
                        text = tr("Refresh screens and windows"),
                        symbol = MobileSymbol.Refresh,
                        enabled = state.connected && !state.incomingEnded,
                        onClick = { onDismiss(); WorkspaceClient.refreshSources(sessionKey, state.connectionGeneration) }
                    )
                    HorizontalDivider(color = p.borderMuted)
                    PrecisionMenuItem(
                        text = tr("Send file"),
                        symbol = MobileSymbol.Send,
                        enabled = state.connected && state.files,
                        onClick = { onDismiss(); pickerSessionKey = sessionKey; shareSelection = false; filePicker.launch(arrayOf("*/*")) }
                    )
                    PrecisionMenuItem(
                        text = tr("Share file with network"),
                        symbol = MobileSymbol.Share,
                        enabled = state.connected,
                        onClick = { onDismiss(); pickerSessionKey = sessionKey; shareSelection = true; filePicker.launch(arrayOf("*/*")) }
                    )
                    PrecisionMenuItem(
                        text = tr("Retrieve shared files"),
                        symbol = MobileSymbol.Download,
                        enabled = state.connected && state.files && !state.sharedBusy,
                        onClick = { onDismiss(); sharedFilesOpen = true; WorkspaceClient.listSharedFiles() }
                    )
                    PrecisionMenuItem(
                        text = tr("Received files"),
                        symbol = MobileSymbol.Folder,
                        onClick = { onDismiss(); receivedFiles = true }
                    )
                    PrecisionMenuItem(
                        text = tr("Clear my shared list"),
                        symbol = MobileSymbol.Close,
                        enabled = state.connected,
                        onClick = { onDismiss(); WorkspaceClient.clearSharedFiles() }
                    )
                    HorizontalDivider(color = p.borderMuted)
                    PrecisionMenuItem(
                        text = tr("Clipboard"),
                        symbol = MobileSymbol.Clipboard,
                        checked = state.clipboard,
                        enabled = state.connected && state.clipboardAvailable,
                        onClick = { onDismiss(); WorkspaceClient.enableClipboard(!state.clipboard) }
                    )
                    PrecisionMenuItem(
                        text = tr("Send clipboard"),
                        symbol = MobileSymbol.Copy,
                        enabled = state.connected && state.clipboard,
                        onClick = { onDismiss(); sendClipboardNow() }
                    )
                    PrecisionMenuItem(
                        text = tr("Background"),
                        symbol = MobileSymbol.Settings,
                        onClick = { onDismiss(); backgroundSettings = true }
                    )
                    HorizontalDivider(color = p.borderMuted)
                    PrecisionMenuItem(
                        text = tr("Disconnect"),
                        symbol = MobileSymbol.Disconnect,
                        destructive = true,
                        onClick = { onDismiss(); disconnect() }
                    )
                }
            }
        }

        if (sourcePicker) {
            if (state.connectionGeneration != pickerGeneration || state.sessionKey != sessionKey || !state.connected) {
                LaunchedEffect(Unit) { sourcePicker = false }
            } else SourcePicker(state.sources, onClose = { sourcePicker = false }) { source ->
                localError = null; localErrorText = null
                WorkspaceClient.add(source, sessionKey, pickerGeneration)
                sourcePicker = false
            }
        }

        if (backgroundSettings) {
            PrecisionDialog(
                title = tr("Background connection"),
                onDismissRequest = { backgroundSettings = false },
                icon = MobileSymbol.Network,
                dismissButton = {
                    PrecisionButton(
                        text = tr("Cancel"),
                        onClick = { backgroundSettings = false },
                        style = PrecisionButtonStyle.SUBTLE,
                        compact = true
                    )
                },
                confirmButton = {
                    PrecisionButton(
                        text = tr("Save"),
                        onClick = {
                            val seconds = keepAlive.toIntOrNull()
                            if (seconds != null && seconds in 0..86400) {
                                WeaveloClient.updateBackgroundKeepAlive(seconds)
                                WeaveloClient.updateBackgroundServiceEnabled(keepService)
                                preferences.edit().putBoolean("adaptive_background", adaptive).apply()
                                backgroundSettings = false
                            }
                        },
                        style = PrecisionButtonStyle.PRIMARY,
                        compact = true
                    )
                }
            ) {
                Column(
                    modifier = Modifier
                        .heightIn(max = 360.dp)
                        .verticalScroll(rememberScrollState()),
                    verticalArrangement = Arrangement.spacedBy(10.dp)
                ) {
                    PrecisionSwitchRow(
                        title = tr("Keep this connection in background until Disconnect"),
                        subtitle = tr("This is an explicit foreground service, not a battery or system-stop exemption. The idle timeout below applies when this is off."),
                        checked = keepService,
                        onCheckedChange = { keepService = it },
                        leadingSymbol = MobileSymbol.Network
                    )
                    PrecisionTextField(
                        value = keepAlive,
                        onValueChange = { keepAlive = it },
                        label = tr("Idle seconds (0 = always keep)"),
                        isError = keepAlive.toIntOrNull()?.let { it !in 0..86400 } ?: true,
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number)
                    )
                    PrecisionSwitchRow(
                        title = tr("Adjust for battery and mobile data"),
                        subtitle = tr("Media pauses immediately. Idle connections sleep sooner on battery saver (30s), low battery (60s), or metered data (120s). File transfers finish first. Reopen to reconnect."),
                        checked = adaptive,
                        onCheckedChange = { adaptive = it },
                        leadingSymbol = MobileSymbol.Settings
                    )
                }
            }
        }

        Box(Modifier.fillMaxSize().background(if (state.panes.isEmpty()) p.canvas else Color(0xFF070A0F))) {
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
                            modifier = Modifier
                                .align(Alignment.TopCenter)
                                .safeDrawingPadding()
                                .padding(8.dp)
                                .border(1.dp, p.borderStrong, RoundedCornerShape(18.dp)),
                            shape = RoundedCornerShape(18.dp),
                            color = p.surfaceGlass,
                            shadowElevation = 6.dp
                        ) {
                            Row(
                                modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(8.dp)
                            ) {
                                Text(
                                    text = if (localError != null && localErrorText != null) appText(localErrorText!!)
                                        else if (localError == null && state.statusText != null) appText(state.statusText!!)
                                        else trFormat("Details: %s", streamIssue),
                                    color = p.textPrimary,
                                    style = MaterialTheme.typography.labelMedium,
                                    maxLines = 2,
                                    modifier = Modifier.widthIn(max = 240.dp)
                                )
                                if (!state.connected && !state.incomingEnded) {
                                    PrecisionButton(
                                        text = tr("Reconnect"),
                                        onClick = { WorkspaceClient.reconnect() },
                                        style = PrecisionButtonStyle.TONAL,
                                        compact = true
                                    )
                                }
                                if (localError != null) {
                                    MobileAction(MobileSymbol.Close, tr("Close"), tonalWell = true, modifier = Modifier.size(32.dp)) {
                                        localError = null; localErrorText = null
                                    }
                                } else {
                                    MobileAction(MobileSymbol.Close, tr("Exit stream"), tonalWell = true, modifier = Modifier.size(32.dp), onClick = disconnect)
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
                    PrecisionCard(
                        modifier = Modifier
                            .widthIn(max = 460.dp)
                            .padding(horizontal = 18.dp, vertical = 24.dp),
                        cornerRadius = 22.dp,
                        contentPadding = PaddingValues(20.dp)
                    ) {
                        Column(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalAlignment = Alignment.CenterHorizontally,
                            verticalArrangement = Arrangement.spacedBy(12.dp)
                        ) {
                            Box(
                                modifier = Modifier
                                    .size(52.dp)
                                    .clip(RoundedCornerShape(16.dp))
                                    .background(p.brandSoft)
                                    .border(1.dp, p.brandBorder.copy(alpha = 0.55f), RoundedCornerShape(16.dp)),
                                contentAlignment = Alignment.Center
                            ) {
                                if (loadingCatalog) {
                                    CircularProgressIndicator(modifier = Modifier.size(24.dp), strokeWidth = 2.6.dp, color = p.brand)
                                } else {
                                    Icon(MobileSymbol.Sources.image, null, tint = p.brand, modifier = Modifier.size(26.dp))
                                }
                            }
                            Text(when {
                                state.incomingEnded -> tr("Sharing ended")
                                loadingCatalog -> tr("Connecting…")
                                else -> tr("Choose a screen")
                            }, color = p.textPrimary, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
                            Text(when {
                                state.incomingEnded -> tr("You can still exchange files.")
                                loadingCatalog -> tr("Connecting and loading shared screens…")
                                state.sources.isEmpty() -> tr("This device has not shared a screen yet. You can still exchange files.")
                                else -> tr("Open a display or application window. File transfers stay independent.")
                            }, color = p.textSecondary, fontSize = 13.sp, lineHeight = 19.sp,
                                textAlign = androidx.compose.ui.text.style.TextAlign.Center)
                            if (!state.incomingEnded && !loadingCatalog) {
                                Row(
                                    horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.CenterHorizontally),
                                    verticalAlignment = Alignment.CenterVertically,
                                    modifier = Modifier.fillMaxWidth()
                                ) {
                                    if (quickScreen != null) {
                                        PrecisionButton(
                                            text = tr("Open main screen"),
                                            leadingSymbol = MobileSymbol.View,
                                            enabled = state.connected,
                                            style = PrecisionButtonStyle.PRIMARY,
                                            onClick = {
                                                localError = null; localErrorText = null
                                                WorkspaceClient.add(quickScreen, sessionKey, state.connectionGeneration)
                                            }
                                        )
                                    }
                                    PrecisionButton(
                                        text = tr("Browse screens and windows"),
                                        leadingSymbol = MobileSymbol.Sources,
                                        enabled = state.connected,
                                        style = if (quickScreen != null) PrecisionButtonStyle.TONAL else PrecisionButtonStyle.PRIMARY,
                                        onClick = {
                                            WorkspaceClient.refreshSources(sessionKey, state.connectionGeneration)
                                            pickerGeneration = state.connectionGeneration; sourcePicker = true
                                        }
                                    )
                                }
                                if (quickSources.isNotEmpty()) {
                                    HorizontalDivider(color = p.borderMuted)
                                    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                                        quickSources.forEach { source ->
                                            Row(
                                                modifier = Modifier
                                                    .fillMaxWidth()
                                                    .clip(RoundedCornerShape(12.dp))
                                                    .background(p.surfaceSunken)
                                                    .border(1.dp, p.borderMuted, RoundedCornerShape(12.dp))
                                                    .clickable(enabled = state.connected, role = Role.Button) {
                                                        localError = null; localErrorText = null
                                                        WorkspaceClient.add(source, sessionKey, state.connectionGeneration)
                                                    }
                                                    .padding(horizontal = 12.dp, vertical = 9.dp),
                                                verticalAlignment = Alignment.CenterVertically,
                                                horizontalArrangement = Arrangement.spacedBy(8.dp)
                                            ) {
                                                SourceIcon(source)
                                                Text(
                                                    localizedSourceTitle(source),
                                                    color = p.textPrimary,
                                                    fontSize = 13.sp,
                                                    fontWeight = FontWeight.Medium,
                                                    modifier = Modifier.weight(1f),
                                                    maxLines = 1,
                                                    overflow = TextOverflow.Ellipsis
                                                )
                                                Icon(
                                                    MobileSymbol.ChevronRight.image,
                                                    contentDescription = null,
                                                    tint = p.textTertiary,
                                                    modifier = Modifier.size(16.dp)
                                                )
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
                Surface(
                    modifier = Modifier
                        .align(Alignment.TopCenter)
                        .fillMaxWidth()
                        .safeDrawingPadding()
                        .padding(horizontal = 10.dp, vertical = 8.dp)
                        .border(1.dp, p.border, RoundedCornerShape(20.dp)),
                    shape = RoundedCornerShape(20.dp),
                    color = p.surfaceGlass,
                    shadowElevation = 6.dp
                ) {
                    Column(Modifier.padding(horizontal = 4.dp, vertical = 4.dp)) {
                        Row(Modifier.fillMaxWidth().heightIn(min = 50.dp), verticalAlignment = Alignment.CenterVertically) {
                            MobileAction(MobileSymbol.Back, tr("Exit stream"), tonalWell = true, onClick = disconnect)
                            Column(Modifier.weight(1f).padding(horizontal = 8.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                                    Text(
                                        remoteDevice?.name ?: tr("Remote device"),
                                        color = p.textPrimary,
                                        fontSize = 14.5.sp,
                                        fontWeight = FontWeight.SemiBold,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                        modifier = Modifier.weight(1f, fill = false)
                                    )
                                    if (routeBadge != null) {
                                        PrecisionPillBadge(text = routeBadge, tone = PillTone.EMERALD, showDot = state.connected)
                                    }
                                }
                                val subStatus = when {
                                    state.incomingEnded -> tr("Sharing ended")
                                    state.connected -> tr("Connected")
                                    else -> tr("Connecting…")
                                }
                                Text(subStatus, color = p.textSecondary, fontSize = 11.5.sp)
                            }
                            MobileAction(MobileSymbol.Sources, tr("Choose screen or app window"), state.connected && !state.incomingEnded, tonalWell = true) {
                                WorkspaceClient.refreshSources(sessionKey, state.connectionGeneration)
                                pickerGeneration = state.connectionGeneration; sourcePicker = true
                            }
                            MobileAction(MobileSymbol.Refresh, tr("Refresh screens and windows"), state.connected && !state.incomingEnded, tonalWell = true) {
                                WorkspaceClient.refreshSources(sessionKey, state.connectionGeneration)
                            }
                            Box {
                                MobileAction(MobileSymbol.More, tr("Connection actions"), tonalWell = true) { actionsOpen = true }
                                SessionActionsDropdown(actionsOpen) { actionsOpen = false }
                            }
                        }
                        (localError ?: state.status.takeUnless { it == "Connected" })?.takeIf { it.isNotBlank() }?.let {
                            Text(
                                if (localError != null && localErrorText != null) appText(localErrorText!!)
                                else if (localError == null && state.statusText != null) appText(state.statusText!!)
                                else trFormat("Details: %s", it),
                                color = if (localError != null) p.dangerText else p.textSecondary,
                                fontSize = 12.sp,
                                maxLines = 2,
                                modifier = Modifier.padding(horizontal = 12.dp, vertical = 4.dp)
                            )
                        }
                    }
                }
            }
        }
        selectedSettings?.let { selectedPane ->
            val pane = state.panes.firstOrNull { it.id == selectedPane.id && state.connectionGeneration == settingsGeneration }
                ?: selectedPane
            val settingsCurrent = state.connected && state.connectionGeneration == settingsGeneration &&
                state.panes.any { it.id == pane.id && it.accepted } && (state.grid || state.selected == pane.id)
            var fps by remember(pane.id, pane.fps) { mutableStateOf(pane.fps.toString()) }
            var kbps by remember(pane.id, pane.kbps) { mutableStateOf(pane.kbps.toString()) }
            val naturalScroll by WeaveloClient.naturalScrollFlow.collectAsState()
            val isWindowSource = pane.source.isWindow
            val srcW = pane.source.width.coerceAtLeast(2)
            val srcH = pane.source.height.coerceAtLeast(2)
            val (encW, encH) = WorkspaceClient.scaledEncodeSize(srcW, srcH, pane.windowScalePercent)
            val baseStreamPrefs = remember { context.getSharedPreferences("stream", android.content.Context.MODE_PRIVATE) }
            val isWindowCustomized = isWindowSource && (
                pane.windowScalePercent != 100 ||
                    pane.windowLayoutPreset != "original" ||
                    pane.fps != baseStreamPrefs.getInt("fps", 60) ||
                    pane.kbps != baseStreamPrefs.getInt("bitrate_kbps", 20_000) ||
                    pane.windowResizeNotice != null
                )
            val localMetrics = context.resources.displayMetrics
            PrecisionDialog(
                title = if (isWindowSource) tr("Window Stream & Layout") else tr("Stream settings"),
                subtitle = if (isWindowSource) tr("Isolated · Global defaults untouched") else null,
                onDismissRequest = { selectedSettings = null },
                icon = MobileSymbol.Settings,
                dismissButton = {
                    PrecisionButton(
                        text = tr("Cancel"),
                        onClick = { selectedSettings = null },
                        style = PrecisionButtonStyle.SUBTLE,
                        compact = true
                    )
                },
                confirmButton = {
                    PrecisionButton(
                        text = tr("Apply"),
                        enabled = settingsCurrent,
                        onClick = {
                            val f = fps.toIntOrNull(); val k = kbps.toIntOrNull()
                            if (f != null && k != null && f > 0 && k > 0) {
                                val (w, h) = WorkspaceClient.scaledEncodeSize(srcW, srcH, pane.windowScalePercent)
                                WorkspaceClient.settings(
                                    pane.id, sessionKey, settingsGeneration,
                                    width = if (!pane.source.isCamera) w else null,
                                    height = if (!pane.source.isCamera) h else null,
                                    fps = f, kbps = k,
                                    scalePercent = pane.windowScalePercent
                                )
                                selectedSettings = null
                            }
                        },
                        style = PrecisionButtonStyle.PRIMARY,
                        compact = true
                    )
                }
            ) {
                Column(
                    modifier = Modifier
                        .heightIn(max = 440.dp)
                        .verticalScroll(rememberScrollState()),
                    verticalArrangement = Arrangement.spacedBy(10.dp)
                ) {
                    if (!pane.source.isCamera) {
                        PrecisionCard(sunken = true, contentPadding = PaddingValues(11.dp)) {
                            Row(
                                Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.SpaceBetween,
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                Text(
                                    text = tr("Stream Clarity Scale"),
                                    color = p.textPrimary,
                                    fontSize = 12.5.sp,
                                    fontWeight = FontWeight.SemiBold
                                )
                                Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                                    PrecisionPillBadge(
                                        text = "${encW}×${encH}",
                                        tone = PillTone.BRAND,
                                        monospace = true
                                    )
                                    if (isWindowCustomized) {
                                        PrecisionButton(
                                            text = tr("Reset default"),
                                            enabled = settingsCurrent,
                                            style = PrecisionButtonStyle.SUBTLE,
                                            compact = true,
                                            onClick = {
                                                WorkspaceClient.resetWindowOverride(pane.id, sessionKey, settingsGeneration, srcW, srcH)
                                            }
                                        )
                                    }
                                }
                            }
                            Spacer(Modifier.height(8.dp))
                            PrecisionSegmentedRow(
                                options = listOf(
                                    100 to tr("Native 100%"),
                                    75 to tr("HD 75%"),
                                    50 to tr("Std 50%"),
                                    35 to tr("Saver 35%")
                                ),
                                selected = pane.windowScalePercent,
                                enabled = settingsCurrent,
                                modifier = Modifier.fillMaxWidth(),
                                onSelect = { pct ->
                                    val (w, h) = WorkspaceClient.scaledEncodeSize(srcW, srcH, pct)
                                    WorkspaceClient.settings(
                                        pane.id, sessionKey, settingsGeneration,
                                        width = w, height = h,
                                        scalePercent = pct
                                    )
                                }
                            )
                        }
                    }
                    if (isWindowSource) {
                        PrecisionCard(sunken = true, contentPadding = PaddingValues(11.dp)) {
                            Row(
                                Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.SpaceBetween,
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                Text(
                                    text = tr("Remote Window Size"),
                                    color = p.textPrimary,
                                    fontSize = 12.5.sp,
                                    fontWeight = FontWeight.SemiBold
                                )
                                Text(
                                    text = tr("Auto-restores on exit"),
                                    color = p.textTertiary,
                                    fontSize = 10.5.sp
                                )
                            }
                            Spacer(Modifier.height(8.dp))
                            val layoutRows = listOf(
                                listOf(
                                    "original" to tr("Original size"),
                                    "fit_viewport" to tr("Fit viewport")
                                ),
                                listOf(
                                    "1280x800" to "1280×800",
                                    "1440x900" to "1440×900",
                                    "900x1440" to tr("900×1440 Portrait")
                                )
                            )
                            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                                layoutRows.forEach { rowItems ->
                                    PrecisionSegmentedRow(
                                        options = rowItems,
                                        selected = pane.windowLayoutPreset,
                                        enabled = settingsCurrent,
                                        modifier = Modifier.fillMaxWidth(),
                                        onSelect = { preset ->
                                            val target = WorkspaceClient.presetWindowTargetSize(
                                                preset,
                                                localMetrics.widthPixels,
                                                localMetrics.heightPixels
                                            )
                                            WorkspaceClient.resizeWindow(
                                                pane.id, sessionKey, settingsGeneration,
                                                preset = preset,
                                                targetWidth = target?.first,
                                                targetHeight = target?.second
                                            )
                                        }
                                    )
                                }
                            }
                            pane.windowResizeNotice?.let { notice ->
                                Spacer(Modifier.height(6.dp))
                                Text(
                                    text = if (pane.windowResizeError) tr(notice) else "${tr("Remote window adapted to")} $notice",
                                    color = if (pane.windowResizeError) p.dangerText else p.brand,
                                    fontSize = 11.sp,
                                    fontWeight = FontWeight.Medium
                                )
                            }
                        }
                    }
                    PrecisionSegmentedRow(
                        options = listOf(
                            30 to "30 FPS",
                            60 to "60 FPS",
                            120 to "120 FPS"
                        ),
                        selected = fps.toIntOrNull() ?: pane.fps,
                        enabled = settingsCurrent,
                        modifier = Modifier.fillMaxWidth(),
                        onSelect = { selectedFps ->
                            fps = selectedFps.toString()
                            val currentKbps = kbps.toIntOrNull() ?: pane.kbps
                            WorkspaceClient.settings(
                                pane.id, sessionKey, settingsGeneration,
                                fps = selectedFps,
                                kbps = currentKbps.coerceAtLeast(500)
                            )
                        }
                    )
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        PrecisionTextField(
                            value = fps,
                            onValueChange = { fps = it },
                            label = tr("Frames per second"),
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                            modifier = Modifier.weight(1f)
                        )
                        PrecisionTextField(
                            value = kbps,
                            onValueChange = { kbps = it },
                            label = tr("Bitrate (kbps)"),
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                            modifier = Modifier.weight(1f)
                        )
                    }
                    PrecisionSwitchRow(
                        title = tr("Natural scrolling (Apple style)"),
                        subtitle = tr("Content moves in the same direction as your fingers"),
                        checked = naturalScroll,
                        onCheckedChange = { WeaveloClient.updateNaturalScroll(it) },
                        leadingSymbol = MobileSymbol.Touch
                    )
                    pane.audioOwner?.let { owner ->
                        HorizontalDivider(color = p.borderMuted)
                        Row(
                            Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Text(tr("Volume"), color = p.textPrimary, fontSize = 13.sp, fontWeight = FontWeight.SemiBold)
                            Text(
                                "${((volumes[owner] ?: 1f) * 100).toInt()}%",
                                color = p.brand,
                                fontSize = 11.5.sp,
                                fontFamily = FontFamily.Monospace,
                                fontWeight = FontWeight.Bold
                            )
                        }
                        Slider(
                            value = volumes[owner] ?: 1f,
                            onValueChange = { volumes[owner] = it },
                            colors = SliderDefaults.colors(
                                thumbColor = p.brand,
                                activeTrackColor = p.brand,
                                inactiveTrackColor = p.surfaceSunken
                            )
                        )
                    }
                    if (pane.source.isDisplay && !state.androidHost) {
                        val output = pane.audioOutput
                        HorizontalDivider(color = p.borderMuted)
                        Text(tr("Sound on controlled device"), color = p.textPrimary, fontSize = 13.sp, fontWeight = FontWeight.SemiBold)
                        Text(
                            tr(when {
                                output.pendingRevision != null -> "Changing host audio output…"
                                output.localPlayback == "Enabled" -> "Host sound is on"
                                output.localPlayback == "Disabled" -> "Host sound is off"
                                output.localPlayback == "Partial" -> "Host sound is partially on"
                                else -> "Host sound state is unknown"
                            }),
                            color = p.textSecondary,
                            fontSize = 12.sp
                        )
                        output.messageText?.let { Text(appText(it), color = p.textSecondary, fontSize = 11.5.sp) }
                        output.message?.takeIf { it.isNotBlank() }?.let { Text(tr(it), color = p.textSecondary, fontSize = 11.5.sp) }
                        if (output.privateSupported == false) {
                            Text(tr("Private output is unavailable on this host"), color = p.warningText, fontSize = 11.5.sp)
                        }
                        val selectedRoute = if (output.localPlayback == "Disabled") "Private" else "Mirror"
                        PrecisionSegmentedRow(
                            options = listOf(
                                "Mirror" to tr("Play on both"),
                                "Private" to tr("Remote only")
                            ),
                            selected = selectedRoute,
                            enabled = settingsCurrent && output.pendingRevision == null && output.privateSupported != false,
                            onSelect = { mode ->
                                WorkspaceClient.setAudioOutput(pane.id, sessionKey, settingsGeneration, mode)
                            },
                            modifier = Modifier.fillMaxWidth()
                        )
                    }
                }
            }
        }
        if (sharedFilesOpen) {
            PrecisionDialog(
                title = tr("Remote shared files"),
                onDismissRequest = { sharedFilesOpen = false },
                icon = MobileSymbol.Download,
                subtitle = tr("Only explicitly shared files are listed. No remote filesystem paths are accepted."),
                confirmButton = {
                    PrecisionButton(
                        text = tr("Close"),
                        onClick = { sharedFilesOpen = false },
                        style = PrecisionButtonStyle.SUBTLE,
                        compact = true
                    )
                }
            ) {
                Text(state.status, color = p.textSecondary, fontSize = 12.sp)
                Column(
                    modifier = Modifier
                        .heightIn(max = 260.dp)
                        .verticalScroll(rememberScrollState()),
                    verticalArrangement = Arrangement.spacedBy(6.dp)
                ) {
                    state.remoteFiles.forEach { file ->
                        PrecisionMenuItem(
                            text = trFormat("Get %s (%d bytes)", file.name, file.sizeBytes),
                            symbol = MobileSymbol.Download,
                            enabled = state.connected && !state.sharedBusy,
                            onClick = { WorkspaceClient.fetchSharedFile(file.id, sessionKey) }
                        )
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    PrecisionButton(
                        text = tr("Refresh"),
                        leadingSymbol = MobileSymbol.Refresh,
                        enabled = state.connected && !state.sharedBusy,
                        style = PrecisionButtonStyle.TONAL,
                        compact = true,
                        onClick = { WorkspaceClient.listSharedFiles() }
                    )
                    state.sharedNext?.let { next ->
                        PrecisionButton(
                            text = tr("Next page"),
                            enabled = state.connected && !state.sharedBusy,
                            style = PrecisionButtonStyle.SUBTLE,
                            compact = true,
                            onClick = { WorkspaceClient.listSharedFiles(next) }
                        )
                    }
                }
            }
        }
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
    val p = WeaveloTokens.colors
    pane.videoError?.let { reason ->
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            PrecisionCard(
                modifier = Modifier.widthIn(max = 360.dp).padding(20.dp),
                cornerRadius = 18.dp,
                contentPadding = PaddingValues(18.dp)
            ) {
                Column(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(12.dp)
                ) {
                    Text(
                        pane.videoErrorText?.let { appText(it) } ?: trFormat("Details: %s", reason),
                        color = p.textSecondary,
                        fontSize = 13.sp,
                        textAlign = androidx.compose.ui.text.style.TextAlign.Center
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        PrecisionButton(
                            text = tr("Retry this source"),
                            leadingSymbol = MobileSymbol.Refresh,
                            style = PrecisionButtonStyle.PRIMARY,
                            compact = true,
                            onClick = { WorkspaceClient.retry(pane.id, sessionKey, generation) }
                        )
                        PrecisionButton(
                            text = tr("Switch screen"),
                            leadingSymbol = MobileSymbol.Sources,
                            style = PrecisionButtonStyle.TONAL,
                            compact = true,
                            onClick = onPickSource
                        )
                    }
                }
            }
        }
        return
    }
    if (pane.source.isCamera && !pane.accepted) {
        Box(Modifier.fillMaxSize(),contentAlignment=Alignment.Center) { Text(tr("Updating camera…"),color=p.textSecondary) }
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
    LaunchedEffect(baseMeasured, dimensions, pane.windowScalePercent, pane.accepted, keyboardVisible, imeVisible) {
        if (!pane.accepted || keyboardVisible || imeVisible) return@LaunchedEffect
        delay(200)
        val quarterTurn = pane.source.rotation % 180 != 0
        if (!pane.source.isCamera) {
            val (scaledW, scaledH) = WorkspaceClient.scaledEncodeSize(dimensions.width, dimensions.height, pane.windowScalePercent)
            if (scaledW >= 32 && scaledH >= 32) {
                WorkspaceClient.settings(pane.id, sessionKey, generation, width = scaledW, height = scaledH)
            }
        } else {
            VideoDimensions.fitEncoder(
                dimensions.width, dimensions.height,
                (if (quarterTurn) baseMeasured.height else baseMeasured.width).coerceAtMost(4096),
                (if (quarterTurn) baseMeasured.width else baseMeasured.height).coerceAtMost(4096), 2, 2
            )?.takeIf { it.width >= 32 && it.height >= 32 }?.let {
                WorkspaceClient.settings(pane.id, sessionKey, generation, width = it.width, height = it.height)
            }
        }
    }
    LaunchedEffect(pane.id, sessionKey, generation, pane.accepted, pane.windowLayoutPreset, containerMeasured) {
        if (!pane.accepted || !pane.source.isWindow || pane.windowLayoutPreset != "fit_viewport") return@LaunchedEffect
        if (containerMeasured.width < 64 || containerMeasured.height < 64) return@LaunchedEffect
        delay(220)
        val (fitW, fitH) = WorkspaceClient.fitRemoteWindowViewport(containerMeasured.width, containerMeasured.height)
        if (kotlin.math.abs(dimensions.width - fitW) >= 24 || kotlin.math.abs(dimensions.height - fitH) >= 24) {
            WorkspaceClient.resizeWindow(pane.id, sessionKey, generation, preset = "fit_viewport", targetWidth = fitW, targetHeight = fitH)
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
                    WeaveloPointerOverlay(
                        normX = cursor.x,
                        normY = cursor.y,
                        pressing = touching,
                        transform = transform.value,
                        baseWidth = baseMeasured.width.coerceAtLeast(1).toFloat(),
                        baseHeight = baseMeasured.height.coerceAtLeast(1).toFloat()
                    )
                }
                (videoStatus ?: pane.inputError)?.let { message ->
                    Surface(
                        modifier = Modifier
                            .align(Alignment.TopCenter)
                            .padding(12.dp)
                            .border(1.dp, p.border, RoundedCornerShape(14.dp)),
                        shape = RoundedCornerShape(14.dp),
                        color = p.surfaceGlass,
                        shadowElevation = 6.dp
                    ) {
                        Text(
                            if (videoStatus != null) tr(message) else pane.inputErrorText?.let { appText(it) } ?: trFormat("Details: %s", message),
                            color = p.textPrimary,
                            fontSize = 12.sp,
                            modifier = Modifier.padding(horizontal = 12.dp, vertical = 7.dp)
                        )
                    }
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
                modifier = Modifier
                    .align(Alignment.BottomStart)
                    .safeDrawingPadding()
                    .padding(8.dp)
                    .border(1.dp, p.border, RoundedCornerShape(22.dp)),
                shape = RoundedCornerShape(22.dp),
                color = p.surfaceGlass,
                shadowElevation = 6.dp
            ) {
                Row(
                    modifier = Modifier.padding(horizontal = 6.dp, vertical = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(4.dp)
                ) {
                    PrecisionButton(
                        text = tr("Fill height"),
                        leadingSymbol = MobileSymbol.ExpandVertical,
                        style = PrecisionButtonStyle.TONAL,
                        compact = true,
                        onClick = {
                            val bw = baseMeasured.width.coerceAtLeast(1).toFloat()
                            val bh = baseMeasured.height.coerceAtLeast(1).toFloat()
                            val cw = containerMeasured.width.coerceAtLeast(baseMeasured.width).coerceAtLeast(1).toFloat()
                            val ch = containerMeasured.height.coerceAtLeast(baseMeasured.height).coerceAtLeast(1).toFloat()
                            transform.value = transform.value.fillHeight(bw, bh, cw, ch)
                        }
                    )
                    PrecisionButton(
                        text = tr("Landscape view"),
                        leadingSymbol = MobileSymbol.Rotate,
                        style = PrecisionButtonStyle.SUBTLE,
                        compact = true,
                        onClick = onToggleLandscape
                    )
                    MobileAction(MobileSymbol.Close, tr("Close"), modifier = Modifier.size(32.dp), onClick = onDismissPortraitHint)
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
                .background(p.brand.copy(alpha = pillAlpha), RoundedCornerShape(6.dp)))

            if (sideDrawerOpen) {
                val paneTitle = localizedSourceTitle(pane.source)
                val inputReady = pane.input && token != 0L
                Surface(
                    modifier = Modifier
                        .align(Alignment.CenterEnd)
                        .safeDrawingPadding()
                        .padding(end = 42.dp, top = 8.dp, bottom = 8.dp)
                        .widthIn(min = 232.dp, max = 286.dp)
                        .border(1.dp, p.borderStrong, RoundedCornerShape(22.dp))
                        .pointerInput(pane.id, sessionKey, generation) {
                            awaitPointerEventScope {
                                while (true) {
                                    if (awaitPointerEvent(PointerEventPass.Initial).type == PointerEventType.Press) {
                                        sideInteraction++
                                    }
                                }
                            }
                        },
                    shape = RoundedCornerShape(22.dp),
                    color = p.surfaceGlass,
                    shadowElevation = 12.dp
                ) {
                    Column(
                        modifier = Modifier.padding(horizontal = 11.dp, vertical = 9.dp),
                        verticalArrangement = Arrangement.spacedBy(7.dp)
                    ) {
                        // Row 1: Compact device header + more session menu + exit stream
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(4.dp)
                        ) {
                            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                                Text(
                                    text = deviceName,
                                    color = p.textPrimary,
                                    fontSize = 13.5.sp,
                                    fontWeight = FontWeight.SemiBold,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis
                                )
                                if (routeBadge != null) {
                                    PrecisionPillBadge(text = routeBadge, tone = PillTone.EMERALD, showDot = true)
                                }
                            }
                            Box {
                                MobileAction(
                                    symbol = MobileSymbol.More,
                                    label = tr("Connection actions"),
                                    tonalWell = true,
                                    modifier = Modifier.size(38.dp)
                                ) { sideMoreOpen = true }
                                sessionActionsContent(sideMoreOpen) { sideMoreOpen = false }
                            }
                            MobileAction(
                                symbol = MobileSymbol.Disconnect,
                                label = tr("Exit stream"),
                                destructive = true,
                                modifier = Modifier.size(38.dp),
                                onClick = onDisconnect
                            )
                        }

                        // Row 2: Current source / window selector & stream settings (recessed precision slot)
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .clip(RoundedCornerShape(12.dp))
                                .background(p.surfaceSunken)
                                .border(1.dp, p.borderMuted, RoundedCornerShape(12.dp))
                                .padding(horizontal = 6.dp, vertical = 3.dp),
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Row(
                                modifier = Modifier
                                    .weight(1f)
                                    .clip(RoundedCornerShape(8.dp))
                                    .clickable(role = Role.Button) { sideDrawerOpen = false; onOpenSettings() }
                                    .padding(horizontal = 4.dp, vertical = 5.dp),
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                SourceIcon(pane.source.copy(icon = pane.appearance?.application ?: pane.source.icon))
                                Text(
                                    text = paneTitle,
                                    color = p.textPrimary,
                                    fontSize = 12.sp,
                                    fontWeight = FontWeight.Medium,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis
                                )
                            }
                            if (WorkspaceClient.cameraSwitchTarget(pane) != null) {
                                MobileAction(
                                    symbol = MobileSymbol.CameraSwitch,
                                    label = tr("Switch camera"),
                                    modifier = Modifier.size(34.dp)
                                ) {
                                    WorkspaceClient.switchCamera(pane.id, sessionKey, generation)
                                }
                            }
                            MobileAction(
                                symbol = MobileSymbol.Sources,
                                label = tr("Choose screen or app window"),
                                modifier = Modifier.size(34.dp)
                            ) {
                                sideDrawerOpen = false
                                onPickSource()
                            }
                            if (allPanes.size > 1) {
                                MobileAction(
                                    symbol = MobileSymbol.Close,
                                    label = trFormat("Close %s", paneTitle),
                                    modifier = Modifier.size(34.dp)
                                ) {
                                    WorkspaceClient.remove(pane.id)
                                }
                            }
                        }

                        if (!gridMode && allPanes.size > 1) {
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clip(RoundedCornerShape(10.dp))
                                    .background(p.surfaceSunken)
                                    .padding(3.dp)
                                    .horizontalScroll(rememberScrollState()),
                                horizontalArrangement = Arrangement.spacedBy(4.dp)
                            ) {
                                allPanes.forEach { candidate ->
                                    val active = candidate.id == selectedPaneId
                                    Box(
                                        modifier = Modifier
                                            .heightIn(min = 28.dp)
                                            .clip(RoundedCornerShape(8.dp))
                                            .background(if (active) p.surfaceRaised else Color.Transparent)
                                            .let { mod ->
                                                if (active) mod.border(1.dp, p.brandBorder, RoundedCornerShape(8.dp)) else mod
                                            }
                                            .clickable(role = Role.Tab) { WorkspaceClient.layout(false, candidate.id) }
                                            .padding(horizontal = 9.dp, vertical = 4.dp),
                                        contentAlignment = Alignment.Center
                                    ) {
                                        Text(
                                            localizedSourceTitle(candidate.source),
                                            color = if (active) p.brand else p.textSecondary,
                                            fontSize = 11.sp,
                                            fontWeight = if (active) FontWeight.SemiBold else FontWeight.Medium,
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
                                    tonalWell = true,
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
                                    tonalWell = true,
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
                                    tonalWell = true,
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
                                    tonalWell = true,
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
                                tonalWell = true,
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
                                tonalWell = true,
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
                                tonalWell = true,
                                modifier = Modifier.size(40.dp),
                                onClick = onToggleLandscape
                            )
                            MobileAction(
                                symbol = MobileSymbol.Refresh,
                                label = tr("Reset zoom"),
                                tonalWell = true,
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
        Column(Modifier.fillMaxWidth().background(p.surfaceRaised)) {
            HorizontalDivider(color = p.border, thickness = 1.dp)
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 6.dp, vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                Row(
                    Modifier.weight(1f).horizontalScroll(rememberScrollState()),
                    horizontalArrangement = Arrangement.spacedBy(5.dp),
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
                Spacer(Modifier.width(4.dp))
                MobileAction(
                    symbol = MobileSymbol.Collapse,
                    label = tr("Hide keyboard"),
                    tonalWell = true,
                    modifier = Modifier.size(42.dp)
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
}

@Composable
private fun SourceIcon(source: WorkspaceSource) {
    val p = WeaveloTokens.colors
    Box(
        modifier = Modifier
            .size(24.dp)
            .clip(RoundedCornerShape(6.dp))
            .background(p.surfaceRaised)
            .border(0.75.dp, p.borderMuted, RoundedCornerShape(6.dp)),
        contentAlignment = Alignment.Center
    ) {
        val bitmap = source.icon
        if (bitmap != null) {
            Image(
                bitmap = remember(bitmap) { bitmap.asImageBitmap() },
                contentDescription = source.application.ifBlank { source.title },
                modifier = Modifier.size(18.dp)
            )
        } else {
            Icon(
                (if (source.isCamera) MobileSymbol.Camera else if (source.isWindow) MobileSymbol.Sources else MobileSymbol.Devices).image,
                contentDescription = null,
                tint = p.brand,
                modifier = Modifier.size(15.dp)
            )
        }
    }
    Spacer(Modifier.width(6.dp))
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
    val p = WeaveloTokens.colors
    var query by remember { mutableStateOf("") }
    val quickScreen = remember(sources) { primaryScreenSource(sources) }
    val matches = remember(sources, query) {
        val hasDisplay = sources.any { (it.source as? JSONObject)?.has("Display") == true }
        sources.filter { !(hasDisplay && it.source == "MainDisplay") }
            .filter { it.title.contains(query.trim(), true) || it.application.contains(query.trim(), true) }
            .sortedBy { if (it.isCamera) 2 else if (it.isWindow) 1 else 0 }
    }
    PrecisionDialog(
        title = tr("Sources"),
        onDismissRequest = onClose,
        icon = MobileSymbol.Sources,
        maxWidth = 520.dp,
        dismissButton = if (quickScreen != null) {
            {
                PrecisionButton(
                    text = tr("Open main screen"),
                    leadingSymbol = MobileSymbol.View,
                    style = PrecisionButtonStyle.TONAL,
                    compact = true,
                    onClick = { onSelect(quickScreen) }
                )
            }
        } else null,
        confirmButton = {
            PrecisionButton(
                text = tr("Close"),
                onClick = onClose,
                style = PrecisionButtonStyle.SUBTLE,
                compact = true
            )
        }
    ) {
        PrecisionTextField(
            value = query,
            onValueChange = { query = it },
            label = tr("Search apps and windows")
        )
        LazyColumn(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(max = 310.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp)
        ) {
            itemsIndexed(matches, key = { _, source -> source.source.toString() }) { index, source ->
                Column(Modifier.fillMaxWidth()) {
                    if (index == 0 || matches[index - 1].group != source.group) {
                        Text(
                            text = tr(source.group),
                            color = p.brand,
                            fontSize = 11.5.sp,
                            fontWeight = FontWeight.Bold,
                            modifier = Modifier.padding(top = 8.dp, bottom = 4.dp)
                        )
                    }
                    val app = source.application.trim().ifBlank { "Other applications" }
                    if (source.isWindow && (index == 0 || !matches[index - 1].isWindow ||
                            !matches[index - 1].application.trim().ifBlank { "Other applications" }.equals(app, true))
                    ) {
                        Text(
                            text = if (source.application.trim().isBlank()) tr("Other applications") else app,
                            color = p.textSecondary,
                            fontSize = 11.sp,
                            fontWeight = FontWeight.SemiBold,
                            modifier = Modifier.padding(start = 6.dp, top = 6.dp, bottom = 3.dp)
                        )
                    }
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(start = if (source.isWindow) 10.dp else 0.dp)
                            .clip(RoundedCornerShape(11.dp))
                            .background(p.surfaceSunken)
                            .border(1.dp, p.borderMuted, RoundedCornerShape(11.dp))
                            .clickable(role = Role.Button) { onSelect(source) }
                            .padding(horizontal = 10.dp, vertical = 8.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        SourceIcon(source)
                        Text(
                            text = localizedSourceTitle(source),
                            color = p.textPrimary,
                            fontSize = 13.sp,
                            fontWeight = FontWeight.Medium,
                            modifier = Modifier.weight(1f),
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis
                        )
                        Icon(
                            MobileSymbol.ChevronRight.image,
                            contentDescription = null,
                            tint = p.textTertiary,
                            modifier = Modifier.size(16.dp)
                        )
                    }
                }
            }
            if (matches.isEmpty()) {
                item {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(vertical = 8.dp),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text(tr("No matching sources"), color = p.textSecondary, fontSize = 12.5.sp)
                        PrecisionButton(
                            text = tr("Clear search"),
                            style = PrecisionButtonStyle.SUBTLE,
                            compact = true,
                            onClick = { query = "" }
                        )
                    }
                }
            }
        }
    }
}
