package `fun`.hackerlife.weavelo.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class ZeroOcclusionHudPolicyTest {
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
    fun twoStageBackGuardPreventsAccidentalDisconnectDuringEdgeSwipe() {
        // 1. Open overlay/keyboard is dismissed first without disconnecting
        assertEquals(
            BackPressAction.DismissLocalOverlay,
            ZeroOcclusionHudPolicy.evaluateBackPress(
                hasLocalOverlayOpen = true,
                lastBackPromptMs = 1_000L,
                nowMs = 1_500L,
            )
        )
        // 2. First back press in full-bleed canvas prompts and reveals the Orbit Pod
        assertEquals(
            BackPressAction.PromptAndRevealPod,
            ZeroOcclusionHudPolicy.evaluateBackPress(
                hasLocalOverlayOpen = false,
                lastBackPromptMs = 0L,
                nowMs = 5_000L,
            )
        )
        // 3. Second back press within 3s window confirms disconnect
        assertEquals(
            BackPressAction.DisconnectSession,
            ZeroOcclusionHudPolicy.evaluateBackPress(
                hasLocalOverlayOpen = false,
                lastBackPromptMs = 5_000L,
                nowMs = 7_500L,
            )
        )
        // 4. Back press after the 3s window expires prompts again instead of disconnecting
        assertEquals(
            BackPressAction.PromptAndRevealPod,
            ZeroOcclusionHudPolicy.evaluateBackPress(
                hasLocalOverlayOpen = false,
                lastBackPromptMs = 5_000L,
                nowMs = 8_500L,
            )
        )
    }
}
