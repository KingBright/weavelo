package `fun`.hackerlife.weavelo.ui

import androidx.compose.ui.geometry.Offset
import kotlin.math.abs

enum class TwoFingerGestureMode {
    UNDECIDED,
    REMOTE_GESTURE,
    LOCAL_TRANSFORM
}

/**
 * Arbitrate two-finger gestures across all zoom levels so pinch-to-zoom and
 * in-app two-finger scrolling never fight each other:
 * - Diverging/converging finger span -> LOCAL_TRANSFORM (pinch zoom + simultaneous pan).
 * - Parallel two-finger translation -> REMOTE_GESTURE (remote scroll, even while zoomed).
 */
fun classifyTwoFingerGesture(
    currentScale: Float,
    initialDistance: Float,
    currentDistance: Float,
    initialCentroid: Offset,
    currentCentroid: Offset,
    pinchRatioThreshold: Float = 0.085f,
    minPinchPx: Float = 16f,
    panThresholdPx: Float = 12f
): TwoFingerGestureMode {
    @Suppress("UNUSED_PARAMETER")
    val scale = currentScale
    val baseline = initialDistance.coerceAtLeast(1f)
    val distanceDelta = abs(currentDistance - initialDistance)
    val distanceRatio = distanceDelta / baseline
    val centroidDelta = (currentCentroid - initialCentroid).getDistance()

    if (distanceDelta >= minPinchPx &&
        distanceRatio >= pinchRatioThreshold &&
        distanceDelta >= centroidDelta * 0.65f
    ) {
        return TwoFingerGestureMode.LOCAL_TRANSFORM
    }
    if (centroidDelta >= panThresholdPx &&
        (distanceDelta < minPinchPx || centroidDelta > distanceDelta * 1.4f)
    ) {
        return TwoFingerGestureMode.REMOTE_GESTURE
    }
    return TwoFingerGestureMode.UNDECIDED
}

/**
 * 视口缩放与平移状态机及坐标逆映射计算。
 * 支持无级缩放、竖屏/超宽屏放大时动态扩展可视高度与宽度、边界限制及零漂移坐标逆映射。
 */
