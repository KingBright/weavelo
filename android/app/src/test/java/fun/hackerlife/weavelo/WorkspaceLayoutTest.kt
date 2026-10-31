package `fun`.hackerlife.weavelo

import org.junit.Assert.assertEquals
import org.junit.Test

class WorkspaceLayoutTest {
    @Test fun portraitUsesTwoFullWidthWindowsInsteadOfTinySideBySideVideo() {
        assertEquals(1, workspaceGridColumns(460f, 730f, listOf(1.31f, 1.31f)))
    }
    @Test fun landscapeUsesAvailableWidthForTwoWindows() {
        assertEquals(2, workspaceGridColumns(900f, 340f, listOf(1.31f, 1.31f)))
    }
    @Test fun keyboardInsetCanChangeLayoutWithoutChangingSourceCoordinates() {
        assertEquals(1, workspaceGridColumns(460f, 730f, listOf(1.31f, 1.31f)))
        assertEquals(2, workspaceGridColumns(460f, 250f, listOf(1.31f, 1.31f)))
    }
    @Test fun fourDesktopWindowsUseTwoRows() {
        assertEquals(2, workspaceGridColumns(460f, 730f, List(4) { 1.78f }))
    }
    @Test fun emptyUnmeasuredAndSingleSourceLayoutsRemainOneColumn() {
        assertEquals(1, workspaceGridColumns(0f, 730f, listOf(1.31f, 1.31f)))
        assertEquals(1, workspaceGridColumns(460f, 730f, emptyList()))
        assertEquals(1, workspaceGridColumns(460f, 730f, listOf(Float.NaN)))
        assertEquals(1, workspaceGridColumns(900f, 340f, listOf(1.31f)))
    }
}
