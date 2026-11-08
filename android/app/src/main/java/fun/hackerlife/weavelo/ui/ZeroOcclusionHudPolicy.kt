package `fun`.hackerlife.weavelo.ui

enum class BackPressAction {
    DismissLocalOverlay,
    ToggleControls,
}

object ZeroOcclusionHudPolicy {
    const val MIN_VERTICAL_BIAS: Float = 0.14f
    const val MAX_VERTICAL_BIAS: Float = 0.86f

    /** Only the publisher's source directory identifies Android; touch capability does not. */
    fun isMobileTarget(hasMediaProjection: Boolean, isCamera: Boolean): Boolean = hasMediaProjection && !isCamera

    fun supportsRemoteHome(hasMediaProjection: Boolean, isCamera: Boolean, source: String): Boolean =
        isMobileTarget(hasMediaProjection, isCamera) && source == "MainDisplay"

    fun supportsRemoteBack(hasMediaProjection: Boolean, isCamera: Boolean, source: String, isWindow: Boolean): Boolean =
        isMobileTarget(hasMediaProjection, isCamera) && (source == "MainDisplay" || isWindow)

    /** Reveal only on a deliberate inward movement from the right handle. */
    fun revealsControls(deltaX: Float, thresholdPx: Float): Boolean =
        deltaX.isFinite() && thresholdPx.isFinite() && thresholdPx > 0f && deltaX <= -thresholdPx

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

    /** Back first dismisses local UI; it never disconnects a live session. */
    fun evaluateBackPress(hasLocalOverlayOpen: Boolean): BackPressAction =
        if (hasLocalOverlayOpen) BackPressAction.DismissLocalOverlay else BackPressAction.ToggleControls
}
