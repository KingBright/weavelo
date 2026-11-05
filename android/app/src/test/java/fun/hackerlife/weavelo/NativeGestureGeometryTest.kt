package `fun`.hackerlife.weavelo

import org.junit.Assert.*
import org.junit.Test

class NativeGestureGeometryTest {
    @Test fun stationaryAndSubpixelMovesDoNotCreateEmptyContinuations() {
        assertFalse(nativeGestureMoved(115f,1520f,115f,1520f))
        assertFalse(nativeGestureMoved(115.1f,1520.1f,115.49f,1520.49f))
        assertTrue(nativeGestureMoved(115.49f,1520f,115.5f,1520f))
        assertTrue(nativeGestureMoved(115f,1520.49f,115f,1520.5f))
        assertTrue(nativeGestureMoved(115.5f,1520f,115.49f,1520f))
    }
}
