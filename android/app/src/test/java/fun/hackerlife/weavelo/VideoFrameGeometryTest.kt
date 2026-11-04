package `fun`.hackerlife.weavelo

import org.junit.Assert.*
import org.junit.Test

class VideoFrameGeometryTest {
    private val g=VideoFrameGeometry(3,-1L,7,3840,2160,960,540,1920,1080,1280,720)
    @Test fun frameMetadataPreservesUnsignedBitsAndDoesNotEatHevcBytes() {
        val header=g.header()
        assertEquals(60,header.size)
        val nal=byteArrayOf(0,0,0,1,0x26,1,4)
        val bytes=byteArrayOf(1,2,3)+header+nal
        val (decoded,offset)=VideoFrameGeometry.accessUnit(bytes,3)
        assertEquals(g,decoded);assertEquals(63,offset)
        assertArrayEquals(nal,bytes.copyOfRange(offset,bytes.size))
        assertTrue(g.contains(.5f,.5f));assertFalse(g.contains(.1f,.5f))
        assertFalse(g.contains(Float.NaN,.5f))
    }
    @Test fun truncatedOverflowedOrNonVideoPayloadFailsClosed() {
        for (bytes in listOf(g.header(),g.copy(x=Int.MAX_VALUE).header()+byteArrayOf(0,0,1,0x26,1),g.header()+byteArrayOf(4,5,6,7))) {
            try { VideoFrameGeometry.accessUnit(bytes,0);fail("Invalid geometry was accepted") } catch (_: IllegalArgumentException) {}
        }
        assertFalse(g.copy(width=1921).valid());assertFalse(g.copy(revision=0).valid())
    }
    @Test fun cameraHasNoScreenCropOrNewCoordinateSemantics() {
        val nal=byteArrayOf(0,0,0,1,0x26,1)
        assertEquals(null to 0,VideoFrameGeometry.accessUnit(nal,0))
    }
}
