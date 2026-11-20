package `fun`.hackerlife.weavelo.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import `fun`.hackerlife.weavelo.KeymapCatalog
import `fun`.hackerlife.weavelo.KeymapCategory
import `fun`.hackerlife.weavelo.KeymapControl
import `fun`.hackerlife.weavelo.KeymapProfile
import `fun`.hackerlife.weavelo.KeymapStore
import `fun`.hackerlife.weavelo.VirtualControlKind
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.util.Locale
import kotlin.math.hypot
import kotlin.math.min
import kotlin.math.roundToInt

/**
 * Full-viewport virtual keymap HUD rendered over RemoteVideoSurface.
 *
 * In Play Mode (`editMode = false`), each control forwards pointer press/release, WASD stick,
 * look-pad delta, scroll-wheel delta, combo, and macro-burst events to the host session.
 * In Edit Mode (`editMode = true`), controls can be dragged to reposition and global HUD
 * opacity can be adjusted before saving.
 */
@Composable
fun KeymapOverlayHud(
    profile: KeymapProfile,
    modifier: Modifier = Modifier,
    editMode: Boolean = false,
    onKeyEvent: (key: String, pressed: Boolean) -> Unit,
    onMouseButton: (button: String, pressed: Boolean) -> Unit = { _, _ -> },
    onLookDelta: (dx: Float, dy: Float) -> Unit = { _, _ -> },
    onScrollDelta: (dy: Float) -> Unit = {},
    onProfileUpdated: ((KeymapProfile) -> Unit)? = null,
    onRequestPicker: (() -> Unit)? = null,
    onExitEditMode: (() -> Unit)? = null,
) {
    val p = WeaveloTokens.colors
    var draftProfile by remember(profile, editMode) { mutableStateOf(profile) }
    var selectedControlId by remember(profile.id, editMode) { mutableStateOf<String?>(null) }
    val activeProfile = if (editMode) draftProfile else profile

    BoxWithConstraints(modifier = modifier.fillMaxSize()) {
        val density = LocalDensity.current
        val viewportWidthPx = with(density) { maxWidth.toPx().coerceAtLeast(1f) }
        val viewportHeightPx = with(density) { maxHeight.toPx().coerceAtLeast(1f) }
        val minDimPx = min(viewportWidthPx, viewportHeightPx)

        if (editMode) {
            EditGridOverlay(modifier = Modifier.fillMaxSize())
        }

        for (control in activeProfile.controls) {
            val controlSizePx = (control.size * minDimPx).coerceIn(
                with(density) { 40.dp.toPx() },
                with(density) { 188.dp.toPx() },
            )
            val widthPx = when (control.kind) {
                VirtualControlKind.SCROLL_WHEEL -> controlSizePx * 0.52f
                VirtualControlKind.COMBO_BUTTON -> controlSizePx * 1.24f
                else -> controlSizePx
            }
            val heightPx = when (control.kind) {
                VirtualControlKind.SCROLL_WHEEL -> controlSizePx * 1.65f
                else -> controlSizePx
            }
            val widthDp = with(density) { widthPx.toDp() }
            val heightDp = with(density) { heightPx.toDp() }
            val leftPx = (control.x * viewportWidthPx - widthPx / 2f)
                .coerceIn(0f, (viewportWidthPx - widthPx).coerceAtLeast(0f))
            val topPx = (control.y * viewportHeightPx - heightPx / 2f)
                .coerceIn(0f, (viewportHeightPx - heightPx).coerceAtLeast(0f))
            val effectiveAlpha = (activeProfile.opacity * control.opacity).coerceIn(0.18f, 1.0f)

            Box(
                modifier = Modifier
                    .offset { IntOffset(leftPx.roundToInt(), topPx.roundToInt()) }
                    .size(width = widthDp, height = heightDp)
                    .alpha(effectiveAlpha),
            ) {
                if (editMode) {
                    EditableControlSlot(
                        control = control,
                        selected = control.id == selectedControlId,
                        onSelect = { selectedControlId = control.id },
                        onDragDelta = { dragAmount ->
                            selectedControlId = control.id
                            val nextX = (control.x + dragAmount.x / viewportWidthPx).coerceIn(0.04f, 0.96f)
                            val nextY = (control.y + dragAmount.y / viewportHeightPx).coerceIn(0.04f, 0.96f)
                            draftProfile = draftProfile.copy(
                                controls = draftProfile.controls.map { item ->
                                    if (item.id == control.id) item.copy(x = nextX, y = nextY) else item
                                },
                            )
                        },
                    )
                } else {
                    PlayableVirtualControl(
                        control = control,
                        onKeyEvent = onKeyEvent,
                        onMouseButton = onMouseButton,
                        onLookDelta = onLookDelta,
                        onScrollDelta = onScrollDelta,
                    )
                }
            }
        }

        if (editMode) {
            val selectedControl = draftProfile.controls.firstOrNull { it.id == selectedControlId }
            Surface(
                modifier = Modifier
                    .align(Alignment.TopCenter)
                    .padding(horizontal = 12.dp, vertical = 10.dp)
                    .border(1.dp, p.brandBorder, RoundedCornerShape(18.dp)),
                shape = RoundedCornerShape(18.dp),
                color = p.surfaceGlass,
                shadowElevation = 10.dp,
            ) {
                Column(
                    modifier = Modifier.padding(horizontal = 14.dp, vertical = 10.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        PrecisionPillBadge(
                            text = "HUD Edit",
                            tone = PillTone.BRAND,
                            showDot = true,
                        )
                        Text(
                            text = draftProfile.name,
                            color = p.textPrimary,
                            fontSize = 13.sp,
                            fontWeight = FontWeight.SemiBold,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.weight(1f, fill = false),
                        )
                        if (selectedControl != null) {
                            PrecisionPillBadge(
                                text = String.format(
                                    Locale.US,
                                    "%s (%.0f%%, %.0f%%)",
                                    selectedControl.label,
                                    selectedControl.x * 100f,
                                    selectedControl.y * 100f,
                                ),
                                tone = PillTone.CYAN,
                                monospace = true,
                            )
                        }
                    }

                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Text(
                            text = "Opacity ${(draftProfile.opacity * 100f).roundToInt()}%",
                            color = p.textSecondary,
                            fontSize = 11.5.sp,
                            fontFamily = FontFamily.Monospace,
                        )
                        Slider(
                            value = draftProfile.opacity,
                            onValueChange = { value ->
                                draftProfile = draftProfile.copy(
                                    opacity = value.coerceIn(KeymapControl.MIN_OPACITY, KeymapControl.MAX_OPACITY),
                                )
                            },
                            valueRange = 0.15f..1.0f,
                            colors = SliderDefaults.colors(
                                thumbColor = p.brand,
                                activeTrackColor = p.brand,
                                inactiveTrackColor = p.borderStrong,
                            ),
                            modifier = Modifier.width(150.dp),
                        )
                        if (onRequestPicker != null) {
                            PrecisionButton(
                                text = "Presets",
                                style = PrecisionButtonStyle.SUBTLE,
                                compact = true,
                                onClick = onRequestPicker,
                            )
                        }
                        PrecisionButton(
                            text = "Reset",
                            style = PrecisionButtonStyle.SUBTLE,
                            compact = true,
                            onClick = {
                                draftProfile = profile
                                selectedControlId = null
                            },
                        )
                        if (onExitEditMode != null) {
                            PrecisionButton(
                                text = tr("Cancel"),
                                style = PrecisionButtonStyle.SUBTLE,
                                compact = true,
                                onClick = onExitEditMode,
                            )
                        }
                        PrecisionButton(
                            text = tr("Save"),
                            leadingSymbol = MobileSymbol.Check,
                            style = PrecisionButtonStyle.PRIMARY,
                            compact = true,
                            onClick = {
                                onProfileUpdated?.invoke(draftProfile.copy(builtIn = false))
                                onExitEditMode?.invoke()
                            },
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun EditGridOverlay(modifier: Modifier = Modifier) {
    val lineColor = WeaveloTokens.colors.brand.copy(alpha = 0.22f)
    Canvas(modifier = modifier) {
        val cols = 10
        val rows = 6
        for (i in 1 until cols) {
            val x = size.width * i / cols
            drawLine(lineColor, Offset(x, 0f), Offset(x, size.height), strokeWidth = 1f)
        }
        for (j in 1 until rows) {
            val y = size.height * j / rows
            drawLine(lineColor, Offset(0f, y), Offset(size.width, y), strokeWidth = 1f)
        }
    }
}

@Composable
private fun EditableControlSlot(
    control: KeymapControl,
    selected: Boolean,
    onSelect: () -> Unit,
    onDragDelta: (Offset) -> Unit,
) {
    val p = WeaveloTokens.colors
    val currentOnDrag by rememberUpdatedState(onDragDelta)
    val currentOnSelect by rememberUpdatedState(onSelect)
    val shape = controlShape(control.kind)
    val borderColor = if (selected) p.brand else Color.White.copy(alpha = 0.72f)
    val bgColor = if (selected) p.brand.copy(alpha = 0.34f) else Color(0xAA0F172A)

    Box(
        modifier = Modifier
            .fillMaxSize()
            .clip(shape)
            .background(bgColor)
            .border(BorderStroke(if (selected) 2.dp else 1.2.dp, borderColor), shape)
            .pointerInput(control.id) {
                detectDragGestures(
                    onDragStart = { currentOnSelect() },
                    onDrag = { change, dragAmount ->
                        change.consume()
                        currentOnDrag(dragAmount)
                    },
                )
            }
            .padding(4.dp),
        contentAlignment = Alignment.Center,
    ) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Text(
                text = control.label,
                color = Color.White,
                fontSize = 11.5.sp,
                fontWeight = FontWeight.Bold,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                textAlign = TextAlign.Center,
            )
            Text(
                text = control.kind.displayLabel,
                color = Color.White.copy(alpha = 0.75f),
                fontSize = 9.sp,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
private fun PlayableVirtualControl(
    control: KeymapControl,
    onKeyEvent: (String, Boolean) -> Unit,
    onMouseButton: (String, Boolean) -> Unit,
    onLookDelta: (Float, Float) -> Unit,
    onScrollDelta: (Float) -> Unit,
) {
    when (control.kind) {
        VirtualControlKind.JOYSTICK_WASD -> VirtualWasdJoystick(control = control, onKeyEvent = onKeyEvent)
        VirtualControlKind.LOOK_PAD -> VirtualLookPad(control = control, onLookDelta = onLookDelta)
        VirtualControlKind.SCROLL_WHEEL -> VirtualScrollWheel(control = control, onScrollDelta = onScrollDelta)
        VirtualControlKind.KEY_BUTTON,
        VirtualControlKind.COMBO_BUTTON,
        VirtualControlKind.MACRO_BURST,
        VirtualControlKind.MOUSE_BUTTON -> VirtualActionButton(
            control = control,
            onKeyEvent = onKeyEvent,
            onMouseButton = onMouseButton,
        )
    }
}

@Composable
private fun VirtualWasdJoystick(
    control: KeymapControl,
    onKeyEvent: (String, Boolean) -> Unit,
) {
    val p = WeaveloTokens.colors
    val currentOnKey by rememberUpdatedState(onKeyEvent)
    val dirs = remember(control.keys) { control.effectiveDirectionalKeys() }
    var knobOffset by remember { mutableStateOf(Offset.Zero) }
    var activeKeys by remember { mutableStateOf(emptySet<String>()) }

    fun syncDirectionalKeys(nextKeys: Set<String>) {
        val released = activeKeys - nextKeys
        val pressed = nextKeys - activeKeys
        released.forEach { currentOnKey(it, false) }
        pressed.forEach { currentOnKey(it, true) }
        activeKeys = nextKeys
    }

    DisposableEffect(control.id) {
        onDispose {
            activeKeys.forEach { currentOnKey(it, false) }
            activeKeys = emptySet()
        }
    }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .pointerInput(control.id, dirs) {
                val center = Offset(size.width / 2f, size.height / 2f)
                val maxRadius = (min(size.width, size.height) * 0.36f).coerceAtLeast(1f)
                val deadzone = 0.25f

                awaitEachGesture {
                    val down = awaitFirstDown(requireUnconsumed = false)
                    down.consume()

                    fun updateFromPosition(pos: Offset) {
                        val raw = pos - center
                        val dist = hypot(raw.x, raw.y)
                        val clamped = if (dist > maxRadius) {
                            Offset(raw.x / dist * maxRadius, raw.y / dist * maxRadius)
                        } else {
                            raw
                        }
                        knobOffset = clamped
                        val nx = clamped.x / maxRadius
                        val ny = clamped.y / maxRadius
                        val next = buildSet {
                            if (ny < -deadzone) add(dirs[0]) // Up (W)
                            if (nx < -deadzone) add(dirs[1]) // Left (A)
                            if (ny > deadzone) add(dirs[2])  // Down (S)
                            if (nx > deadzone) add(dirs[3])  // Right (D)
                        }
                        syncDirectionalKeys(next)
                    }

                    updateFromPosition(down.position)
                    while (true) {
                        val event = awaitPointerEvent()
                        val pointer = event.changes.firstOrNull { it.id == down.id }
                        if (pointer == null || !pointer.pressed) {
                            pointer?.consume()
                            break
                        }
                        pointer.consume()
                        updateFromPosition(pointer.position)
                    }
                    knobOffset = Offset.Zero
                    syncDirectionalKeys(emptySet())
                }
            },
        contentAlignment = Alignment.Center,
    ) {
        val ringColor = if (activeKeys.isNotEmpty()) p.brand else Color.White.copy(alpha = 0.65f)
        Canvas(modifier = Modifier.fillMaxSize()) {
            val radius = size.minDimension / 2f
            drawCircle(color = Color(0x7A0F172A), radius = radius)
            drawCircle(
                color = ringColor,
                radius = radius - 1.5.dp.toPx(),
                style = Stroke(width = 1.6.dp.toPx()),
            )
            val knobCenter = Offset(size.width / 2f + knobOffset.x, size.height / 2f + knobOffset.y)
            drawCircle(
                color = if (activeKeys.isNotEmpty()) p.brand.copy(alpha = 0.88f) else Color.White.copy(alpha = 0.80f),
                radius = radius * 0.34f,
                center = knobCenter,
            )
        }
        Text(
            text = control.label,
            color = Color.White.copy(alpha = 0.72f),
            fontSize = 10.sp,
            fontWeight = FontWeight.SemiBold,
        )
    }
}

@Composable
private fun VirtualLookPad(
    control: KeymapControl,
    onLookDelta: (Float, Float) -> Unit,
) {
    val currentOnLook by rememberUpdatedState(onLookDelta)
    var active by remember { mutableStateOf(false) }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .clip(RoundedCornerShape(18.dp))
            .background(if (active) Color(0x4438CFE0) else Color(0x280F172A))
            .border(
                BorderStroke(1.dp, if (active) Color(0xCC38CFE0) else Color.White.copy(alpha = 0.35f)),
                RoundedCornerShape(18.dp),
            )
            .pointerInput(control.id, control.sensitivity) {
                detectDragGestures(
                    onDragStart = { active = true },
                    onDragEnd = { active = false },
                    onDragCancel = { active = false },
                    onDrag = { change, dragAmount ->
                        change.consume()
                        currentOnLook(
                            dragAmount.x * control.sensitivity,
                            dragAmount.y * control.sensitivity,
                        )
                    },
                )
            },
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = control.label,
            color = Color.White.copy(alpha = 0.65f),
            fontSize = 10.5.sp,
            fontWeight = FontWeight.Medium,
        )
    }
}

@Composable
private fun VirtualScrollWheel(
    control: KeymapControl,
    onScrollDelta: (Float) -> Unit,
) {
    val currentOnScroll by rememberUpdatedState(onScrollDelta)
    var active by remember { mutableStateOf(false) }

    Box(
        modifier = Modifier
            .fillMaxSize()
            .clip(RoundedCornerShape(16.dp))
            .background(if (active) Color(0x8838CFE0) else Color(0x660F172A))
            .border(
                BorderStroke(1.2.dp, if (active) Color(0xEE38CFE0) else Color.White.copy(alpha = 0.55f)),
                RoundedCornerShape(16.dp),
            )
            .pointerInput(control.id, control.sensitivity) {
                detectDragGestures(
                    onDragStart = { active = true },
                    onDragEnd = { active = false },
                    onDragCancel = { active = false },
                    onDrag = { change, dragAmount ->
                        change.consume()
                        currentOnScroll(dragAmount.y * control.sensitivity)
                    },
                )
            }
            .padding(vertical = 6.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = control.label,
            color = Color.White,
            fontSize = 9.5.sp,
            fontWeight = FontWeight.SemiBold,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun VirtualActionButton(
    control: KeymapControl,
    onKeyEvent: (String, Boolean) -> Unit,
    onMouseButton: (String, Boolean) -> Unit,
) {
    val p = WeaveloTokens.colors
    val scope = rememberCoroutineScope()
    val currentOnKey by rememberUpdatedState(onKeyEvent)
    val currentOnMouse by rememberUpdatedState(onMouseButton)
    var pressed by remember { mutableStateOf(false) }
    var toggledOn by remember { mutableStateOf(false) }
    var burstJob by remember { mutableStateOf<Job?>(null) }

    fun releaseControl() {
        burstJob?.cancel()
        burstJob = null
        if (!pressed) return
        pressed = false
        when (control.kind) {
            VirtualControlKind.KEY_BUTTON -> {
                if (!control.toggle) {
                    control.keys.firstOrNull()?.let { currentOnKey(it, false) }
                }
            }
            VirtualControlKind.COMBO_BUTTON -> {
                control.keys.asReversed().forEach { key ->
                    dispatchSingleControlKey(key, false, currentOnKey, currentOnMouse)
                }
            }
            VirtualControlKind.MOUSE_BUTTON -> {
                control.keys.firstOrNull()?.let { currentOnMouse(it, false) }
            }
            VirtualControlKind.MACRO_BURST -> {
                control.keys.firstOrNull()?.let { key ->
                    dispatchSingleControlKey(key, false, currentOnKey, currentOnMouse)
                }
            }
            else -> Unit
        }
    }

    DisposableEffect(control.id) {
        onDispose {
            releaseControl()
            if (toggledOn) {
                control.keys.firstOrNull()?.let { currentOnKey(it, false) }
                toggledOn = false
            }
        }
    }

    val shape = controlShape(control.kind)
    val highlighted = pressed || toggledOn
    val bgColor = if (highlighted) p.brand.copy(alpha = 0.85f) else Color(0x880F172A)
    val borderColor = if (highlighted) Color.White else Color.White.copy(alpha = 0.62f)

    Box(
        modifier = Modifier
            .fillMaxSize()
            .clip(shape)
            .background(bgColor)
            .border(BorderStroke(1.4.dp, borderColor), shape)
            .pointerInput(control.id, control.kind, control.keys, control.toggle, control.burstIntervalMs) {
                awaitEachGesture {
                    val down = awaitFirstDown(requireUnconsumed = false)
                    down.consume()
                    pressed = true

                    when (control.kind) {
                        VirtualControlKind.KEY_BUTTON -> {
                            val key = control.keys.firstOrNull()
                            if (key != null) {
                                if (control.toggle) {
                                    val next = !toggledOn
                                    toggledOn = next
                                    currentOnKey(key, next)
                                } else {
                                    currentOnKey(key, true)
                                }
                            }
                        }
                        VirtualControlKind.COMBO_BUTTON -> {
                            control.keys.forEach { key ->
                                dispatchSingleControlKey(key, true, currentOnKey, currentOnMouse)
                            }
                        }
                        VirtualControlKind.MOUSE_BUTTON -> {
                            control.keys.firstOrNull()?.let { currentOnMouse(it, true) }
                        }
                        VirtualControlKind.MACRO_BURST -> {
                            val key = control.keys.firstOrNull()
                            if (key != null) {
                                burstJob?.cancel()
                                burstJob = scope.launch {
                                    val interval = control.burstIntervalMs.toLong()
                                        .coerceIn(
                                            KeymapControl.MIN_BURST_INTERVAL_MS.toLong(),
                                            KeymapControl.MAX_BURST_INTERVAL_MS.toLong(),
                                        )
                                    val holdMs = (interval / 2L).coerceAtLeast(10L)
                                    while (isActive) {
                                        dispatchSingleControlKey(key, true, currentOnKey, currentOnMouse)
                                        delay(holdMs)
                                        dispatchSingleControlKey(key, false, currentOnKey, currentOnMouse)
                                        delay((interval - holdMs).coerceAtLeast(10L))
                                    }
                                }
                            }
                        }
                        else -> Unit
                    }

                    while (true) {
                        val event = awaitPointerEvent()
                        val pointer = event.changes.firstOrNull { it.id == down.id }
                        if (pointer == null || !pointer.pressed) {
                            pointer?.consume()
                            break
                        }
                        pointer.consume()
                    }
                    releaseControl()
                }
            }
            .padding(horizontal = 4.dp, vertical = 2.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = control.label,
            color = Color.White,
            fontSize = 11.5.sp,
            fontWeight = FontWeight.Bold,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            textAlign = TextAlign.Center,
        )
    }
}

private fun dispatchSingleControlKey(
    key: String,
    pressed: Boolean,
    onKeyEvent: (String, Boolean) -> Unit,
    onMouseButton: (String, Boolean) -> Unit,
) {
    if (key.startsWith("mouse_", ignoreCase = true)) {
        onMouseButton(key, pressed)
    } else {
        onKeyEvent(key, pressed)
    }
}

private fun controlShape(kind: VirtualControlKind) = when (kind) {
    VirtualControlKind.JOYSTICK_WASD,
    VirtualControlKind.KEY_BUTTON,
    VirtualControlKind.MOUSE_BUTTON,
    VirtualControlKind.MACRO_BURST -> CircleShape
    VirtualControlKind.COMBO_BUTTON,
    VirtualControlKind.LOOK_PAD,
    VirtualControlKind.SCROLL_WHEEL -> RoundedCornerShape(14.dp)
}

/**
 * Searchable catalog dialog for choosing from 500+ built-in presets and user custom keymaps.
 */
@Composable
fun KeymapPickerDialog(
    onDismiss: () -> Unit,
    activeProfileId: String? = null,
    onSelectProfile: (KeymapProfile) -> Unit,
    onEditProfile: ((KeymapProfile) -> Unit)? = null,
    onDisableHud: (() -> Unit)? = null,
) {
    val p = WeaveloTokens.colors
    val context = LocalContext.current.applicationContext
    var customProfiles by remember { mutableStateOf(KeymapStore.listCustom(context)) }
    var searchQuery by remember { mutableStateOf("") }
    var selectedCategory by remember { mutableStateOf<KeymapCategory?>(null) }

    val filteredProfiles = remember(customProfiles, searchQuery, selectedCategory) {
        val customIds = customProfiles.mapTo(HashSet()) { it.id }
        val builtIn = when (selectedCategory) {
            null -> KeymapCatalog.search(searchQuery)
            KeymapCategory.CUSTOM -> emptyList()
            else -> KeymapCatalog.search(searchQuery, selectedCategory)
        }.filterNot { it.id in customIds }

        val normalizedQuery = searchQuery.trim().lowercase(Locale.ROOT)
        val filteredCustom = customProfiles.filter { profile ->
            val categoryMatch = selectedCategory == null ||
                selectedCategory == KeymapCategory.CUSTOM ||
                profile.category == selectedCategory
            val queryMatch = normalizedQuery.isEmpty() ||
                profile.name.lowercase(Locale.ROOT).contains(normalizedQuery) ||
                profile.tags.any { it.lowercase(Locale.ROOT).contains(normalizedQuery) } ||
                profile.processNames.any { it.lowercase(Locale.ROOT).contains(normalizedQuery) }
            categoryMatch && queryMatch
        }
        filteredCustom + builtIn
    }

    PrecisionDialog(
        title = "Virtual Keymap Presets",
        subtitle = "Browse ${KeymapCatalog.all().size}+ built-in game & app layouts or use your custom profiles.",
        icon = MobileSymbol.Keyboard,
        maxWidth = 560.dp,
        onDismissRequest = onDismiss,
        dismissButton = if (onDisableHud != null && activeProfileId != null) {
            {
                PrecisionButton(
                    text = "Hide HUD",
                    style = PrecisionButtonStyle.DANGER_TONAL,
                    compact = true,
                    onClick = {
                        onDisableHud()
                        onDismiss()
                    },
                )
            }
        } else {
            null
        },
        confirmButton = {
            PrecisionButton(
                text = tr("Close"),
                style = PrecisionButtonStyle.TONAL,
                compact = true,
                onClick = onDismiss,
            )
        },
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            PrecisionTextField(
                value = searchQuery,
                onValueChange = { searchQuery = it },
                label = "Search game, app, or process (.exe)",
            )

            LazyRow(
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                contentPadding = PaddingValues(vertical = 2.dp),
            ) {
                item(key = "cat-all") {
                    CategoryChip(
                        label = "All",
                        selected = selectedCategory == null,
                        onClick = { selectedCategory = null },
                    )
                }
                items(KeymapCategory.entries, key = { it.wireValue }) { category ->
                    CategoryChip(
                        label = category.displayLabel,
                        selected = selectedCategory == category,
                        onClick = { selectedCategory = category },
                    )
                }
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                PrecisionPillBadge(
                    text = "${filteredProfiles.size} presets",
                    tone = PillTone.CYAN,
                    monospace = true,
                )
                if (customProfiles.isNotEmpty()) {
                    PrecisionPillBadge(
                        text = "${customProfiles.size} custom",
                        tone = PillTone.BRAND,
                        monospace = true,
                    )
                }
                Spacer(Modifier.weight(1f))
            }

            if (filteredProfiles.isEmpty()) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(12.dp))
                        .background(p.surfaceSunken)
                        .border(BorderStroke(1.dp, p.borderMuted), RoundedCornerShape(12.dp))
                        .padding(16.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = "No keymap presets match \"$searchQuery\".",
                        color = p.textSecondary,
                        fontSize = 12.5.sp,
                    )
                }
            } else {
                LazyColumn(
                    modifier = Modifier.heightIn(max = 320.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    items(filteredProfiles, key = { it.id }) { profile ->
                        val isActive = profile.id == activeProfileId
                        KeymapPresetRow(
                            profile = profile,
                            isActive = isActive,
                            onSelect = {
                                onSelectProfile(profile)
                                onDismiss()
                            },
                            onEdit = onEditProfile?.let { editor ->
                                {
                                    editor(profile)
                                    onDismiss()
                                }
                            },
                            onDelete = if (!profile.builtIn) {
                                {
                                    customProfiles = KeymapStore.removeCustom(context, profile.id)
                                }
                            } else {
                                null
                            },
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun CategoryChip(
    label: String,
    selected: Boolean,
    onClick: () -> Unit,
) {
    val p = WeaveloTokens.colors
    Box(
        modifier = Modifier
            .clip(RoundedCornerShape(10.dp))
            .background(if (selected) p.brandSoft else p.surfaceSunken)
            .border(
                BorderStroke(1.dp, if (selected) p.brandBorder else p.borderMuted),
                RoundedCornerShape(10.dp),
            )
            .clickable(role = Role.Tab, onClick = onClick)
            .padding(horizontal = 10.dp, vertical = 6.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = label,
            color = if (selected) p.brand else p.textSecondary,
            fontSize = 11.5.sp,
            fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Medium,
            maxLines = 1,
        )
    }
}

@Composable
private fun KeymapPresetRow(
    profile: KeymapProfile,
    isActive: Boolean,
    onSelect: () -> Unit,
    onEdit: (() -> Unit)?,
    onDelete: (() -> Unit)?,
) {
    val p = WeaveloTokens.colors
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .background(if (isActive) p.brandSoft else p.surface)
            .border(
                BorderStroke(1.dp, if (isActive) p.brandBorder else p.border),
                RoundedCornerShape(12.dp),
            )
            .padding(horizontal = 12.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                PrecisionPillBadge(
                    text = profile.category.displayLabel,
                    tone = if (isActive) PillTone.EMERALD else PillTone.BRAND,
                    showDot = isActive,
                )
                if (!profile.builtIn) {
                    PrecisionPillBadge(
                        text = "Custom",
                        tone = PillTone.CYAN,
                    )
                }
                Text(
                    text = profile.name,
                    color = p.textPrimary,
                    fontSize = 13.sp,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Text(
                text = buildString {
                    append("${profile.controls.size} controls")
                    profile.processNames.firstOrNull()?.let { proc ->
                        append(" · ")
                        append(proc)
                    }
                },
                color = p.textTertiary,
                fontSize = 11.5.sp,
                fontFamily = FontFamily.Monospace,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        if (onEdit != null) {
            PrecisionButton(
                text = "Edit",
                style = PrecisionButtonStyle.SUBTLE,
                compact = true,
                onClick = onEdit,
            )
        }
        PrecisionButton(
            text = if (isActive) "Active" else "Use",
            style = if (isActive) PrecisionButtonStyle.TONAL else PrecisionButtonStyle.PRIMARY,
            compact = true,
            enabled = !isActive,
            onClick = onSelect,
        )
        if (onDelete != null) {
            MobileAction(
                symbol = MobileSymbol.Close,
                label = tr("Delete"),
                destructive = true,
                modifier = Modifier.size(34.dp),
                onClick = onDelete,
            )
        }
    }
}
