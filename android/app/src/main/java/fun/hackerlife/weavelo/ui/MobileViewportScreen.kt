package `fun`.hackerlife.weavelo.ui

import android.util.Log
import android.view.SurfaceView
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.systemGestureExclusion
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import `fun`.hackerlife.weavelo.AudioOpusPlayer
import `fun`.hackerlife.weavelo.MediaCodecPlayer
import `fun`.hackerlife.weavelo.SessionState
import `fun`.hackerlife.weavelo.TouchMode
import `fun`.hackerlife.weavelo.VideoDimensions
import `fun`.hackerlife.weavelo.WeaveloClient
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun MobileViewportScreen(
    onDisconnect: () -> Unit
) {
    StreamOrientation(active = true)
    var touchMode by remember { mutableStateOf(TouchMode.DIRECT) }
    var lastControlMode by remember { mutableStateOf(TouchMode.DIRECT) }
    val isModifierBarVisible = WindowInsets.ime.getBottom(LocalDensity.current) > 0
    var controlsExpanded by remember { mutableStateOf(false) }
    var handleFraction by rememberSaveable { mutableFloatStateOf(0.5f) }
    var touching by remember { mutableStateOf(false) }
    LaunchedEffect(controlsExpanded) {
        if (controlsExpanded) {
            kotlinx.coroutines.delay(5000)
            controlsExpanded = false
        }
    }
    var heldModifiers by remember { mutableStateOf(emptySet<String>()) }
    val virtualKeys = remember { StickyKeyController(WeaveloClient::sendVirtualKey) }
    var showKeyboard by remember { mutableStateOf(false) }
    var textInputToken by remember { mutableStateOf<WeaveloClient.TextInputToken?>(null) }
    var imeWasVisible by remember { mutableStateOf(false) }
    LaunchedEffect(isModifierBarVisible) {
        if (isModifierBarVisible) {
            controlsExpanded = false
        } else if (imeWasVisible) {
            showKeyboard = false
            textInputToken = null
            controlsExpanded = false
        }
        imeWasVisible = isModifierBarVisible
    }
    val viewportTransform = remember { mutableStateOf(ViewportTransform(maxScale = 6.0f)) }
    var transform by viewportTransform
    val sessionState by WeaveloClient.sessionState.collectAsState()
    val telemetry by WeaveloClient.telemetry.collectAsState()
    var playbackError by remember { mutableStateOf<String?>(null) }
    var playbackErrorText by remember { mutableStateOf<`fun`.hackerlife.weavelo.AppText?>(null) }
    var decodedFps by remember { mutableStateOf<Int?>(null) }
    var videoDimensions by remember { mutableStateOf(VideoDimensions(1920, 1080)) }
    val mediaPaused by WeaveloClient.mediaPaused.collectAsState()
    val pausePending by WeaveloClient.mediaPausePending.collectAsState()
    var showStreamSettings by remember { mutableStateOf(false) }
    var fpsText by remember { mutableStateOf(WeaveloClient.streamFps.toString()) }
    var bitrateText by remember { mutableStateOf(WeaveloClient.streamBitrateKbps.toString()) }
    var keepAliveText by remember { mutableStateOf(WeaveloClient.backgroundKeepAliveSeconds.toString()) }
    var keepService by remember { mutableStateOf(WeaveloClient.backgroundServiceEnabled()) }
    var settingsError by remember { mutableStateOf<String?>(null) }
    var settingsErrorText by remember { mutableStateOf<`fun`.hackerlife.weavelo.AppText?>(null) }
    val scope = rememberCoroutineScope()

    var inputSessionToken by remember { mutableStateOf<WeaveloClient.InputIntentToken?>(null) }
    val inputEnabled = sessionState == SessionState.STREAMING && !mediaPaused && !pausePending && !showStreamSettings
    DisposableEffect(virtualKeys) { onDispose { virtualKeys.releaseAll() } }
    LaunchedEffect(inputEnabled, isModifierBarVisible) {
        if (!inputEnabled || !isModifierBarVisible) {
            virtualKeys.releaseAll()
            heldModifiers = emptySet()
        }
        if (!inputEnabled) { showKeyboard = false; textInputToken = null }
    }

    LaunchedEffect(inputEnabled) {
        while (true) {
            val current = if (inputEnabled) WeaveloClient.inputIntentToken() else null
            if ((!inputEnabled || current != null) && current != inputSessionToken) {
                virtualKeys.forgetAll()
                heldModifiers = emptySet()
                inputSessionToken = current
            }
            val composingOwner = textInputToken
            val observedOwner = if (showKeyboard && composingOwner != null) WeaveloClient.textInputToken() else null
            if (observedOwner != null && observedOwner != composingOwner) {
                showKeyboard = false; textInputToken = null
            }
            if (!inputEnabled) break
            delay(100)
        }
    }
    LaunchedEffect(inputEnabled, inputSessionToken) {
        val token = inputSessionToken
        if (inputEnabled && token != null) {
            while (true) {
                delay(100)
                if (!WeaveloClient.refreshInputIntent(token)) {
                    virtualKeys.forgetAll()
                    heldModifiers = emptySet()
                }
            }
        }
    }

    if (showStreamSettings) {
        AlertDialog(onDismissRequest = { showStreamSettings = false },
            title = { Text(tr("Stream settings")) },
            text = { Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                OutlinedTextField(value = fpsText, onValueChange = { fpsText = it },
                    label = { Text(tr("Frame rate (FPS)")) }, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number))
                OutlinedTextField(value = bitrateText, onValueChange = { bitrateText = it },
                    label = { Text(tr("Video bitrate (kbps)")) }, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number))
                OutlinedTextField(value = keepAliveText, onValueChange = { keepAliveText = it },
                    label = { Text(tr("Background keepalive (seconds)")) }, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number))
                Row(verticalAlignment = Alignment.CenterVertically) {
                    androidx.compose.material3.Checkbox(keepService, { keepService = it })
                    Text(tr("Keep this connection in background until Disconnect"))
                }
                Text(tr("When off, the idle timeout applies (0 keeps the connection). This is not a system-stop or battery exemption."))
                settingsError?.let { Text(settingsErrorText?.let { text -> appText(text) } ?: trFormat("Details: %s", it), color = MaterialTheme.colorScheme.error) }
            } },
            confirmButton = { TextButton(onClick = {
                val fps = fpsText.toIntOrNull()
                val bitrate = bitrateText.toIntOrNull()
                val keepAlive = keepAliveText.toIntOrNull()
                if (fps == null || bitrate == null || fps <= 0 || bitrate <= 0 || keepAlive == null || keepAlive !in 0..86400) {
                    settingsError = "Use positive FPS/kbps and 0–86400 keepalive seconds"
                    settingsErrorText = `fun`.hackerlife.weavelo.AppText("Use positive FPS/kbps and 0–86400 keepalive seconds")
                } else scope.launch {
                    settingsErrorText = null
                    settingsError = withContext(Dispatchers.IO) { WeaveloClient.updateStreamRates(fps, bitrate) }
                    if (settingsError == null) settingsError = WeaveloClient.updateBackgroundKeepAlive(keepAlive)
                    if (settingsError == null) { WeaveloClient.updateBackgroundServiceEnabled(keepService); showStreamSettings = false }
                }
            }) { Text(tr("Apply")) } },
            dismissButton = { TextButton(onClick = { showStreamSettings = false }) { Text(tr("Cancel")) } })
    }

    Column(Modifier.fillMaxSize().imePadding().background(Color.Black)) {
        BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
            val viewportHeightPx = with(LocalDensity.current) { maxHeight.toPx().coerceAtLeast(1f) }
            val gestureOwner = inputSessionToken
            RemoteViewport(
                description = tr("Remote screen"),
                dimensions = videoDimensions,
                transformState = viewportTransform,
                expandToContainer = true,
                enabled = inputEnabled,
                contentKey = videoDimensions to inputSessionToken,
                modifier = Modifier.fillMaxSize(),
                touchMode = touchMode,
                sendTouch = { action, id, x, y, pressure, eventTime ->
                    touching = action == 0 || action == 1
                    if (action == 0 && controlsExpanded) controlsExpanded = false
                    if (gestureOwner != null) {
                        WeaveloClient.sendTouchEvent(gestureOwner, action, id, x, y, pressure, eventTime)
                    }
                },
                overlay = {},
                factory = { context ->
                    SurfaceView(context).apply {
                        holder.addCallback(object : android.view.SurfaceHolder.Callback {
                            var feeder: Thread? = null
                            var running = AtomicBoolean(false)
                            override fun surfaceCreated(holder: android.view.SurfaceHolder) {
                                val active = AtomicBoolean(true)
                                running = active
                                playbackError = null
                                playbackErrorText = null
                                decodedFps = null
                                feeder = Thread {
                                    val codec = MediaCodecPlayer(holder.surface) { dimensions ->
                                        post {
                                            if (active.get()) {
                                                videoDimensions = dimensions
                                                WeaveloClient.setScreenBounds(dimensions.width, dimensions.height)
                                            }
                                        }
                                    }
                                    val audioPlayer = AudioOpusPlayer()
                                    var audioEnabled = true
                                    fun report(message: String, error: Exception, text: `fun`.hackerlife.weavelo.AppText) {
                                        Log.e("Weavelo", message, error)
                                        post { if (active.get()) { playbackError = message; playbackErrorText = text } }
                                    }
                                    try {
                                        codec.start(1920, 1080)
                                        if (active.get()) WeaveloClient.setViewerSurfaceReady(true)
                                        var waitingForKeyframe = true
                                        var pending: `fun`.hackerlife.weavelo.EncodedVideoFrame? = null
                                        var pendingSince = 0L
                                        var lastKeyframeRequest = System.nanoTime()
                                        var sampleTime = lastKeyframeRequest
                                        var sampleFrames = 0L
                                        var pauseHandled = false
                                        WeaveloClient.requestKeyframe()
                                        while (active.get() && !Thread.currentThread().isInterrupted) {
                                            if (WeaveloClient.mediaPaused.value) {
                                                if (!pauseHandled) {
                                                    pending = null
                                                    waitingForKeyframe = true
                                                    codec.flush()
                                                    audioPlayer.stop()
                                                    pauseHandled = true
                                                }
                                                Thread.sleep(50)
                                                continue
                                            }
                                            val now = System.nanoTime()
                                            if (pauseHandled) {
                                                sampleTime = now
                                                sampleFrames = codec.outputFrames
                                                pauseHandled = false
                                            }
                                            if (pending == null) {
                                                val frame = WeaveloClient.pollVideoFrame()
                                                if (frame != null && (!waitingForKeyframe || frame.keyframe)) {
                                                    pending = frame
                                                    pendingSince = now
                                                }
                                            }
                                            val frame = pending
                                            if (frame != null) {
                                                if (codec.feedNalu(frame.data, frame.keyframe, frame.ptsUs, frame.dataOffset)) {
                                                    pending = null
                                                    waitingForKeyframe = false
                                                } else if (now - pendingSince > 100_000_000L) {
                                                    pending = null
                                                    waitingForKeyframe = true
                                                }
                                            }
                                            codec.drain()
                                            if (now - sampleTime >= 500_000_000L) {
                                                val fps = ((codec.outputFrames - sampleFrames) * 1_000_000_000.0 / (now - sampleTime)).toInt()
                                                post { if (active.get()) decodedFps = fps }
                                                sampleTime = now
                                                sampleFrames = codec.outputFrames
                                            }
                                            if (waitingForKeyframe && now - lastKeyframeRequest >= 1_000_000_000L) {
                                                WeaveloClient.requestKeyframe()
                                                lastKeyframeRequest = now
                                            }
                                            val audioPacket = WeaveloClient.pollAudioPacket()
                                            if (audioEnabled) {
                                                try {
                                                    if (audioPacket != null) audioPlayer.feed(audioPacket)
                                                    audioPlayer.drain()
                                                } catch (e: Exception) {
                                                    audioEnabled = false
                                                    audioPlayer.stop()
                                                    report("Audio unavailable: ${e.message}", e, `fun`.hackerlife.weavelo.AppText("Audio unavailable: %s", listOf(e.message ?: "")))
                                                }
                                            }
                                            if (frame == null || pending != null) Thread.sleep(4)
                                        }
                                    } catch (_: InterruptedException) {
                                        Thread.currentThread().interrupt()
                                    } catch (e: Exception) {
                                        report("Video unavailable: ${e.message}", e, `fun`.hackerlife.weavelo.AppText("Video unavailable: %s", listOf(e.message ?: "")))
                                    } finally {
                                        codec.stop()
                                        audioPlayer.stop()
                                    }
                                }.apply { name = "weavelo-media"; start() }
                            }
                            override fun surfaceChanged(holder: android.view.SurfaceHolder, format: Int, width: Int, height: Int) {
                                WeaveloClient.setScreenBounds(videoDimensions.width, videoDimensions.height)
                            }
                            override fun surfaceDestroyed(holder: android.view.SurfaceHolder) {
                                running.set(false)
                                WeaveloClient.setViewerSurfaceReady(false)
                                feeder?.interrupt()
                                feeder = null
                            }
                        })
                    }
                }
            )

            val keyboardToken = textInputToken
            RemoteKeyboard(identity = keyboardToken ?: "no-input", visible = showKeyboard,
                enabled = inputEnabled && keyboardToken != null,
                onText = { text -> keyboardToken?.let { if (!WeaveloClient.commitText(it,text)) { playbackError="Text input source changed; reopen the keyboard"; playbackErrorText = `fun`.hackerlife.weavelo.AppText("Text input source changed; reopen the keyboard") } } },
                onKey = { key, pressed -> keyboardToken?.let { if (!WeaveloClient.textKey(it,key,pressed)) { playbackError="Keyboard input is unavailable"; playbackErrorText = `fun`.hackerlife.weavelo.AppText("Keyboard input is unavailable") } } })

            playbackError?.let { message ->
                Text(playbackErrorText?.let { appText(it) } ?: trFormat("Details: %s", message), color = Color.White,
                    modifier = Modifier.align(Alignment.Center)
                        .background(Color.Black.copy(alpha = 0.85f)).padding(16.dp))
            }

            NetworkStatusChip(sessionState, telemetry,
                modifier = Modifier.align(Alignment.BottomEnd).safeDrawingPadding().padding(8.dp))

            if (!isModifierBarVisible && transform.scale > 1.05f) {
                ZoomedCanvasChip(
                    scale = transform.scale,
                    panActive = touchMode == TouchMode.VIEW,
                    canTogglePan = inputEnabled,
                    modifier = Modifier.align(Alignment.TopStart).safeDrawingPadding().padding(8.dp),
                    onTogglePan = {
                        if (touchMode == TouchMode.VIEW) {
                            touchMode = lastControlMode
                            WeaveloClient.setTouchMode(touchMode)
                        } else {
                            lastControlMode = touchMode
                            touchMode = TouchMode.VIEW
                        }
                    },
                    onResetZoom = {
                        transform = transform.reset()
                        if (inputEnabled && touchMode == TouchMode.VIEW) {
                            touchMode = lastControlMode
                            WeaveloClient.setTouchMode(touchMode)
                        }
                    }
                )
            }

            // Right-edge magnetic handle + compact right-side drawer (no top bars!)
            if (!isModifierBarVisible) {
                val pillAlpha = ZeroOcclusionHudPolicy.edgePillAlpha(touching = touching, imeVisible = false)
                Box(
                    Modifier
                        .align(Alignment.CenterEnd)
                        .offset { IntOffset(0, ((handleFraction - .5f) * viewportHeightPx).toInt()) }
                        .size(width = 44.dp, height = 64.dp)
                        .systemGestureExclusion { coordinates ->
                            Rect(0f, 0f, coordinates.size.width.toFloat(), coordinates.size.height.toFloat())
                        }
                        .pointerInput(viewportHeightPx) {
                            var horizontal = 0f
                            detectDragGestures(onDragStart = { horizontal = 0f }, onDragEnd = {
                                if (ZeroOcclusionHudPolicy.revealsControls(horizontal, 24.dp.toPx())) {
                                    controlsExpanded = true
                                }
                            }) { change, drag ->
                                change.consume(); horizontal += drag.x
                                handleFraction = ZeroOcclusionHudPolicy.dragEdgePillFraction(handleFraction, drag.y, viewportHeightPx)
                            }
                        }
                        .clickable(onClickLabel = tr("Show remote controls")) {
                            controlsExpanded = !controlsExpanded
                        }
                        .padding(start = 36.dp, end = 2.dp, top = 10.dp, bottom = 10.dp)
                        .background(ColorAccentCyan.copy(alpha = pillAlpha), RoundedCornerShape(6.dp))
                )

                if (controlsExpanded) {
                    Surface(
                        modifier = Modifier
                            .align(Alignment.CenterEnd)
                            .safeDrawingPadding()
                            .padding(end = 42.dp, top = 8.dp, bottom = 8.dp)
                            .widthIn(min = 220.dp, max = 276.dp),
                        shape = RoundedCornerShape(20.dp),
                        color = ColorSurfaceCard.copy(alpha = .95f),
                        shadowElevation = 8.dp
                    ) {
                        Column(
                            modifier = Modifier.padding(horizontal = 10.dp, vertical = 8.dp),
                            verticalArrangement = Arrangement.spacedBy(6.dp)
                        ) {
                            Row(
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.SpaceBetween,
                                modifier = Modifier.fillMaxWidth()
                            ) {
                                Row(
                                    verticalAlignment = Alignment.CenterVertically,
                                    horizontalArrangement = Arrangement.spacedBy(6.dp)
                                ) {
                                    Box(modifier = Modifier.size(6.dp).background(ColorStatusGreen, CircleShape))
                                    Text(
                                        text = if (sessionState == SessionState.RECONNECTING) tr("Reconnecting…")
                                            else if (pausePending) tr("Waiting for host…")
                                            else if (mediaPaused) tr("Paused · connected")
                                            else decodedFps?.let { trFormat("Decoded %s FPS", it) } ?: tr("Remote session"),
                                        color = ColorTextPrimary,
                                        fontSize = 11.sp,
                                        fontWeight = FontWeight.SemiBold
                                    )
                                }
                                TouchIslandIconButton(text = tr("DISCONNECT"), active = false, isDestructive = true) {
                                    virtualKeys.releaseAll()
                                    onDisconnect()
                                }
                            }
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.SpaceBetween,
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                TextButton(onClick = {
                                    WeaveloClient.setManualMediaPaused(!WeaveloClient.manualMediaPaused)
                                }, contentPadding = PaddingValues(horizontal = 6.dp, vertical = 2.dp)) {
                                    Text(if (WeaveloClient.manualMediaPaused) tr("Resume") else tr("Pause"))
                                }
                                TextButton(onClick = {
                                    fpsText = WeaveloClient.streamFps.toString()
                                    bitrateText = WeaveloClient.streamBitrateKbps.toString()
                                    settingsError = null
                                    controlsExpanded = false
                                    showStreamSettings = true
                                }, contentPadding = PaddingValues(horizontal = 6.dp, vertical = 2.dp)) {
                                    Text(tr("Stream settings"))
                                }
                                LanguageSelector()
                            }
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.SpaceBetween,
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                MobileAction(MobileSymbol.Trackpad, tr("Trackpad"), inputEnabled, Modifier.size(40.dp),
                                    selected = touchMode == TouchMode.TRACKPAD) {
                                    lastControlMode = TouchMode.TRACKPAD; touchMode = TouchMode.TRACKPAD
                                    WeaveloClient.setTouchMode(touchMode)
                                }
                                MobileAction(MobileSymbol.Touch, tr("Direct touch"), inputEnabled, Modifier.size(40.dp),
                                    selected = touchMode == TouchMode.DIRECT) {
                                    lastControlMode = TouchMode.DIRECT; touchMode = TouchMode.DIRECT
                                    WeaveloClient.setTouchMode(touchMode)
                                }
                                MobileAction(MobileSymbol.Pan, tr("Pan canvas"), true, Modifier.size(40.dp),
                                    selected = touchMode == TouchMode.VIEW) {
                                    if (touchMode == TouchMode.VIEW && inputEnabled) {
                                        touchMode = lastControlMode
                                        WeaveloClient.setTouchMode(touchMode)
                                    } else {
                                        if (touchMode != TouchMode.VIEW) lastControlMode = touchMode
                                        touchMode = TouchMode.VIEW
                                    }
                                }
                                MobileAction(MobileSymbol.Keyboard, tr("Show keyboard"), inputEnabled, Modifier.size(40.dp)) {
                                    controlsExpanded = false
                                    if (showKeyboard) {
                                        showKeyboard = false; textInputToken = null
                                    } else {
                                        textInputToken = WeaveloClient.textInputToken()
                                        showKeyboard = textInputToken != null
                                    }
                                }
                                MobileAction(MobileSymbol.Refresh, tr("Reset zoom"), modifier = Modifier.size(40.dp)) {
                                    transform = transform.reset()
                                    if (inputEnabled && touchMode == TouchMode.VIEW) {
                                        touchMode = lastControlMode
                                        WeaveloClient.setTouchMode(touchMode)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if (isModifierBarVisible) {
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
                        VirtualKeyPill(name = key, active = key.lowercase() in heldModifiers, enabled = inputEnabled) {
                            virtualKeys.tap(key, inputEnabled); heldModifiers = virtualKeys.selected()
                        }
                    }
                }
                MobileAction(MobileSymbol.Collapse, tr("Hide keyboard"), modifier = Modifier.size(40.dp)) {
                    showKeyboard = false; textInputToken = null; controlsExpanded = false
                }
            }
        }
    }
}

@Composable
fun TouchIslandIconButton(
    text: String,
    active: Boolean,
    isDestructive: Boolean = false,
    enabled: Boolean = true,
    onClick: () -> Unit
) {
    Box(
        modifier = Modifier
            .clip(RoundedCornerShape(9999.dp))
            .background(
                when {
                    isDestructive -> Color(0xFFFFEBE9)
                    active -> Color(0xFFEFF6FF)
                    else -> Color(0xFFF1F5F9)
                }
            )
            .border(
                1.dp,
                when {
                    isDestructive -> Color(0xFFFF5252)
                    active -> ColorAccentCyan
                    else -> ColorBorderFine
                },
                RoundedCornerShape(9999.dp)
            )
            .sizeIn(minWidth = 44.dp, minHeight = 44.dp)
            .clickable(enabled = enabled, onClick = onClick)
            .padding(horizontal = 8.dp, vertical = 4.dp),
        contentAlignment = Alignment.Center
    ) {
        Text(
            text = text,
            color = when {
                isDestructive -> Color(0xFFB42318)
                active -> ColorAccentCyan
                else -> ColorTextSecondary
            },
            fontSize = 9.sp,
            fontWeight = FontWeight.Bold,
            fontFamily = FontFamily.Monospace
        )
    }
}

@Composable
fun VirtualKeyPill(name: String, active: Boolean = false, enabled: Boolean = true, onClick: () -> Unit) {
    Box(
        modifier = Modifier
            .clip(RoundedCornerShape(6.dp))
            .background(if (active) Color(0xFFEFF6FF) else Color(0xFFF1F5F9))
            .border(1.dp, if (active) ColorAccentCyan else ColorBorderFine, RoundedCornerShape(6.dp))
            .sizeIn(minWidth = 44.dp, minHeight = 44.dp)
            .clickable(enabled = enabled, onClick = onClick)
            .padding(horizontal = 8.dp, vertical = 6.dp),
        contentAlignment = Alignment.Center
    ) {
        Text(
            text = name,
            color = ColorTextPrimary,
            fontSize = 10.sp,
            fontFamily = FontFamily.Monospace,
            fontWeight = FontWeight.Medium
        )
    }
}
