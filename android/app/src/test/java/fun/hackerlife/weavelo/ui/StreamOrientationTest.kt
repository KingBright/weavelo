package `fun`.hackerlife.weavelo.ui

import android.content.pm.ActivityInfo
import org.junit.Assert.assertEquals
import org.junit.Test

class StreamOrientationTest {
    @Test fun defaultModeRespectsSystemRotationLock() {
        assertEquals(ActivityInfo.SCREEN_ORIENTATION_FULL_USER, resolveStreamOrientation(null))
    }

    @Test fun manualOverridesSwitchExplicitlyWhileRespectingUserLock() {
        assertEquals(ActivityInfo.SCREEN_ORIENTATION_USER_LANDSCAPE, resolveStreamOrientation(true))
        assertEquals(ActivityInfo.SCREEN_ORIENTATION_USER_PORTRAIT, resolveStreamOrientation(false))
    }
}
