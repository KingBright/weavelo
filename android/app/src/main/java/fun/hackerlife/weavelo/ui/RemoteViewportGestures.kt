package `fun`.hackerlife.weavelo.ui

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChanged
import androidx.compose.ui.unit.IntSize
import `fun`.hackerlife.weavelo.TouchMode

internal data class RemoteTouchChange(val action: Int, val pointerId: Int,
    val x: Float, val y: Float, val pressure: Float, val time: Long)

/** The production gesture handler, with an injectable transport sink for on-device tests. */
internal fun Modifier.remoteViewportGestures(
    transformState: MutableState<ViewportTransform>,
    touchMode: TouchMode,
    viewportSize: IntSize,
    enabled: Boolean,
    contentKey: Any? = null,
    baseViewportSize: IntSize = viewportSize,
    restrictSingleTouchToVideo: Boolean = false,
    sendTouch: (Int, Int, Float, Float, Float, Long) -> Unit,
    sendTouchFrame: (List<RemoteTouchChange>) -> Unit = {}
): Modifier = composed {
    val currentSink by rememberUpdatedState(sendTouch)
    val currentFrameSink by rememberUpdatedState(sendTouchFrame)
    pointerInput(touchMode, viewportSize, baseViewportSize, restrictSingleTouchToVideo, enabled, contentKey) {
        var transform by transformState
        val singleFingerSlopPx = minOf(viewConfiguration.touchSlop * 0.8f, 14f)
        // Two-finger parallel drag/tap stays remote scroll/right-click across zoom levels.
        // Pinch-to-zoom and three-finger canvas pan take local ownership cleanly.
        awaitEachGesture {
            // Cleanup belongs to the callback that started this gesture. A
            // replacement surface/session must never receive the old cancel.
            val gestureSink = currentSink
            val frameSink = currentFrameSink
            val activePointers = mutableSetOf<Int>()
            var maxPressedCount = 0
            var twoFingerMode = TwoFingerGestureMode.UNDECIDED
            var twoFingerStarted = false
            var remoteTwoFingerSynced = false
            var threeFingerStarted = false
            var localTransformGesture = false
            var outsideVideoSingleTouch = false
            var initialDistance = 0f
            var initialCentroid = Offset.Zero
            var prevDistance = 0f
            var prevCentroid = Offset.Zero
            var prevThreeCentroid = Offset.Zero
            var pairIds = emptySet<Long>()
            var tripleIds = emptySet<Long>()
            var suppressGesture = false
            var singleFingerId: Int? = null
            var singleFingerOrigin = Offset.Zero
            var singleFingerMovedPastSlop = false

            fun isInsideVisibleVideo(
                pos: Offset,
                baseW: Float,
                baseH: Float,
                containerW: Float,
                containerH: Float
            ): Boolean {
                if (!restrictSingleTouchToVideo) return true
                val (visW, visH) = transform.expandedViewportSize(baseW, baseH, containerW, containerH)
                val left = (containerW - visW) / 2f
                val top = (containerH - visH) / 2f
                return pos.x in left..(left + visW) && pos.y in top..(top + visH)
            }

            fun mapNormalized(
                position: Offset,
                baseW: Float,
                baseH: Float,
                containerW: Float,
                containerH: Float
            ): Pair<Float, Float> =
                if (touchMode == TouchMode.DIRECT) {
                    transform.screenToNormalized(
                        position.x,
                        position.y,
                        baseW,
                        baseH,
                        containerW,
                        containerH
                    )
                } else {
                    // Trackpad deltas must not shrink as the viewport zooms, and
                    // allow thumb gliding across the full container (including black bars).
                    (position.x / containerW).coerceIn(0f, 1f) to
                        (position.y / containerH).coerceIn(0f, 1f)
                }

            fun forwardRemoteChanges(
                changes: List<androidx.compose.ui.input.pointer.PointerInputChange>,
                baseW: Float,
                baseH: Float,
                containerW: Float,
                containerH: Float,
                forceMove: Boolean = false,
                guardSingleFingerSlop: Boolean = false
            ) {
                // Paused/disconnected/modal viewing may still zoom locally, but
                // must never issue commands against an old remote frame.
                if (!enabled) {
                    for (change in changes) change.consume()
                    return
                }
                for (change in changes) {
                    val pointerId = change.id.value.toInt()
                    val pressure = if (change.pressed) 1f else 0f
                    when {
                        !change.previousPressed && change.pressed -> {
                            if (touchMode == TouchMode.DIRECT &&
                                activePointers.isEmpty() &&
                                !isInsideVisibleVideo(change.position, baseW, baseH, containerW, containerH)
                            ) {
                                outsideVideoSingleTouch = true
                                change.consume()
                                continue
                            }
                            if (guardSingleFingerSlop && activePointers.isEmpty()) {
                                singleFingerId = pointerId
                                singleFingerOrigin = change.position
                                singleFingerMovedPastSlop = false
                            }
                            val (normX, normY) = mapNormalized(change.position, baseW, baseH, containerW, containerH)
                            activePointers.add(pointerId)
                            gestureSink(
                                0, pointerId, normX, normY, pressure, change.uptimeMillis
                            )
                        }
                        change.previousPressed && !change.pressed && activePointers.contains(pointerId) -> {
                            val effectivePos = if (
                                guardSingleFingerSlop &&
                                pointerId == singleFingerId &&
                                !singleFingerMovedPastSlop
                            ) {
                                singleFingerOrigin
                            } else {
                                change.position
                            }
                            val (normX, normY) = mapNormalized(effectivePos, baseW, baseH, containerW, containerH)
                            activePointers.remove(pointerId)
                            if (pointerId == singleFingerId) singleFingerId = null
                            gestureSink(
                                2, pointerId, normX, normY, pressure, change.uptimeMillis
                            )
                        }
                        change.pressed && (forceMove || change.positionChanged()) && activePointers.contains(pointerId) -> {
                            if (guardSingleFingerSlop && pointerId == singleFingerId && !singleFingerMovedPastSlop) {
                                if ((change.position - singleFingerOrigin).getDistance() < singleFingerSlopPx) {
                                    change.consume()
                                    continue
                                }
                                singleFingerMovedPastSlop = true
                            }
                            val (normX, normY) = mapNormalized(change.position, baseW, baseH, containerW, containerH)
                            activePointers.add(pointerId)
                            gestureSink(
                                1, pointerId, normX, normY, pressure, change.uptimeMillis
                            )
                        }
                    }
                    change.consume()
                }
            }

            fun cancelRemotePointers() {
                if (touchMode == TouchMode.NATIVE && activePointers.isNotEmpty()) {
                    val now=android.os.SystemClock.uptimeMillis()
                    frameSink(activePointers.map { RemoteTouchChange(3,it,0f,0f,0f,now) })
                } else for (id in activePointers) {
                    gestureSink(3, id, 0f, 0f, 0f, android.os.SystemClock.uptimeMillis())
                }
                activePointers.clear()
                singleFingerId = null
            }

            try {
                while (true) {
                    val event = awaitPointerEvent()
                    // Child controls (including RESET) own their input. Never
                    // forward a consumed local tap to the remote computer.
                    if (event.changes.any { it.isConsumed }) {
                        cancelRemotePointers()
                        break
                    }
                    val containerWidth = size.width.coerceAtLeast(1).toFloat()
                    val containerHeight = size.height.coerceAtLeast(1).toFloat()
                    val baseWidth = if (baseViewportSize.width > 0) baseViewportSize.width.toFloat() else containerWidth
                    val baseHeight = if (baseViewportSize.height > 0) baseViewportSize.height.toFloat() else containerHeight
                    val pressedChanges = event.changes.filter { it.pressed }
                    maxPressedCount = maxOf(maxPressedCount, pressedChanges.size)

                    if (touchMode == TouchMode.NATIVE) {
                        if (pressedChanges.size > 10) { suppressGesture=true;cancelRemotePointers() }
                        val sample=ArrayList<RemoteTouchChange>()
                        if (enabled && !suppressGesture) for(change in event.changes) {
                            val id=change.id.value.toInt()
                            if (!change.previousPressed && change.pressed &&
                                !isInsideVisibleVideo(change.position, baseWidth, baseHeight, containerWidth, containerHeight)
                            ) {
                                continue
                            }
                            val point=transform.screenToNormalized(
                                change.position.x,
                                change.position.y,
                                baseWidth,
                                baseHeight,
                                containerWidth,
                                containerHeight
                            )
                            val action=when {
                                !change.previousPressed && change.pressed -> {activePointers.add(id);0}
                                change.previousPressed && !change.pressed && activePointers.remove(id) -> 2
                                change.pressed && change.positionChanged() && id in activePointers -> 1
                                else -> null
                            }
                            if(action!=null) sample.add(RemoteTouchChange(action,id,point.first,point.second,
                                if(change.pressed) change.pressure.coerceIn(0f,1f) else 0f,change.uptimeMillis))
                        }
                        if(sample.isNotEmpty()) frameSink(sample)
                        event.changes.forEach { it.consume() }
                        if(event.changes.none { it.pressed }) break
                        continue
                    }
                    if (touchMode == TouchMode.VIEW && pressedChanges.size == 1 && !localTransformGesture && maxPressedCount == 1) {
                        val change=pressedChanges.single()
                        if(change.previousPressed) transform=transform.update(
                            1f,
                            change.position - change.previousPosition,
                            baseWidth,
                            baseHeight,
                            change.previousPosition,
                            containerWidth,
                            containerHeight
                        )
                        event.changes.forEach { it.consume() }
                        continue
                    }

                    if (pressedChanges.size > 3) {
                        suppressGesture = true
                        cancelRemotePointers()
                    }
                    if (suppressGesture) {
                        for (change in event.changes) change.consume()
                    } else if (pressedChanges.size == 3) {
                        // Three-finger drag pans the zoomed canvas locally without
                        // switching modes or leaking clicks/scrolls to the host.
                        val c3 = (pressedChanges[0].position + pressedChanges[1].position + pressedChanges[2].position) / 3f
                        val currentTriple = pressedChanges.map { it.id.value }.toSet()
                        if (!threeFingerStarted || tripleIds != currentTriple) {
                            tripleIds = currentTriple
                            threeFingerStarted = true
                            prevThreeCentroid = c3
                            localTransformGesture = true
                            cancelRemotePointers()
                        } else {
                            val pan = c3 - prevThreeCentroid
                            prevThreeCentroid = c3
                            transform = transform.update(
                                1f,
                                pan,
                                baseWidth,
                                baseHeight,
                                c3,
                                containerWidth,
                                containerHeight
                            )
                        }
                        for (change in event.changes) change.consume()
                    } else if (maxPressedCount >= 3) {
                        // Swallow staggered finger-up tail after a three-finger gesture.
                        for (change in event.changes) change.consume()
                    } else if (pressedChanges.size == 2) {
                        val p0 = pressedChanges[0].position
                        val p1 = pressedChanges[1].position
                        val currentDistance = (p0 - p1).getDistance()
                        val currentCentroid = (p0 + p1) / 2f

                        val currentPair = pressedChanges.map { it.id.value }.toSet()
                        if (!twoFingerStarted || pairIds != currentPair) {
                            pairIds = currentPair
                            twoFingerStarted = true
                            remoteTwoFingerSynced = false
                            initialDistance = currentDistance
                            initialCentroid = currentCentroid
                            prevDistance = currentDistance
                            prevCentroid = currentCentroid
                            twoFingerMode = classifyTwoFingerGesture(
                                transform.scale,
                                initialDistance,
                                currentDistance,
                                initialCentroid,
                                currentCentroid
                            )
                        } else if (twoFingerMode == TwoFingerGestureMode.UNDECIDED) {
                            twoFingerMode = classifyTwoFingerGesture(
                                transform.scale,
                                initialDistance,
                                currentDistance,
                                initialCentroid,
                                currentCentroid
                            )
                        }

                        if (touchMode == TouchMode.VIEW || twoFingerMode == TwoFingerGestureMode.LOCAL_TRANSFORM) {
                            val baselineDistance = if (localTransformGesture) prevDistance else initialDistance
                            val baselineCentroid = if (localTransformGesture) prevCentroid else initialCentroid
                            if (!localTransformGesture) {
                                localTransformGesture = true
                                cancelRemotePointers()
                            }
                            val zoom = if (baselineDistance > 1f) currentDistance / baselineDistance else 1f
                            val pan = currentCentroid - baselineCentroid
                            transform = transform.update(
                                zoom,
                                pan,
                                baseWidth,
                                baseHeight,
                                baselineCentroid,
                                containerWidth,
                                containerHeight
                            )
                            for (change in event.changes) change.consume()
                        } else if (outsideVideoSingleTouch && touchMode == TouchMode.DIRECT) {
                            // Two-finger parallel scroll started outside the visible video in Direct mode:
                            // ignore remote scroll unless fingers are inside the visible video.
                            if (isInsideVisibleVideo(currentCentroid, baseWidth, baseHeight, containerWidth, containerHeight)) {
                                outsideVideoSingleTouch = false
                            } else {
                                for (change in event.changes) change.consume()
                            }
                        } else {
                            // Buffer movement while classifying. Forward only
                            // pointer edges so a pinch cannot scroll remotely
                            // before it crosses the local pinch threshold.
                            if (twoFingerMode == TwoFingerGestureMode.UNDECIDED) {
                                val edgeChanges = event.changes.filter { it.pressed != it.previousPressed }
                                forwardRemoteChanges(edgeChanges, baseWidth, baseHeight, containerWidth, containerHeight)
                            } else {
                                // Ensure both pointers are registered if first finger started near border
                                for (pressed in pressedChanges) {
                                    val pid = pressed.id.value.toInt()
                                    if (enabled && pid !in activePointers) {
                                        val (nx, ny) = mapNormalized(pressed.position, baseWidth, baseHeight, containerWidth, containerHeight)
                                        activePointers.add(pid)
                                        gestureSink(0, pid, nx, ny, 1f, pressed.uptimeMillis)
                                    }
                                }
                                val syncAll = !remoteTwoFingerSynced
                                remoteTwoFingerSynced = true
                                forwardRemoteChanges(event.changes, baseWidth, baseHeight, containerWidth, containerHeight, forceMove = syncAll)
                            }
                            for (change in event.changes) change.consume()
                        }
                        prevDistance = currentDistance
                        prevCentroid = currentCentroid
                    } else if (localTransformGesture || (outsideVideoSingleTouch && maxPressedCount <= 1)) {
                        // Swallow staggered pinch finger-up tail or black-bar taps.
                        for (change in event.changes) change.consume()
                    } else {
                        forwardRemoteChanges(
                            event.changes,
                            baseWidth,
                            baseHeight,
                            containerWidth,
                            containerHeight,
                            guardSingleFingerSlop = touchMode == TouchMode.DIRECT && !twoFingerStarted
                        )
                    }

                    if (event.changes.none { it.pressed }) break
                }
            } finally {
                cancelRemotePointers()
            }
        }
    }
}
