package `fun`.hackerlife.weavelo

import androidx.compose.ui.geometry.Offset
import `fun`.hackerlife.weavelo.ui.TwoFingerGestureMode
import `fun`.hackerlife.weavelo.ui.ViewportTransform
import `fun`.hackerlife.weavelo.ui.classifyTwoFingerGesture
import org.junit.Assert.*
import org.junit.Test

class ViewportTransformTest {
    @Test fun viewportDemandTracksContinuousPanAndZoomWithNoCoordinateDrift() {
        assertArrayEquals(intArrayOf(0,0,1_000_000,1_000_000),ViewportTransform().visibleRect(1000f,600f))
        var t=ViewportTransform(scale=2f)
        assertArrayEquals(intArrayOf(250_000,250_000,500_000,500_000),t.visibleRect(1000f,600f))
        for (offset in listOf(Offset(500f,300f),Offset(-500f,-300f),Offset.Zero)) {
            t=t.copy(offset=offset)
            val rect=t.visibleRect(1000f,600f)
            val (x,y)=t.screenToNormalized(0f,0f,1000f,600f)
            assertEquals(x,rect[0]/1_000_000f,.000002f);assertEquals(y,rect[1]/1_000_000f,.000002f)
            assertTrue(rect[0]+rect[2]<=1_000_000 && rect[1]+rect[3]<=1_000_000)
        }
        val followed=t.reveal(.9f,.9f,1000f,600f)
        val screen=followed.normalizedToScreen(.9f,.9f,1000f,600f)
        assertTrue(screen.first in 0f..1000f && screen.second in 0f..600f)
        assertEquals(Offset.Zero,followed.reset().offset)
    }

    @Test
    fun resizingPreservesTheNormalizedCenterFocus() {
        val original = ViewportTransform(2.2f, Offset(180f, -120f))
        val resized = original.resized(1000f, 800f, 500f, 1600f)
        val before = original.screenToNormalized(500f, 400f, 1000f, 800f)
        val after = resized.screenToNormalized(250f, 800f, 500f, 1600f)
        assertEquals(before.first, after.first, .00001f)
        assertEquals(before.second, after.second, .00001f)
        assertEquals(90f, resized.offset.x, .0001f)
        assertEquals(-240f, resized.offset.y, .0001f)
    }

    @Test
    fun firstLayoutAndOneXResizeHaveNoInvalidOffset() {
        assertEquals(Offset.Zero, ViewportTransform().resized(0f, 0f, 1080f, 600f).offset)
        assertEquals(Offset.Zero, ViewportTransform(offset = Offset(100f, 100f)).resized(1000f, 800f, 500f, 400f).offset)
    }

    @Test
    fun initialScaleIsOneAndOffsetIsZero() {
        val transform = ViewportTransform()
        assertEquals(1.0f, transform.scale, 0.0001f)
        assertEquals(Offset.Zero, transform.offset)
    }

    @Test
    fun scaleLimitsAreRespected() {
        val transform = ViewportTransform()
        // 放大测试：缩放因子 5.0，上限应被限制在 4x
        val zoomedIn = transform.update(zoomFactor = 5.0f, panDelta = Offset.Zero, viewWidth = 1920f, viewHeight = 1080f)
        assertEquals(4.0f, zoomedIn.scale, 0.0001f)

        // 缩小测试：缩放因子 0.2，下限应被限制在 1.0x
        val zoomedOut = zoomedIn.update(zoomFactor = 0.1f, panDelta = Offset.Zero, viewWidth = 1920f, viewHeight = 1080f)
        assertEquals(1.0f, zoomedOut.scale, 0.0001f)
        assertEquals(Offset.Zero, zoomedOut.offset)
    }

    @Test
    fun panOffsetClampedToViewportBoundaries() {
        val transform = ViewportTransform(scale = 2.0f)
        val viewWidth = 1000f
        val viewHeight = 600f
        // 在 2.0x 下，最大平移量为 (1000 * 1.0) / 2 = 500, (600 * 1.0) / 2 = 300
        val clamped = transform.clampOffset(Offset(800f, -500f), viewWidth, viewHeight)
        assertEquals(500f, clamped.x, 0.0001f)
        assertEquals(-300f, clamped.y, 0.0001f)

        // 1.0x 下偏移量强制归零（杜绝边缘黑边）
        val zeroClamped = transform.clampOffset(Offset(100f, 100f), viewWidth, viewHeight, currentScale = 1.0f)
        assertEquals(Offset.Zero, zeroClamped)
    }

