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
    sendTouchBatch: ((List<RemoteTouchChange>) -> Unit)? = null,
    sendTouchFrame: (List<RemoteTouchChange>) -> Unit = {}
): Modifier = composed {
    val currentSink by rememberUpdatedState(sendTouch)
    val currentBatchSink by rememberUpdatedState(sendTouchBatch)
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
            val batchSink = currentBatchSink
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
            var initialP0 = Offset.Zero
            var initialP1 = Offset.Zero
            var firstPairId = 0L
            var secondPairId = 0L
            var twoFingerStartUptime = 0L
            var maxTwoFingerCentroidDrift = 0f
            var maxTwoFingerSpanDrift = 0f
            var twoFingerDirectAnchor: Pair<Float, Float>? = null
            var undecidedTwoFingerTapHandled = false
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

            fun mapTwoFingerScrollNormalized(
                position: Offset,
                baseW: Float,
                baseH: Float,
                containerW: Float,
                containerH: Float
            ): Pair<Float, Float> =
                if (touchMode == TouchMode.DIRECT) {
                    val anchor = twoFingerDirectAnchor ?: transform.screenToNormalized(
                        initialCentroid.x,
                        initialCentroid.y,
                        baseW,
                        baseH,
                        containerW,
                        containerH
                    ).also { twoFingerDirectAnchor = it }
                    (anchor.first + (position.x - initialCentroid.x) / containerW) to
                        (anchor.second + (position.y - initialCentroid.y) / containerH)
                } else {
                    (position.x / containerW).coerceIn(0f, 1f) to
                        (position.y / containerH).coerceIn(0f, 1f)
                }

            fun dispatchRemoteBatch(batch: List<RemoteTouchChange>) {
                if (batch.isEmpty()) return
                if (batchSink != null) {
                    batchSink(batch)
                } else {
                    for (item in batch) {
                        gestureSink(item.action, item.pointerId, item.x, item.y, item.pressure, item.time)
                    }
                }
            }

            fun forwardRemoteChanges(
                changes: List<androidx.compose.ui.input.pointer.PointerInputChange>,
                baseW: Float,
                baseH: Float,
                containerW: Float,
                containerH: Float,
                forceMove: Boolean = false,
                guardSingleFingerSlop: Boolean = false,
                useTwoFingerScrollSpace: Boolean = false,
                prefixBatch: List<RemoteTouchChange> = emptyList()
            ) {
                // Paused/disconnected/modal viewing may still zoom locally, but
                // must never issue commands against an old remote frame.
                if (!enabled) {
                    for (change in changes) change.consume()
                    return
                }
                val mapPoint: (Offset) -> Pair<Float, Float> = { pos ->
                    if (useTwoFingerScrollSpace) {
                        mapTwoFingerScrollNormalized(pos, baseW, baseH, containerW, containerH)
                    } else {
                        mapNormalized(pos, baseW, baseH, containerW, containerH)
                    }
                }
                val batch = ArrayList<RemoteTouchChange>(prefixBatch.size + changes.size)
                batch.addAll(prefixBatch)
                for (change in changes) {
                    val pointerId = change.id.value.toInt()
                    val pressure = if (change.pressed) 1f else 0f
                    when {
                        !change.previousPressed && change.pressed -> {
                            if (touchMode == TouchMode.DIRECT &&
                                !useTwoFingerScrollSpace &&
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
                            val (normX, normY) = mapPoint(change.position)
                            activePointers.add(pointerId)
                            batch.add(RemoteTouchChange(0, pointerId, normX, normY, pressure, change.uptimeMillis))
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
                            val (normX, normY) = mapPoint(effectivePos)
                            activePointers.remove(pointerId)
                            if (pointerId == singleFingerId) singleFingerId = null
                            batch.add(RemoteTouchChange(2, pointerId, normX, normY, pressure, change.uptimeMillis))
                        }
                        change.pressed && (forceMove || change.positionChanged()) && activePointers.contains(pointerId) -> {
                            if (guardSingleFingerSlop && pointerId == singleFingerId && !singleFingerMovedPastSlop) {
                                if ((change.position - singleFingerOrigin).getDistance() < singleFingerSlopPx) {
                                    change.consume()
                                    continue
                                }
                                singleFingerMovedPastSlop = true
                            }
                            val (normX, normY) = mapPoint(change.position)
                            activePointers.add(pointerId)
                            batch.add(RemoteTouchChange(1, pointerId, normX, normY, pressure, change.uptimeMillis))
                        }
                    }
                    change.consume()
                }
                dispatchRemoteBatch(batch)
            }

            fun cancelRemotePointers() {
                if (touchMode == TouchMode.NATIVE && activePointers.isNotEmpty()) {
                    val now=android.os.SystemClock.uptimeMillis()
                    frameSink(activePointers.map { RemoteTouchChange(3,it,0f,0f,0f,now) })
                } else if (activePointers.isNotEmpty()) {
                    val now = android.os.SystemClock.uptimeMillis()
                    dispatchRemoteBatch(activePointers.map { RemoteTouchChange(3, it, 0f, 0f, 0f, now) })
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
                        val preferRemoteScroll =
                            touchMode == TouchMode.TRACKPAD || touchMode == TouchMode.DIRECT
                        val currentPair = pressedChanges.map { it.id.value }.toSet()
                        if (!twoFingerStarted || pairIds != currentPair) {
                            pairIds = currentPair
                            twoFingerStarted = true
                            remoteTwoFingerSynced = false
                            undecidedTwoFingerTapHandled = false
                            twoFingerDirectAnchor = null
                            firstPairId = pressedChanges[0].id.value
                            secondPairId = pressedChanges[1].id.value
                            initialP0 = pressedChanges[0].position
                            initialP1 = pressedChanges[1].position
                            val currentDistance = (initialP0 - initialP1).getDistance()
                            val currentCentroid = (initialP0 + initialP1) / 2f
                            initialDistance = currentDistance
                            initialCentroid = currentCentroid
                            prevDistance = currentDistance
                            prevCentroid = currentCentroid
                            twoFingerStartUptime = event.changes.minOf { it.uptimeMillis }
                            maxTwoFingerCentroidDrift = 0f
                            maxTwoFingerSpanDrift = 0f
                            twoFingerMode = classifyTwoFingerGesture(
                                transform.scale,
                                initialDistance,
                                currentDistance,
                                initialCentroid,
                                currentCentroid,
                                preferRemoteScroll = preferRemoteScroll,
                                firstDelta = Offset.Zero,
                                secondDelta = Offset.Zero
                            )
                        }
                        val p0 = pressedChanges.firstOrNull { it.id.value == firstPairId }?.position
                            ?: pressedChanges[0].position
                        val p1 = pressedChanges.firstOrNull { it.id.value == secondPairId }?.position
                            ?: pressedChanges[1].position
                        val currentDistance = (p0 - p1).getDistance()
                        val currentCentroid = (p0 + p1) / 2f
                        maxTwoFingerCentroidDrift = maxOf(
                            maxTwoFingerCentroidDrift,
                            (currentCentroid - initialCentroid).getDistance()
                        )
                        maxTwoFingerSpanDrift = maxOf(
                            maxTwoFingerSpanDrift,
                            kotlin.math.abs(currentDistance - initialDistance)
                        )
                        if (twoFingerMode == TwoFingerGestureMode.UNDECIDED) {
                            twoFingerMode = classifyTwoFingerGesture(
                                transform.scale,
                                initialDistance,
                                currentDistance,
                                initialCentroid,
                                currentCentroid,
                                preferRemoteScroll = preferRemoteScroll,
                                firstDelta = p0 - initialP0,
                                secondDelta = p1 - initialP1
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
                        } else if (twoFingerMode == TwoFingerGestureMode.UNDECIDED) {
                            // Hold two-finger Down edges until gesture arbitration resolves so
                            // hesitant touches or pinches never leak right-clicks to the host.
                            if (activePointers.isNotEmpty()) {
                                cancelRemotePointers()
                            }
                            for (change in event.changes) change.consume()
                        } else {
                            outsideVideoSingleTouch = false
                            val missingDowns = ArrayList<RemoteTouchChange>(2)
                            val uptime = pressedChanges.minOf { it.uptimeMillis }
                            val initialPairs = listOf(
                                firstPairId.toInt() to initialP0,
                                secondPairId.toInt() to initialP1
                            )
                            for ((pid, originPos) in initialPairs) {
                                if (enabled && pid !in activePointers) {
                                    val (nx, ny) = mapTwoFingerScrollNormalized(
                                        originPos,
                                        baseWidth,
                                        baseHeight,
                                        containerWidth,
                                        containerHeight
                                    )
                                    activePointers.add(pid)
                                    missingDowns.add(RemoteTouchChange(0, pid, nx, ny, 1f, uptime))
                                }
                            }
                            val syncAll = !remoteTwoFingerSynced
                            remoteTwoFingerSynced = true
                            forwardRemoteChanges(
                                event.changes,
                                baseWidth,
                                baseHeight,
                                containerWidth,
                                containerHeight,
                                forceMove = syncAll,
                                useTwoFingerScrollSpace = true,
                                prefixBatch = missingDowns
                            )
                            for (change in event.changes) change.consume()
                        }
                        prevDistance = currentDistance
                        prevCentroid = currentCentroid
                    } else if (localTransformGesture || (outsideVideoSingleTouch && maxPressedCount <= 1)) {
                        // Swallow staggered pinch finger-up tail or black-bar taps.
                        for (change in event.changes) change.consume()
                    } else if (twoFingerStarted && twoFingerMode == TwoFingerGestureMode.UNDECIDED) {
                        if (!undecidedTwoFingerTapHandled) {
                            undecidedTwoFingerTapHandled = true
                            val liftUptime = event.changes.maxOf { it.uptimeMillis }
                            val cleanTwoFingerTap = enabled &&
                                (liftUptime - twoFingerStartUptime) in 0L..260L &&
                                maxTwoFingerCentroidDrift <= singleFingerSlopPx &&
                                maxTwoFingerSpanDrift <= singleFingerSlopPx * 1.5f &&
                                (touchMode != TouchMode.DIRECT || isInsideVisibleVideo(
                                    initialCentroid,
                                    baseWidth,
                                    baseHeight,
                                    containerWidth,
                                    containerHeight
                                ))
                            if (cleanTwoFingerTap) {
                                val (nx0, ny0) = mapNormalized(initialP0, baseWidth, baseHeight, containerWidth, containerHeight)
                                val (nx1, ny1) = mapNormalized(initialP1, baseWidth, baseHeight, containerWidth, containerHeight)
                                val id0 = firstPairId.toInt()
                                val id1 = secondPairId.toInt()
                                dispatchRemoteBatch(
                                    listOf(
                                        RemoteTouchChange(0, id0, nx0, ny0, 1f, twoFingerStartUptime),
                                        RemoteTouchChange(0, id1, nx1, ny1, 1f, twoFingerStartUptime),
                                        RemoteTouchChange(2, id0, nx0, ny0, 0f, liftUptime),
                                        RemoteTouchChange(2, id1, nx1, ny1, 0f, liftUptime)
                                    )
                                )
                            }
                        }
                        for (change in event.changes) change.consume()
                    } else {
                        forwardRemoteChanges(
                            event.changes,
                            baseWidth,
                            baseHeight,
                            containerWidth,
                            containerHeight,
                            useTwoFingerScrollSpace = twoFingerMode == TwoFingerGestureMode.REMOTE_GESTURE,
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