data class ViewportTransform(
    val scale: Float = 1.0f,
    val offset: Offset = Offset.Zero,
    val minScale: Float = 1.0f,
    val maxScale: Float = 4.0f
) {
    /**
     * 计算当前缩放倍率下的实际可视窗口尺寸：
     * 在竖屏查看横屏桌面时，随着 scale 增大，纵向可视高度从 1.0x 基准高度平滑扩展至整个屏幕容器高度。
     */
    fun expandedViewportSize(
        viewWidth: Float,
        viewHeight: Float,
        containerWidth: Float = viewWidth,
        containerHeight: Float = viewHeight,
        currentScale: Float = scale
    ): Pair<Float, Float> {
        val bw = viewWidth.coerceAtLeast(1f)
        val bh = viewHeight.coerceAtLeast(1f)
        val cw = containerWidth.coerceAtLeast(bw)
        val ch = containerHeight.coerceAtLeast(bh)
        val s = currentScale.coerceAtLeast(0.01f)
        return minOf(cw, bw * s) to minOf(ch, bh * s)
    }

    /**
     * 计算平移边界限制：确保放大状态下可视窗口始终覆盖在内容有效区域内，无黑边越界。
     */
    fun clampOffset(
        proposedOffset: Offset,
        viewWidth: Float,
        viewHeight: Float,
        currentScale: Float = scale,
        containerWidth: Float = viewWidth,
        containerHeight: Float = viewHeight
    ): Offset {
        if (currentScale <= 1.0f) {
            return Offset.Zero
        }
        val (visW, visH) = expandedViewportSize(
            viewWidth,
            viewHeight,
            containerWidth,
            containerHeight,
            currentScale
        )
        val maxOffsetX = ((viewWidth * currentScale - visW) / 2f).coerceAtLeast(0f)
        val maxOffsetY = ((viewHeight * currentScale - visH) / 2f).coerceAtLeast(0f)
        val clampedX = proposedOffset.x.coerceIn(-maxOffsetX, maxOffsetX)
        val clampedY = proposedOffset.y.coerceIn(-maxOffsetY, maxOffsetY)
        return Offset(clampedX, clampedY)
    }

    /**
     * 缩放和平移更新：支持以捏合焦点 (focalPoint) 为中心进行无漂移缩放与平移
     */
    fun update(
        zoomFactor: Float,
        panDelta: Offset,
        viewWidth: Float,
        viewHeight: Float,
        focalPoint: Offset = Offset(viewWidth / 2f, viewHeight / 2f),
        containerWidth: Float = viewWidth,
        containerHeight: Float = viewHeight
    ): ViewportTransform {
        val newScale = (scale * zoomFactor).coerceIn(minScale, maxScale)
        val cx = containerWidth.coerceAtLeast(1f) / 2f
        val cy = containerHeight.coerceAtLeast(1f) / 2f
        val scaleRatio = if (scale > 0.001f) newScale / scale else 1.0f

        // 以 focalPoint 为缩放中心：
        // 新偏移量 = (focalPoint + panDelta) - center - (focalPoint - center - offset) * scaleRatio
        val proposedX = (focalPoint.x + panDelta.x) - cx - (focalPoint.x - cx - offset.x) * scaleRatio
        val proposedY = (focalPoint.y + panDelta.y) - cy - (focalPoint.y - cy - offset.y) * scaleRatio

        val newOffset = clampOffset(
            Offset(proposedX, proposedY),
            viewWidth,
            viewHeight,
            newScale,
            containerWidth,
            containerHeight
        )
        return copy(scale = newScale, offset = newOffset)
    }

    /**
     * 快速复位至 1.0x
     */
    fun reset(): ViewportTransform = copy(scale = 1.0f, offset = Offset.Zero)

    /**
     * 一键纵向铺满（竖屏查看桌面时将纵向可视区域扩展至容器高度）。
     */
    fun fillHeight(
        viewWidth: Float,
        viewHeight: Float,
        containerWidth: Float,
        containerHeight: Float
    ): ViewportTransform {
        val bh = viewHeight.coerceAtLeast(1f)
        val targetScale = (containerHeight.coerceAtLeast(bh) / bh).coerceIn(minScale, maxScale)
        return copy(
            scale = targetScale,
            offset = clampOffset(offset, viewWidth, viewHeight, targetScale, containerWidth, containerHeight)
        )
    }

    /** Full-source edge coordinates of the real visible video viewport (including expanded container area). */
    fun visibleRect(
        viewWidth: Float,
        viewHeight: Float,
        containerWidth: Float = viewWidth,
        containerHeight: Float = viewHeight
    ): IntArray {
        val cw = containerWidth.coerceAtLeast(1f)
        val ch = containerHeight.coerceAtLeast(1f)
        val (visW, visH) = expandedViewportSize(viewWidth, viewHeight, cw, ch)
        val leftX = (cw - visW) / 2f
        val topY = (ch - visH) / 2f
        val (left, top) = screenToNormalized(leftX, topY, viewWidth, viewHeight, cw, ch)
        val (right, bottom) = screenToNormalized(leftX + visW, topY + visH, viewWidth, viewHeight, cw, ch)
        val x = kotlin.math.floor(left.toDouble() * 1_000_000).toInt().coerceIn(0, 999_999)
        val y = kotlin.math.floor(top.toDouble() * 1_000_000).toInt().coerceIn(0, 999_999)
        val endX = kotlin.math.ceil(right.toDouble() * 1_000_000).toInt().coerceIn(x + 1, 1_000_000)
        val endY = kotlin.math.ceil(bottom.toDouble() * 1_000_000).toInt().coerceIn(y + 1, 1_000_000)
        return intArrayOf(x, y, endX - x, endY - y)
    }

    /** Trackpad navigation follows its cursor while it stays in known pixels. */
    fun reveal(
        nx: Float,
        ny: Float,
        width: Float,
        height: Float,
        containerWidth: Float = width,
        containerHeight: Float = height
    ): ViewportTransform {
        val (visW, visH) = expandedViewportSize(width, height, containerWidth, containerHeight)
        val (x, y) = normalizedToScreen(nx, ny, width, height, visW, visH)
        val margin = minOf(56f, visW / 6f, visH / 6f)
        val dx = when {
            x < margin -> margin - x
            x > visW - margin -> visW - margin - x
            else -> 0f
        }
        val dy = when {
            y < margin -> margin - y
            y > visH - margin -> visH - margin - y
            else -> 0f
        }
        return copy(
            offset = clampOffset(
                offset + Offset(dx, dy),
                width,
                height,
                scale,
                containerWidth,
                containerHeight
            )
        )
    }

    fun reclamp(
        viewWidth: Float,
        viewHeight: Float,
        containerWidth: Float = viewWidth,
        containerHeight: Float = viewHeight
    ): ViewportTransform = copy(
        offset = clampOffset(offset, viewWidth, viewHeight, scale, containerWidth, containerHeight)
    )

    /** Preserve the normalized content at the viewport center across layout/orientation changes. */
    fun resized(
        previousWidth: Float,
        previousHeight: Float,
        viewWidth: Float,
        viewHeight: Float,
        containerWidth: Float = viewWidth,
        containerHeight: Float = viewHeight
    ): ViewportTransform {
        if (previousWidth <= 0f || previousHeight <= 0f) {
            return reclamp(viewWidth, viewHeight, containerWidth, containerHeight)
        }
        val scaledOffset = Offset(
            offset.x * viewWidth / previousWidth,
            offset.y * viewHeight / previousHeight
        )
        return copy(
            offset = clampOffset(
                scaledOffset,
                viewWidth,
                viewHeight,
                scale,
                containerWidth,
                containerHeight
            )
        )
    }

    /**
     * 精准逆映射：将容器（或视口）触控坐标映射回内容空间归一化坐标 [0f, 1f]。
     */
    fun screenToNormalized(
        screenX: Float,
        screenY: Float,
        viewWidth: Float,
        viewHeight: Float,
        containerWidth: Float = viewWidth,
        containerHeight: Float = viewHeight
    ): Pair<Float, Float> {
        val bw = viewWidth.coerceAtLeast(1f)
        val bh = viewHeight.coerceAtLeast(1f)
        val cx = containerWidth.coerceAtLeast(1f) / 2f
        val cy = containerHeight.coerceAtLeast(1f) / 2f
        val s = scale.coerceAtLeast(0.01f)

        val normX = (0.5f + (screenX - cx - offset.x) / (bw * s)).coerceIn(0f, 1f)
        val normY = (0.5f + (screenY - cy - offset.y) / (bh * s)).coerceIn(0f, 1f)
        return Pair(normX, normY)
    }

    /**
     * 正向映射：将内容空间归一化坐标映射至目标容器（或视口）像素坐标。
     */
    fun normalizedToScreen(
        normX: Float,
        normY: Float,
        viewWidth: Float,
        viewHeight: Float,
        containerWidth: Float = viewWidth,
        containerHeight: Float = viewHeight
    ): Pair<Float, Float> {
        val bw = viewWidth.coerceAtLeast(1f)
        val bh = viewHeight.coerceAtLeast(1f)
        val cx = containerWidth.coerceAtLeast(1f) / 2f
        val cy = containerHeight.coerceAtLeast(1f) / 2f

        val screenX = cx + (normX - 0.5f) * bw * scale + offset.x
        val screenY = cy + (normY - 0.5f) * bh * scale + offset.y
        return Pair(screenX, screenY)
    }
}
