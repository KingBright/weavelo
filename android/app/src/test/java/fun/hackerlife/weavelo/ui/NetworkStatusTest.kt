package `fun`.hackerlife.weavelo.ui

import `fun`.hackerlife.weavelo.SessionState
import `fun`.hackerlife.weavelo.TelemetrySnapshot
import org.junit.Assert.*
import org.junit.Test

class NetworkStatusTest {
    private val measured = TelemetrySnapshot(networkRttMs = 12.0, observedAtElapsedMs = 1000)
    @Test fun healthyRequiresCurrentSessionMeasurements() {
        assertEquals(NetworkTone.ONLINE, networkIndicator(SessionState.STREAMING, measured, 1500).tone)
        assertEquals(NetworkTone.NEUTRAL, networkIndicator(SessionState.STREAMING, measured, 6001).tone)
        assertEquals(NetworkTone.NEUTRAL, networkIndicator(SessionState.STREAMING, TelemetrySnapshot(), 1500).tone)
        assertEquals(NetworkTone.NEUTRAL, networkIndicator(SessionState.STREAMING, measured.copy(networkRttMs = Double.NaN), 1500).tone)
    }
    @Test fun reconnectAndDisconnectNeverReuseHealthyTelemetry() {
        assertEquals("Reconnecting", networkIndicator(SessionState.RECONNECTING, measured, 1500).label)
        assertEquals("Not connected", networkIndicator(SessionState.DISCONNECTED, measured, 1500).label)
        assertEquals("Connecting", networkIndicator(SessionState.CONNECTING, measured, 1500).label)
    }
    @Test fun latencyAndLossDetermineQualityRatherThanFrameRate() {
        assertEquals(NetworkTone.ONLINE, networkIndicator(SessionState.STREAMING, measured.copy(fps = 0f), 1500).tone)
        assertEquals(NetworkTone.WARNING, networkIndicator(SessionState.STREAMING, measured.copy(networkRttMs = 81.0), 1500).tone)
        assertEquals(NetworkTone.ERROR, networkIndicator(SessionState.STREAMING, measured.copy(networkRttMs = 151.0), 1500).tone)
        assertEquals(NetworkTone.WARNING, networkIndicator(SessionState.STREAMING, measured.copy(packetLossPercent = 1f), 1500).tone)
        assertEquals(NetworkTone.WARNING, networkIndicator(SessionState.STREAMING, measured.copy(networkRttMs = null, packetLossPercent = 2f), 1500).tone)
        assertEquals(NetworkTone.ERROR, networkIndicator(SessionState.STREAMING, measured.copy(packetLossPercent = 5f), 1500).tone)
        assertEquals(NetworkTone.NEUTRAL, networkIndicator(SessionState.STREAMING, measured.copy(packetLossPercent = Float.NaN), 1500).tone)
    }
}
