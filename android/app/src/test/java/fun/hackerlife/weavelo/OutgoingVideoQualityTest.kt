package `fun`.hackerlife.weavelo

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class OutgoingVideoQualityTest {
    @Test fun acceptsOriginalAndUsefulBounds() {
        assertTrue(OutgoingVideoQuality().valid())
        assertTrue(OutgoingVideoQuality(0, 1, 100).valid())
        assertTrue(OutgoingVideoQuality(4320, 120, 100_000).valid())
    }

    @Test fun rejectsInvalidValuesBeforeSendingOrStoring() {
        for (value in listOf(OutgoingVideoQuality(1), OutgoingVideoQuality(4321),
            OutgoingVideoQuality(fps = 0), OutgoingVideoQuality(fps = 121),
            OutgoingVideoQuality(bitrateKbps = 99), OutgoingVideoQuality(bitrateKbps = 100_001))) {
            assertFalse(value.valid())
        }
    }
}
