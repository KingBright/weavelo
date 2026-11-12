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
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
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
    val p = WeaveloTokens.colors
    var landscapeOverride by rememberSaveable { mutableStateOf<Boolean?>(null) }
    StreamOrientation(active = true, landscapeOverride = landscapeOverride)
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
    var baseMeasured by remember { mutableStateOf(androidx.compose.ui.unit.IntSize.Zero) }
    var cursorPos by remember { mutableStateOf(0.5f to 0.5f) }
    val naturalScroll by WeaveloClient.naturalScrollFlow.collectAsState()
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
        PrecisionDialog(
            title = tr("Stream settings"),
            onDismissRequest = { showStreamSettings = false },
            icon = MobileSymbol.Settings,
            dismissButton = {
                PrecisionButton(
                    text = tr("Cancel"),
                    onClick = { showStreamSettings = false },
                    style = PrecisionButtonStyle.SUBTLE,
                    compact = true
                )
            },
            confirmButton = {
                PrecisionButton(
                    text = tr("Apply"),
                    onClick = {
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
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    PrecisionTextField(
                        value = fpsText,
                        onValueChange = { fpsText = it },
                        label = tr("Frame rate (FPS)"),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                        modifier = Modifier.weight(1f)
                    )
                    PrecisionTextField(
                        value = bitrateText,
                        onValueChange = { bitrateText = it },
                        label = tr("Video bitrate (kbps)"),
                        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                        modifier = Modifier.weight(1f)
                    )
                }
                PrecisionTextField(
                    value = keepAliveText,
                    onValueChange = { keepAliveText = it },
                    label = tr("Background keepalive (seconds)"),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number)
                )
                HorizontalDivider(color = p.borderMuted)
                PrecisionSwitchRow(
                    title = tr("Natural scrolling (Apple style)"),
                    subtitle = tr("Content moves in the same direction as your fingers"),
                    checked = naturalScroll,
                    onCheckedChange = { WeaveloClient.updateNaturalScroll(it) },
                    leadingSymbol = MobileSymbol.Touch
                )
                PrecisionSwitchRow(
                    title = tr("Keep this connection in background until Disconnect"),
                    subtitle = tr("When off, the idle timeout applies (0 keeps the connection). This is not a system-stop or battery exemption."),
                    checked = keepService,
                    onCheckedChange = { keepService = it },
                    leadingSymbol = MobileSymbol.Network
                )
                settingsError?.let {
                    Text(
                        text = settingsErrorText?.let { text -> appText(text) } ?: trFormat("Details: %s", it),
                        color = p.dangerText,
                        fontSize = 12.sp
                    )
                }
            }
        }
    }

    Column(Modifier.fillMaxSize().imePadding().background(Color(0xFF070A0F))) {
        BoxWithConstraints(Modifier.weight(1f).fillMaxWidth()) {
            val portraitViewport = maxHeight > maxWidth
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
                onViewportSizeChanged = { baseMeasured = it },
                sendTouch = { action, id, x, y, pressure, eventTime ->
                    touching = action == 0 || action == 1
                    if (action == 0 && controlsExpanded) controlsExpanded = false
                    if (gestureOwner != null) {
                        WeaveloClient.sendTouchEvent(gestureOwner, action, id, x, y, pressure, eventTime)
                        if (touchMode == TouchMode.TRACKPAD) {
                            cursorPos = WeaveloClient.cursorPosition()
                        }
                    }
                },
                overlay = {
                    if (inputEnabled && touchMode == TouchMode.TRACKPAD) {
                        WeaveloPointerOverlay(
                            normX = cursorPos.first,
                            normY = cursorPos.second,
                            pressing = touching,
                            transform = transform,
                            baseWidth = baseMeasured.width.coerceAtLeast(1).toFloat(),
                            baseHeight = baseMeasured.height.coerceAtLeast(1).toFloat()
                        )
                    }
                },
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
                                            var didWork = false
                                            if (pending == null) {
                                                var latestKeyframe: `fun`.hackerlife.weavelo.EncodedVideoFrame? = null
                                                val batch = ArrayList<`fun`.hackerlife.weavelo.EncodedVideoFrame>(12)
                                                while (batch.size < 12) {
                                                    val next = WeaveloClient.pollVideoFrame() ?: break
                                                    if (next.keyframe) latestKeyframe = next
                                                    batch.add(next)
                                                }
                                                if (batch.isNotEmpty()) {
                                                    didWork = true
                                                    val startIdx = if (latestKeyframe != null) batch.indexOfLast { it === latestKeyframe }.coerceAtLeast(0) else 0
                                                    for (idx in startIdx until batch.size) {
                                                        val candidate = batch[idx]
                                                        if (waitingForKeyframe && !candidate.keyframe) continue
                                                        if (codec.feedNalu(candidate.data, candidate.keyframe, candidate.ptsUs, candidate.dataOffset)) {
                                                            waitingForKeyframe = false
                                                        } else {
                                                            pending = candidate
                                                            pendingSince = now
                                                            break
                                                        }
                                                    }
                                                }
                                            } else {
                                                val frame = pending
                                                if (frame != null) {
                                                    if (codec.feedNalu(frame.data, frame.keyframe, frame.ptsUs, frame.dataOffset)) {
                                                        pending = null
                                                        waitingForKeyframe = false
                                                        didWork = true
                                                    } else if (now - pendingSince > 80_000_000L) {
                                                        pending = null
                                                        waitingForKeyframe = true
                                                    }
                                                }
                                            }
                                            if (codec.drain()) didWork = true
                                            if (now - sampleTime >= 500_000_000L) {
                                                val fps = ((codec.outputFrames - sampleFrames) * 1_000_000_000.0 / (now - sampleTime)).toInt()
                                                post { if (active.get()) decodedFps = fps }
                                                sampleTime = now
                                                sampleFrames = codec.outputFrames
                                            }
                                            if (waitingForKeyframe && now - lastKeyframeRequest >= 500_000_000L) {
                                                WeaveloClient.requestKeyframe()
                                                lastKeyframeRequest = now
                                            }
                                            val audioPacket = WeaveloClient.pollAudioPacket()
                                            if (audioPacket != null) didWork = true
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
                                            if (!didWork) java.util.concurrent.locks.LockSupport.parkNanos(400_000L)
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
                Surface(
                    modifier = Modifier
                        .align(Alignment.Center)
                        .padding(16.dp)
                        .border(1.dp, p.dangerBorder, RoundedCornerShape(16.dp)),
                    shape = RoundedCornerShape(16.dp),
                    color = p.surfaceGlass,
                    shadowElevation = 8.dp
                ) {
                    Text(
                        text = playbackErrorText?.let { appText(it) } ?: trFormat("Details: %s", message),
                        color = p.dangerText,
                        fontSize = 13.sp,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp)
                    )
                }
            }

            NetworkStatusChip(sessionState, telemetry,
                modifier = Modifier.align(Alignment.BottomEnd).safeDrawingPadding().padding(8.dp))

            if (!isModifierBarVisible) {
                val orientationAlpha = ZeroOcclusionHudPolicy.edgePillAlpha(touching = touching, imeVisible = false)
                StreamOrientationToggleButton(
                    isLandscape = !portraitViewport,
                    alpha = orientationAlpha,
                    modifier = Modifier
                        .align(Alignment.BottomStart)
                        .safeDrawingPadding()
                        .padding(8.dp),
                    onClick = {
                        landscapeOverride = if (!portraitViewport) false else true
                    }
                )
            }

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
                        .background(p.brand.copy(alpha = pillAlpha), RoundedCornerShape(6.dp))
                )

                if (controlsExpanded) {
                    Surface(
                        modifier = Modifier
                            .align(Alignment.CenterEnd)
                            .safeDrawingPadding()
                            .padding(end = 42.dp, top = 8.dp, bottom = 8.dp)
                            .widthIn(min = 232.dp, max = 286.dp)
                            .border(1.dp, p.borderStrong, RoundedCornerShape(22.dp)),
                        shape = RoundedCornerShape(22.dp),
                        color = p.surfaceGlass,
                        shadowElevation = 12.dp
                    ) {
                        Column(
                            modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
                            verticalArrangement = Arrangement.spacedBy(8.dp)
                        ) {
                            Row(
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.SpaceBetween,
                                modifier = Modifier.fillMaxWidth()
                            ) {
                                Row(
                                    verticalAlignment = Alignment.CenterVertically,
                                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                                    modifier = Modifier.weight(1f)
                                ) {
                                    Box(modifier = Modifier.size(7.dp).background(p.emeraldDot, CircleShape))
                                    Text(
                                        text = if (sessionState == SessionState.RECONNECTING) tr("Reconnecting…")
                                            else if (pausePending) tr("Waiting for host…")
                                            else if (mediaPaused) tr("Paused · connected")
                                            else decodedFps?.let { trFormat("Decoded %s FPS", it) } ?: tr("Remote session"),
                                        color = p.textPrimary,
                                        fontSize = 11.5.sp,
                                        fontWeight = FontWeight.SemiBold,
                                        maxLines = 1
                                    )
                                }
                                MobileAction(
                                    symbol = MobileSymbol.Disconnect,
                                    label = tr("Exit stream"),
                                    destructive = true,
                                    modifier = Modifier.size(38.dp)
                                ) {
                                    virtualKeys.releaseAll()
                                    onDisconnect()
                                }
                            }
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.spacedBy(6.dp),
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                PrecisionButton(
                                    text = if (WeaveloClient.manualMediaPaused) tr("Resume") else tr("Pause"),
                                    onClick = { WeaveloClient.setManualMediaPaused(!WeaveloClient.manualMediaPaused) },
                                    style = PrecisionButtonStyle.SUBTLE,
                                    compact = true
                                )
                                PrecisionButton(
                                    text = tr("Stream settings"),
                                    leadingSymbol = MobileSymbol.Settings,
                                    onClick = {
                                        fpsText = WeaveloClient.streamFps.toString()
                                        bitrateText = WeaveloClient.streamBitrateKbps.toString()
                                        settingsError = null
                                        controlsExpanded = false
                                        showStreamSettings = true
                                    },
                                    style = PrecisionButtonStyle.TONAL,
                                    compact = true,
                                    modifier = Modifier.weight(1f)
                                )
                            }
                            LanguageSelector(modifier = Modifier.fillMaxWidth())
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clip(RoundedCornerShape(15.dp))
                                    .background(if (p.isDark) Color.White.copy(alpha = 0.055f) else p.surfaceSunken.copy(alpha = 0.85f))
                                    .padding(horizontal = 4.dp, vertical = 3.dp),
                                horizontalArrangement = Arrangement.SpaceEvenly,
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                MobileAction(MobileSymbol.Trackpad, tr("Trackpad"), inputEnabled, Modifier.size(42.dp),
                                    selected = touchMode == TouchMode.TRACKPAD) {
                                    lastControlMode = TouchMode.TRACKPAD; touchMode = TouchMode.TRACKPAD
                                    WeaveloClient.setTouchMode(touchMode)
                                }
                                MobileAction(MobileSymbol.Touch, tr("Direct touch"), inputEnabled, Modifier.size(42.dp),
                                    selected = touchMode == TouchMode.DIRECT) {
                                    lastControlMode = TouchMode.DIRECT; touchMode = TouchMode.DIRECT
                                    WeaveloClient.setTouchMode(touchMode)
                                }
                                MobileAction(MobileSymbol.Pan, tr("Pan canvas"), true, Modifier.size(42.dp),
                                    selected = touchMode == TouchMode.VIEW) {
                                    if (touchMode == TouchMode.VIEW && inputEnabled) {
                                        touchMode = lastControlMode
                                        WeaveloClient.setTouchMode(touchMode)
                                    } else {
                                        if (touchMode != TouchMode.VIEW) lastControlMode = touchMode
                                        touchMode = TouchMode.VIEW
                                    }
                                }
                                MobileAction(MobileSymbol.Keyboard, tr("Show keyboard"), inputEnabled, Modifier.size(42.dp)) {
                                    controlsExpanded = false
                                    if (showKeyboard) {
                                        showKeyboard = false; textInputToken = null
                                    } else {
                                        textInputToken = WeaveloClient.textInputToken()
                                        showKeyboard = textInputToken != null
                                    }
                                }
                                MobileAction(MobileSymbol.Refresh, tr("Reset zoom"), modifier = Modifier.size(42.dp)) {
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
                            VirtualKeyPill(name = key, active = key.lowercase() in heldModifiers, enabled = inputEnabled) {
                                virtualKeys.tap(key, inputEnabled); heldModifiers = virtualKeys.selected()
                            }
                        }
                    }
                    Spacer(Modifier.width(4.dp))
                    MobileAction(MobileSymbol.Collapse, tr("Hide keyboard"), tonalWell = true, modifier = Modifier.size(42.dp)) {
                        showKeyboard = false; textInputToken = null; controlsExpanded = false
                    }
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
    val p = WeaveloTokens.colors
    val shape = RoundedCornerShape(999.dp)
    Box(
        modifier = Modifier
            .clip(shape)
            .background(
                when {
                    isDestructive -> p.dangerSoft
                    active -> p.brandSoft
                    else -> p.surfaceSunken
                }
            )
            .border(
                1.dp,
                when {
                    isDestructive -> p.dangerBorder
                    active -> p.brandBorder
                    else -> p.border
                },
                shape
            )
            .sizeIn(minWidth = 44.dp, minHeight = 34.dp)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .padding(horizontal = 10.dp, vertical = 5.dp),
        contentAlignment = Alignment.Center
    ) {
        Text(
            text = text,
            color = when {
                isDestructive -> p.dangerText
                active -> p.brand
                else -> p.textSecondary
            },
            fontSize = 9.5.sp,
            fontWeight = FontWeight.Bold,
            fontFamily = FontFamily.Monospace
        )
    }
}

@Composable
fun VirtualKeyPill(name: String, active: Boolean = false, enabled: Boolean = true, onClick: () -> Unit) {
    val p = WeaveloTokens.colors
    val shape = RoundedCornerShape(9.dp)
    Box(
        modifier = Modifier
            .clip(shape)
            .background(if (active) p.brandSoft else p.surfaceSunken)
            .border(1.dp, if (active) p.brand else p.border, shape)
            .sizeIn(minWidth = 42.dp, minHeight = 36.dp)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .padding(horizontal = 9.dp, vertical = 6.dp),
        contentAlignment = Alignment.Center
    ) {
        Text(
            text = name,
            color = if (active) p.brand else p.textPrimary,
            fontSize = 10.5.sp,
            fontFamily = FontFamily.Monospace,
            fontWeight = if (active) FontWeight.Bold else FontWeight.SemiBold
        )
    }
}
