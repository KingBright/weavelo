package `fun`.hackerlife.weavelo.ui

import org.junit.Assert.*
import org.junit.Test

class EditorFocusGateTest {
    @Test fun replyRequiresLatestTapAndIsSingleUse() {
        val gate = EditorFocusGate<String>()
        val old = gate.intent()
        gate.invalidate() // another gesture or an explicit keyboard dismissal
        gate.record(old, "1", 100, "old")
        assertNull(gate.take("1", 110))
        val current = gate.intent()
        gate.record(current, "2", 100, "current")
        assertNull(gate.take("1", 110))
        assertEquals("current", gate.take("2", 110))
        assertNull(gate.take("2", 111))
    }
    @Test fun expiredOrInvalidatedGeometryNeverOpensKeyboard() {
        val gate = EditorFocusGate<String>()
        gate.record(gate.intent(), "3", 100, "frame")
        assertNull(gate.take("3", 701))
        gate.record(gate.intent(), "4", 100, "frame")
        gate.invalidate() // pane, geometry, control or inset changed
        assertNull(gate.take("4", 200))
    }
}
