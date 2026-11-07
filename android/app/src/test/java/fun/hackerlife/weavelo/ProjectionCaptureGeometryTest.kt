package `fun`.hackerlife.weavelo

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ProjectionCaptureGeometryTest {
    @Test fun sameSizeHalfTurnIsAProducerReconfiguration() {
        val initial = ProjectionCaptureGeometry(1080, 1920, 420, displayRotationDegrees(0))
        val halfTurn = initial.copy(rotation = displayRotationDegrees(2))
        assertNull(captureReconfiguration(initial, initial))
        assertEquals(halfTurn, captureReconfiguration(initial, halfTurn))
        assertEquals(180, halfTurn.rotation)
        val landscape = ProjectionCaptureGeometry(1920, 1080, 420, displayRotationDegrees(1))
        val reverseLandscape = landscape.copy(rotation = displayRotationDegrees(3))
        assertEquals(reverseLandscape, captureReconfiguration(landscape, reverseLandscape))
        assertEquals(270, displayRotationDegrees(3))
    }
}
