package `fun`.hackerlife.weavelo.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class AdaptiveLayoutTest {
    @Test fun classifiesPortraitLandscapeAndFoldableExpandedCorrectly() {
        // Standard phone portrait
        assertEquals(AdaptiveLayoutClass.COMPACT_PORTRAIT, classifyAdaptiveLayout(412, 915))
        // Narrow foldable cover screen portrait
        assertEquals(AdaptiveLayoutClass.COMPACT_PORTRAIT, classifyAdaptiveLayout(360, 820))
        // Standard phone landscape (short height, wide width)
        assertEquals(AdaptiveLayoutClass.COMPACT_LANDSCAPE, classifyAdaptiveLayout(915, 412))
        assertEquals(AdaptiveLayoutClass.COMPACT_LANDSCAPE, classifyAdaptiveLayout(780, 360))
        // Unfolded foldable / tablet
        assertEquals(AdaptiveLayoutClass.EXPANDED, classifyAdaptiveLayout(720, 760))
        assertEquals(AdaptiveLayoutClass.EXPANDED, classifyAdaptiveLayout(840, 680))
    }
}
