package `fun`.hackerlife.weavelo.ui

import android.content.Context
import android.view.SurfaceView
import androidx.compose.runtime.Composable
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.foundation.layout.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.viewinterop.AndroidView
import `fun`.hackerlife.weavelo.TouchMode
import `fun`.hackerlife.weavelo.VideoDimensions

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
    onViewportSizeChanged: (IntSize) -> Unit = {},
    sendTouch: (Int, Int, Float, Float, Float, Long) -> Unit,
    overlay: @Composable BoxScope.() -> Unit = {},
    factory: (Context) -> SurfaceView
) {
    var viewportSize by remember { mutableStateOf(IntSize.Zero) }
    val onSize by rememberUpdatedState(onViewportSizeChanged)
    BoxWithConstraints(modifier, contentAlignment = Alignment.Center) {
        val width = minOf(maxWidth, maxHeight * dimensions.aspectRatio)
        Box(viewportModifier.width(width).aspectRatio(dimensions.aspectRatio).clipToBounds()
            .onSizeChanged { next ->
                val previous = viewportSize
                viewportSize = next
                transformState.value = transformState.value.resized(
                    previous.width.toFloat(), previous.height.toFloat(),
                    next.width.toFloat(), next.height.toFloat())
                onSize(next)
            }
            .remoteViewportGestures(transformState, touchMode, viewportSize, enabled, contentKey, sendTouch)
        ) {
            RemoteVideoSurface(transformState.value, Modifier.fillMaxSize(), factory)
            overlay()
        }
    }
}

/** Production video surface composition, also exercised by the offline instrumentation fixture. */
@Composable
internal fun RemoteVideoSurface(
    transform: ViewportTransform,
    modifier: Modifier = Modifier,
    factory: (Context) -> SurfaceView
) {
    AndroidView(
        modifier = modifier.graphicsLayer {
            scaleX = transform.scale
            scaleY = transform.scale
            translationX = transform.offset.x
            translationY = transform.offset.y
        },
        factory = factory
    )
}
