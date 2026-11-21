package `fun`.hackerlife.weavelo

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test

class WindowSwitcherLogicTest {
    @Test
    fun workspaceStateDefaultsToExclusiveSingleWindowMode() {
        assertFalse(WorkspaceState().grid)
    }

    @Test
    fun cycleWorkspacePaneIdWrapsForwardAndBackward() {
        assertNull(cycleWorkspacePaneId(emptyList(), 0, 1))
        assertEquals(1, cycleWorkspacePaneId(listOf(1), 1, 1))
        assertEquals(2, cycleWorkspacePaneId(listOf(1, 2, 3), 1, 1))
        assertEquals(1, cycleWorkspacePaneId(listOf(1, 2, 3), 3, 1))
        assertEquals(3, cycleWorkspacePaneId(listOf(1, 2, 3), 1, -1))
        assertEquals(2, cycleWorkspacePaneId(listOf(1, 2, 3), 99, 1))
    }
}
