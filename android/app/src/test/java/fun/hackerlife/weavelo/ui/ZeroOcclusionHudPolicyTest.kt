package `fun`.hackerlife.weavelo.ui

import `fun`.hackerlife.weavelo.isAndroidProjectionSource
import org.junit.Assert.assertEquals
import org.junit.Test

class ZeroOcclusionHudPolicyTest {
    @Test
    fun androidMarkerRequiresTheProjectionMainDisplay() {
        assertEquals(true, isAndroidProjectionSource("MainDisplay", "MediaProjection"))
        assertEquals(false, isAndroidProjectionSource("Window", "MediaProjection"))
        assertEquals(false, isAndroidProjectionSource("MainDisplay", "Desktop"))
    }

    @Test
    fun mobileClassificationRequiresPublisherMarkerAndExcludesCamera() {
        assertEquals(true, ZeroOcclusionHudPolicy.isMobileTarget(true, false))
        assertEquals(false, ZeroOcclusionHudPolicy.isMobileTarget(false, false))
        assertEquals(false, ZeroOcclusionHudPolicy.isMobileTarget(true, true))
        assertEquals(true, ZeroOcclusionHudPolicy.supportsRemoteHome(true, false, "MainDisplay"))
        assertEquals(false, ZeroOcclusionHudPolicy.supportsRemoteHome(true, false, "Window"))
        assertEquals(false, ZeroOcclusionHudPolicy.supportsRemoteHome(false, false, "MainDisplay"))
        assertEquals(false, ZeroOcclusionHudPolicy.supportsRemoteHome(true, true, "MainDisplay"))
        assertEquals(true, ZeroOcclusionHudPolicy.supportsRemoteBack(true, false, "MainDisplay", false))
        assertEquals(true, ZeroOcclusionHudPolicy.supportsRemoteBack(true, false, "Window", true))
        assertEquals(false, ZeroOcclusionHudPolicy.supportsRemoteBack(true, false, "Other", false))
        assertEquals(false, ZeroOcclusionHudPolicy.supportsRemoteBack(false, false, "Window", true))
    }

    @Test
    fun onlyDeliberateInwardHandleDragRevealsControls() {
        assertEquals(true, ZeroOcclusionHudPolicy.revealsControls(-24f, 24f))
        assertEquals(false, ZeroOcclusionHudPolicy.revealsControls(-23f, 24f))
        assertEquals(false, ZeroOcclusionHudPolicy.revealsControls(30f, 24f))
        assertEquals(false, ZeroOcclusionHudPolicy.revealsControls(Float.NaN, 24f))
        assertEquals(false, ZeroOcclusionHudPolicy.revealsControls(-24f, 0f))
    }

    @Test
    fun edgePillStaysAwayFromTopCutoutAndBottomGestureBar() {
        assertEquals(0.45f, ZeroOcclusionHudPolicy.clampEdgePillFraction(Float.NaN), 0.0001f)
        assertEquals(ZeroOcclusionHudPolicy.MIN_VERTICAL_BIAS, ZeroOcclusionHudPolicy.clampEdgePillFraction(-0.5f), 0.0001f)
        assertEquals(ZeroOcclusionHudPolicy.MAX_VERTICAL_BIAS, ZeroOcclusionHudPolicy.clampEdgePillFraction(0.99f), 0.0001f)
        assertEquals(0.55f, ZeroOcclusionHudPolicy.dragEdgePillFraction(0.45f, 100f, 1000f), 0.0001f)
    }

    @Test
    fun edgePillGhostsDuringActiveTouchAndTyping() {
        assertEquals(0.24f, ZeroOcclusionHudPolicy.edgePillAlpha(touching = true, imeVisible = false), 0.0001f)
        assertEquals(0.32f, ZeroOcclusionHudPolicy.edgePillAlpha(touching = false, imeVisible = true), 0.0001f)
        assertEquals(0.88f, ZeroOcclusionHudPolicy.edgePillAlpha(touching = false, imeVisible = false), 0.0001f)
    }

    @Test
    fun backNeverDisconnectsAndDismissesLocalOverlaysFirst() {
        assertEquals(BackPressAction.DismissLocalOverlay, ZeroOcclusionHudPolicy.evaluateBackPress(true))
        assertEquals(BackPressAction.ToggleControls, ZeroOcclusionHudPolicy.evaluateBackPress(false))
    }
}