    @Test
    fun resetRestoresInitialScaleAndOffset() {
        val transform = ViewportTransform(scale = 2.8f, offset = Offset(150f, -80f))
        val reset = transform.reset()
        assertEquals(1.0f, reset.scale, 0.0001f)
        assertEquals(Offset.Zero, reset.offset)
    }

    @Test
    fun screenToNormalizedProducesIdentityAtOneX() {
        val transform = ViewportTransform()
        val viewWidth = 1000f
        val viewHeight = 500f

        val (normX, normY) = transform.screenToNormalized(250f, 100f, viewWidth, viewHeight)
        assertEquals(0.25f, normX, 0.0001f)
        assertEquals(0.20f, normY, 0.0001f)
    }

    @Test
    fun centerPointStaysAtCenterWhenZoomedWithoutPan() {
        val transform = ViewportTransform(scale = 2.5f)
        val viewWidth = 1200f
        val viewHeight = 800f

        val (normX, normY) = transform.screenToNormalized(600f, 400f, viewWidth, viewHeight)
        assertEquals(0.5f, normX, 0.0001f)
        assertEquals(0.5f, normY, 0.0001f)
    }

    @Test
    fun inverseMappingHasZeroDriftAcrossScalesAndOffsets() {
        val viewWidth = 1920f
        val viewHeight = 1080f
        val testScales = listOf(1.0f, 1.25f, 2.0f, 3.0f, 4.0f)
        val testOffsets = listOf(
            Offset.Zero,
            Offset(100f, -50f),
            Offset(-200f, 150f),
            Offset(300f, 200f)
        )
        val testNormPoints = listOf(
            0.0f to 0.0f,
            0.1f to 0.9f,
            0.5f to 0.5f,
            0.25f to 0.75f,
            1.0f to 1.0f
        )

        for (scale in testScales) {
            for (unclampedOffset in testOffsets) {
                val transform = ViewportTransform(scale = scale)
                val clampedOffset = transform.clampOffset(unclampedOffset, viewWidth, viewHeight)
                val activeTransform = transform.copy(offset = clampedOffset)

                for ((origX, origY) in testNormPoints) {
                    // 正向：内容坐标 -> 屏幕像素
                    val (screenX, screenY) = activeTransform.normalizedToScreen(origX, origY, viewWidth, viewHeight)

                    // 逆向：屏幕像素 -> 归一化内容坐标
                    val (mappedX, mappedY) = activeTransform.screenToNormalized(screenX, screenY, viewWidth, viewHeight)

                    // 验证零漂移可逆性（精度 1e-4）
                    assertEquals("Zero drift failed for scale=$scale, offset=$clampedOffset, point=($origX, $origY)",
                        origX, mappedX, 0.0005f)
                    assertEquals("Zero drift failed for scale=$scale, offset=$clampedOffset, point=($origX, $origY)",
                        origY, mappedY, 0.0005f)
                }
            }
        }
    }

    @Test
    fun twoFingerGestureArbitrationPreservesRemoteScrollAcrossZoomLevels() {
        val initialDistance = 200f
        val initialCentroid = Offset(500f, 300f)

        assertEquals(
            TwoFingerGestureMode.UNDECIDED,
            classifyTwoFingerGesture(
                currentScale = 1f,
                initialDistance = initialDistance,
                currentDistance = 203f,
                initialCentroid = initialCentroid,
                currentCentroid = Offset(503f, 302f)
            )
        )
        // Parallel two-finger drag scrolls remote apps at 1x AND while zoomed in.
        for (scale in listOf(1f, 2f, 3.5f)) {
            assertEquals(
                TwoFingerGestureMode.REMOTE_GESTURE,
                classifyTwoFingerGesture(
                    currentScale = scale,
                    initialDistance = initialDistance,
                    currentDistance = 206f,
                    initialCentroid = initialCentroid,
                    currentCentroid = Offset(500f, 336f)
                )
            )
        }
    }

    @Test
    fun twoFingerGestureArbitrationClaimsRealPinchAcrossZoomLevels() {
        val initialCentroid = Offset(500f, 300f)
        for (scale in listOf(1f, 2.2f)) {
            assertEquals(
                TwoFingerGestureMode.LOCAL_TRANSFORM,
                classifyTwoFingerGesture(
                    currentScale = scale,
                    initialDistance = 200f,
                    currentDistance = 224f,
                    initialCentroid = initialCentroid,
                    currentCentroid = initialCentroid
                )
            )
            assertEquals(
                TwoFingerGestureMode.LOCAL_TRANSFORM,
                classifyTwoFingerGesture(
                    currentScale = scale,
                    initialDistance = 200f,
                    currentDistance = 172f,
                    initialCentroid = initialCentroid,
                    currentCentroid = Offset(508f, 304f)
                )
            )
        }
    }

