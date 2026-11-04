package `fun`.hackerlife.weavelo

import java.nio.ByteBuffer
import java.nio.ByteOrder

/** The geometry carried by this access unit, never the latest requested ROI. */
internal data class VideoFrameGeometry(
    val sourceRevision: Int, val demandRevision: Long, val revision: Long,
    val sourceWidth: Int, val sourceHeight: Int,
    val x: Int, val y: Int, val cropWidth: Int, val cropHeight: Int,
    val width: Int, val height: Int
) {
    fun valid(): Boolean = revision != 0L && sourceWidth in 2..32768 && sourceHeight in 2..32768 &&
        width in 2..8192 && height in 2..8192 && width.toLong() * height <= 33_554_432 &&
        x >= 0 && y >= 0 && cropWidth >= width && cropHeight >= height &&
        x.toLong() + cropWidth <= sourceWidth && y.toLong() + cropHeight <= sourceHeight
    fun contains(nx: Float, ny: Float): Boolean = nx.isFinite() && ny.isFinite() &&
        nx * sourceWidth >= x && ny * sourceHeight >= y &&
        nx * sourceWidth <= x.toLong() + cropWidth && ny * sourceHeight <= y.toLong() + cropHeight
    fun header(): ByteArray = ByteBuffer.allocate(HEADER_BYTES).order(ByteOrder.BIG_ENDIAN).apply {
        put(MAGIC); putInt(sourceRevision); putLong(demandRevision); putLong(revision)
        for (value in intArrayOf(sourceWidth, sourceHeight, x, y, cropWidth, cropHeight, width, height)) putInt(value)
    }.array()
    companion object {
        const val HEADER_BYTES = 60
        private val MAGIC = byteArrayOf(87, 86, 82, 79, 73, 0, 1, 0)
        /** A camera keeps normal Annex-B. A malformed screen header fails closed. */
        fun accessUnit(bytes: ByteArray, offset: Int): Pair<VideoFrameGeometry?, Int> {
            require(offset in 0 until bytes.size)
            if (bytes.size - offset < MAGIC.size || MAGIC.indices.any { bytes[offset + it] != MAGIC[it] }) return null to offset
            require(bytes.size - offset > HEADER_BYTES) { "Truncated viewport frame" }
            val b = ByteBuffer.wrap(bytes, offset + MAGIC.size, HEADER_BYTES - MAGIC.size).order(ByteOrder.BIG_ENDIAN)
            val g = VideoFrameGeometry(b.int, b.long, b.long, b.int, b.int, b.int, b.int, b.int, b.int, b.int, b.int)
            require(g.valid()) { "Invalid viewport frame geometry" }
            val start = offset + HEADER_BYTES
            require(bytes.size - start >= 4 && bytes[start] == 0.toByte() && bytes[start + 1] == 0.toByte() &&
                (bytes[start + 2] == 1.toByte() || bytes[start + 2] == 0.toByte() && bytes[start + 3] == 1.toByte())) { "Viewport has no HEVC access unit" }
            return g to start
        }
    }
}
