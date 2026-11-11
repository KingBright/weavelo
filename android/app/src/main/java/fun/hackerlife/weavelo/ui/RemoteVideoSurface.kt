package `fun`.hackerlife.weavelo.ui

import android.content.Context
import android.view.SurfaceView
import androidx.compose.foundation.layout.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.viewinterop.AndroidView
import `fun`.hackerlife.weavelo.TouchMode
import `fun`.hackerlife.weavelo.VideoDimensions
import kotlin.math.roundToInt

/** One visible rectangle owns the native surface, sizing and inverse touch transform. */
@Composable
internal fun RemoteViewport(
    dimensions: VideoDimensions,
    transformState: MutableState<ViewportTransform>,
    enabled: Boolean,
    contentKey: Any?,
    modifier: Modifier = Modifier,
    viewportModifier: Modifier = Modifier,
    touchMode: TouchMode = TouchMode.DIRECT,
    rotation: Int = 0,
    expandToContainer: Boolean = false,
    onViewportSizeChanged: (IntSize) -> Unit = {},
    onContainerSizeChanged: (IntSize) -> Unit = {},
    sendTouch: (Int, Int, Float, Float, Float, Long) -> Unit,
    sendTouchFrame: (List<RemoteTouchChange>) -> Unit = {},
    overlay: @Composable BoxScope.() -> Unit = {},
    description: String? = null,
    factory: (Context) -> SurfaceView
) {
    var baseSize by remember { mutableStateOf(IntSize.Zero) }
    var containerSize by remember { mutableStateOf(IntSize.Zero) }
    val onSize by rememberUpdatedState(onViewportSizeChanged)
    val onContainerSize by rememberUpdatedState(onContainerSizeChanged)
    require(rotation in listOf(0, 90, 180, 270)) { "Unsupported video rotation" }
    val quarterTurn = rotation % 180 != 0
    val displayAspect = if (quarterTurn) 1f / dimensions.aspectRatio else dimensions.aspectRatio
    val density = LocalDensity.current

    BoxWithConstraints(modifier, contentAlignment = Alignment.Center) {
        val baseWidth = minOf(maxWidth, maxHeight * displayAspect)
        val baseHeight = baseWidth / displayAspect

        if (!expandToContainer) {
            Box(
                viewportModifier
                    .width(baseWidth)
                    .height(baseHeight)
                    .clipToBounds()
                    .onSizeChanged { next ->
                        val previous = baseSize
                        baseSize = next
                        containerSize = next
                        transformState.value = transformState.value.resized(
                            previous.width.toFloat(),
                            previous.height.toFloat(),
                            next.width.toFloat(),
                            next.height.toFloat()
                        )
                        onSize(next)
                        onContainerSize(next)
                    }
                    .remoteViewportGestures(
                        transformState = transformState,
                        touchMode = touchMode,
                        viewportSize = baseSize,
                        enabled = enabled && touchMode != TouchMode.VIEW,
                        contentKey = contentKey to rotation,
                        sendTouch = sendTouch,
                        sendTouchFrame = sendTouchFrame
                    )
            ) {
                RemoteVideoSurface(
                    transformState.value,
                    Modifier
                        .align(Alignment.Center)
                        .requiredSize(
                            if (quarterTurn) baseHeight else baseWidth,
                            if (quarterTurn) baseWidth else baseHeight
                        ),
                    rotation,
                    description,
                    factory
                )
                overlay()
            }
        } else {
            // Stationary outer gesture frame prevents coordinate-origin jitter
            // while the inner clipped viewport expands vertically/horizontally on zoom.
            val scale = transformState.value.scale.coerceAtLeast(1f)
            val visibleWidth = minOf(maxWidth, baseWidth * scale)
            val visibleHeight = minOf(maxHeight, baseHeight * scale)
            val nextBasePx = with(density) {
                IntSize(
                    baseWidth.toPx().roundToInt().coerceAtLeast(1),
                    baseHeight.toPx().roundToInt().coerceAtLeast(1)
                )
            }

            Box(
                Modifier
                    .fillMaxSize()
                    .onSizeChanged { nextContainer ->
                        val previousBase = baseSize
                        baseSize = nextBasePx
                        containerSize = nextContainer
                        transformState.value = transformState.value.resized(
                            previousBase.width.toFloat(),
                            previousBase.height.toFloat(),
                            nextBasePx.width.toFloat(),
                            nextBasePx.height.toFloat(),
                            nextContainer.width.toFloat(),
                            nextContainer.height.toFloat()
                        )
                        onSize(nextBasePx)
                        onContainerSize(nextContainer)
                    }
                    .remoteViewportGestures(
                        transformState = transformState,
                        touchMode = touchMode,
                        viewportSize = containerSize,
                        baseViewportSize = nextBasePx,
                        restrictSingleTouchToVideo = true,
                        enabled = enabled && touchMode != TouchMode.VIEW,
                        contentKey = (contentKey to rotation) to nextBasePx,
                        sendTouch = sendTouch,
                        sendTouchFrame = sendTouchFrame
                    ),
                contentAlignment = Alignment.Center
            ) {
                Box(
                    viewportModifier
                        .width(visibleWidth)
                        .height(visibleHeight)
                        .clipToBounds()
                ) {
                    RemoteVideoSurface(
                        transformState.value,
                        Modifier
                            .align(Alignment.Center)
                            .requiredSize(
                                if (quarterTurn) baseHeight else baseWidth,
                                if (quarterTurn) baseWidth else baseHeight
                            ),
                        rotation,
                        description,
                        factory
                    )
                    overlay()
                }
            }
        }
    }
}

/** Production video surface composition, also exercised by the offline instrumentation fixture. */
@Composable
internal fun RemoteVideoSurface(
    transform: ViewportTransform,
    modifier: Modifier = Modifier,
    rotation: Int = 0,
    description: String? = null,
    factory: (Context) -> SurfaceView
) {
    AndroidView(
        modifier = modifier.graphicsLayer {
            scaleX = transform.scale
            scaleY = transform.scale
            translationX = transform.offset.x
            translationY = transform.offset.y
            rotationZ = rotation.toFloat()
        },
        factory = factory,
        update = { if (description != null) it.contentDescription = description }
    )
}