    @Test
    fun reclampAfterViewportResizeKeepsZoomedContentCoveringViewport() {
        val transform = ViewportTransform(scale = 2f, offset = Offset(500f, 300f))
        val resized = transform.reclamp(400f, 300f)
        assertEquals(200f, resized.offset.x, 0.0001f)
        assertEquals(150f, resized.offset.y, 0.0001f)
    }

    @Test
    fun focalPointPinchZoomPreservesFocalContent() {
        val initial = ViewportTransform(scale = 1.0f)
        val viewWidth = 1000f
        val viewHeight = 600f
        val focalPoint = Offset(300f, 200f)

        // 缩放前焦点对应的归一化内容坐标
        val (initialNormX, initialNormY) = initial.screenToNormalized(focalPoint.x, focalPoint.y, viewWidth, viewHeight)
        assertEquals(0.3f, initialNormX, 0.0001f)
        assertEquals(0.33333f, initialNormY, 0.0001f)

        // 以 (300, 200) 为手势中心放大到 2.0x
        val zoomed = initial.update(
            zoomFactor = 2.0f,
            panDelta = Offset.Zero,
            viewWidth = viewWidth,
            viewHeight = viewHeight,
            focalPoint = focalPoint
        )
        assertEquals(2.0f, zoomed.scale, 0.0001f)

        // 放大后相同屏幕焦点位置下的归一化内容坐标必须保持恒定（零漂移）
        val (zoomedNormX, zoomedNormY) = zoomed.screenToNormalized(focalPoint.x, focalPoint.y, viewWidth, viewHeight)
        assertEquals(initialNormX, zoomedNormX, 0.0001f)
        assertEquals(initialNormY, zoomedNormY, 0.0001f)

        // 验证正向映射：屏幕像素点仍为 (300, 200)
        val (screenX, screenY) = zoomed.normalizedToScreen(initialNormX, initialNormY, viewWidth, viewHeight)
        assertEquals(focalPoint.x, screenX, 0.0001f)
        assertEquals(focalPoint.y, screenY, 0.0001f)
    }

    @Test
    fun portraitContainerExpandsVerticalVisibleAreaAndPreservesZeroDriftMapping() {
        // 1080x2400 竖屏容器查看 16:9 桌面（1.0x 基准尺寸 1080x607.5）
        val baseW = 1080f
        val baseH = 607.5f
        val containerW = 1080f
        val containerH = 2400f

        val oneX = ViewportTransform(scale = 1f, maxScale = 6f)
        val (w1, h1) = oneX.expandedViewportSize(baseW, baseH, containerW, containerH)
        assertEquals(1080f, w1, 0.01f)
        assertEquals(607.5f, h1, 0.01f)
        assertArrayEquals(
            intArrayOf(0, 0, 1_000_000, 1_000_000),
            oneX.visibleRect(baseW, baseH, containerW, containerH)
        )

        // 2.0x 放大时，纵向可视高度从 607.5px 扩展至 1215px（物理可视高度翻倍），且完整显示 100% 桌面高度
        val twoX = oneX.update(2f, Offset.Zero, baseW, baseH, Offset(540f, 1200f), containerW, containerH)
        val (w2, h2) = twoX.expandedViewportSize(baseW, baseH, containerW, containerH)
        assertEquals(1080f, w2, 0.01f)
        assertEquals(1215f, h2, 0.01f)
        assertArrayEquals(
            intArrayOf(250_000, 0, 500_000, 1_000_000),
            twoX.visibleRect(baseW, baseH, containerW, containerH)
        )

        // 一键纵向铺满：可视高度铺满整块 2400px 竖屏
        val filled = oneX.fillHeight(baseW, baseH, containerW, containerH)
        val (wf, hf) = filled.expandedViewportSize(baseW, baseH, containerW, containerH)
        assertEquals(1080f, wf, 0.01f)
        assertEquals(2400f, hf, 0.5f)

        // 5.0x 深度放大 + 纵向/横向平移下正逆映射零漂移
        val fiveX = ViewportTransform(scale = 5f, maxScale = 6f).let {
            it.copy(offset = it.clampOffset(Offset(-400f, 260f), baseW, baseH, 5f, containerW, containerH))
        }
        for ((nx, ny) in listOf(0.15f to 0.1f, 0.5f to 0.5f, 0.85f to 0.9f)) {
            val (sx, sy) = fiveX.normalizedToScreen(nx, ny, baseW, baseH, containerW, containerH)
            val (mx, my) = fiveX.screenToNormalized(sx, sy, baseW, baseH, containerW, containerH)
            assertEquals(nx, mx, 0.0005f)
            assertEquals(ny, my, 0.0005f)
        }
    }
}
