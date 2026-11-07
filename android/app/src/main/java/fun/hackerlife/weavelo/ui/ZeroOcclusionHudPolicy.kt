package `fun`.hackerlife.weavelo.ui

enum class BackPressAction {
    DismissLocalOverlay,
    PromptAndRevealPod,
    DisconnectSession,
}

object ZeroOcclusionHudPolicy {
    const val CONFIRM_WINDOW_MS: Long = 3_000L
    const val MIN_VERTICAL_BIAS: Float = 0.14f
    const val MAX_VERTICAL_BIAS: Float = 0.86f

    /**
     * Clamps the magnetic edge micro-pill's vertical position so it stays clear of
     * top camera cutouts/status bars and bottom gesture navigation bars.
     */
    fun clampEdgePillFraction(fraction: Float): Float {
        if (!fraction.isFinite()) return 0.45f
        return fraction.coerceIn(MIN_VERTICAL_BIAS, MAX_VERTICAL_BIAS)
    }

    /**
     * Updates the vertical fraction from a drag delta in pixels.
     */
    fun dragEdgePillFraction(currentFraction: Float, deltaY: Float, containerHeightPx: Float): Float {
        if (!deltaY.isFinite() || !containerHeightPx.isFinite() || containerHeightPx <= 1f) {
            return clampEdgePillFraction(currentFraction)
        }
        return clampEdgePillFraction(currentFraction + (deltaY / containerHeightPx))
    }

    /**
     * Ghosting alpha for the docked edge micro-pill so it never visually distracts
     * during active touch gestures or text composition.
     */
    fun edgePillAlpha(touching: Boolean, imeVisible: Boolean): Float = when {
        touching -> 0.24f
        imeVisible -> 0.32f
        else -> 0.88f
    }

    /**
     * Two-stage anti-mispress back guard for mobile remote sessions:
     * 1. If any local overlay (keyboard, sticky modifier bar, source picker, actions menu, or expanded HUD pod)
     *    is open, the back gesture closes that local overlay first without disconnecting.
     * 2. In full-bleed canvas mode, the first back swipe reveals the Orbit Pod + anti-mispress hint;
     *    only a second back swipe within [confirmWindowMs] disconnects the session.
     */
    fun evaluateBackPress(
        hasLocalOverlayOpen: Boolean,
        lastBackPromptMs: Long,
        nowMs: Long,
        confirmWindowMs: Long = CONFIRM_WINDOW_MS,
    ): BackPressAction {
        if (hasLocalOverlayOpen) {
            return BackPressAction.DismissLocalOverlay
        }
        if (lastBackPromptMs > 0L && nowMs >= lastBackPromptMs && (nowMs - lastBackPromptMs) <= confirmWindowMs) {
            return BackPressAction.DisconnectSession
        }
        return BackPressAction.PromptAndRevealPod
    }
}
