package `fun`.hackerlife.weavelo

import org.junit.Assert.*
import org.junit.Test

class IncomingVideoReceivingTest {
    private fun pane(id: Int, source: String, accepted: Boolean = true) =
        WorkspacePane(id, WorkspaceSource(source, source, 540, 380), 60, 20000, accepted = accepted)

    @Test fun oneLiveWindowContinuesAfterAnotherOfferedWindowFails() {
        assertTrue(incomingVideoReceiving(listOf("first", "failed"), listOf(pane(256, "first")), setOf(256)))
    }
    @Test fun acceptanceAndNativeSurfaceSubmissionAreBothRequired() {
        assertFalse(incomingVideoReceiving(listOf("first"), listOf(pane(256, "first", false)), setOf(256)))
        assertFalse(incomingVideoReceiving(listOf("first"), listOf(pane(256, "first")), emptySet()))
    }
    @Test fun closedOrUnofferedPaneCannotReportReceiving() {
        assertFalse(incomingVideoReceiving(listOf("first"), emptyList(), setOf(256)))
        assertFalse(incomingVideoReceiving(listOf("first"), listOf(pane(512, "other")), setOf(512)))
        assertFalse(incomingVideoReceiving(listOf("first"), listOf(pane(512, "first")), setOf(256)))
    }
}
