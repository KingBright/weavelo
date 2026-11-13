package `fun`.hackerlife.weavelo

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build
import android.os.Bundle
import android.view.Surface

/** Shared hardware surface encoder for screen and camera producers. No CPU pixel conversion. */
internal class SurfaceHevcEncoder(val plan: Plan) : AutoCloseable {
    data class Plan(val codec: String, val width: Int, val height: Int, val fps: Int, val bitrateKbps: Int,
        val srgb: Boolean = false)
    companion object {
        private val hardware by lazy {
            MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.filter {
                it.isEncoder && it.supportedTypes.any { type -> type.equals(MediaFormat.MIMETYPE_VIDEO_HEVC, true) } &&
                    (if (Build.VERSION.SDK_INT >= 29) it.isHardwareAccelerated
                     else !it.name.startsWith("OMX.google.") && !it.name.startsWith("c2.android."))
            }
        }
        fun exact(width: Int, height: Int, fps: Int, bitrateKbps: Int): Plan? {
            val bits = Math.multiplyExact(bitrateKbps, 1000)
            return hardware.firstOrNull {
                val caps = it.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_HEVC).videoCapabilities
                width % caps.widthAlignment == 0 && height % caps.heightAlignment == 0 &&
                    caps.areSizeAndRateSupported(width, height, fps.toDouble()) && caps.bitrateRange.contains(bits)
            }?.let { Plan(it.name, width, height, fps, bitrateKbps) }
        }
        fun fit(sourceWidth: Int, sourceHeight: Int, width: Int, height: Int, fps: Int, bitrateKbps: Int): Plan? {
            val bits = Math.multiplyExact(bitrateKbps, 1000)
            return hardware.firstNotNullOfOrNull {
                val caps = it.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_HEVC).videoCapabilities
                val size = VideoDimensions.fitEncoder(sourceWidth, sourceHeight, width, height, caps.widthAlignment, caps.heightAlignment)
                if (size != null && caps.areSizeAndRateSupported(size.width, size.height, fps.toDouble()) && caps.bitrateRange.contains(bits))
                    Plan(it.name, size.width, size.height, fps, bitrateKbps) else null
            }
        }
        /** Select only plans at or below every requested bound. */
        fun fitAtMost(sourceWidth: Int, sourceHeight: Int, width: Int, height: Int, fps: Int, bitrateKbps: Int): Plan? {
            if (fps <= 0 || bitrateKbps <= 0) return null
            val bits=bitrateKbps.toLong()*1000
            for (percent in intArrayOf(100,90,80,70,60,50,40,30,25,20,10)) {
                for (codec in hardware) {
                    val caps=codec.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_HEVC).videoCapabilities
                    val targetW=(width.toLong()*percent/100).toInt().coerceAtLeast(1)
                    val targetH=(height.toLong()*percent/100).toInt().coerceAtLeast(1)
                    val size=VideoDimensions.fitEncoder(sourceWidth,sourceHeight,targetW,targetH,caps.widthAlignment,caps.heightAlignment) ?: continue
                    val plan=runCatching {
                        val range=caps.getSupportedFrameRatesFor(size.width,size.height)
                        val selectedFps=minOf(fps,kotlin.math.floor(range.upper).toInt())
                        val selectedKbps=minOf(bits,caps.bitrateRange.upper.toLong())/1000
                        if (selectedFps.toDouble()<range.lower || selectedKbps*1000<caps.bitrateRange.lower ||
                            !caps.areSizeAndRateSupported(size.width,size.height,selectedFps.toDouble())) null
                        else Plan(codec.name,size.width,size.height,selectedFps,selectedKbps.toInt())
                    }.getOrNull()
                    if (plan!=null) return plan
                }
            }
            return null
        }
    }
    private val codec = MediaCodec.createByCodecName(plan.codec)
    val surface: Surface
    private val info = MediaCodec.BufferInfo()
    private var header = ByteArray(0)
    private var keyframe = -1L
    private var ended = false
    init {
        var input: Surface? = null
        try {
            fun buildFormat(withExplicitSrgb: Boolean) =
                MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_HEVC, plan.width, plan.height).apply {
                    setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
                    setInteger(MediaFormat.KEY_BIT_RATE, Math.multiplyExact(plan.bitrateKbps, 1000))
                    setInteger(MediaFormat.KEY_FRAME_RATE, plan.fps)
                    setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 2)
                    setInteger(MediaFormat.KEY_PRIORITY, 0)
                    if (Build.VERSION.SDK_INT >= 29) setInteger(MediaFormat.KEY_MAX_B_FRAMES, 0)
                    if (withExplicitSrgb) {
                        setInteger(MediaFormat.KEY_COLOR_STANDARD, MediaFormat.COLOR_STANDARD_BT709)
                        setInteger(MediaFormat.KEY_COLOR_TRANSFER, MediaFormat.COLOR_TRANSFER_SDR_VIDEO)
                        setInteger(MediaFormat.KEY_COLOR_RANGE, MediaFormat.COLOR_RANGE_FULL)
                    }
                }
            if (plan.srgb) {
                try {
                    codec.configure(buildFormat(true), null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
                } catch (colorError: Exception) {
                    android.util.Log.w("WeaveloMedia", "Encoder rejected explicit full-range sRGB tags; retrying default SDR color", colorError)
                    runCatching { codec.reset() }
                    codec.configure(buildFormat(false), null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
                }
            } else {
                codec.configure(buildFormat(false), null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            }
            input = codec.createInputSurface(); codec.start(); surface = input
        } catch (error: Exception) { input?.release(); codec.release(); throw error }
    }
    fun requestKeyframe(sequence: Long) {
        if (keyframe != sequence) {
            keyframe = sequence
            forceSyncFrame()
        }
    }
    fun forceSyncFrame() {
        codec.setParameters(Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) })
    }
    fun drain(send: (ByteArray, Long) -> Boolean): Boolean = drain(10_000,send)
    fun drain(waitUs: Long, send: (ByteArray, Long) -> Boolean): Boolean {
        var accepted = true
        repeat(8) { attempt ->
            val index = codec.dequeueOutputBuffer(info, if (attempt == 0) waitUs else 0)
            if (index == MediaCodec.INFO_TRY_AGAIN_LATER) return accepted
            if (index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                val format = codec.outputFormat
                format.getByteBuffer("csd-0")?.duplicate()?.let { bytes ->
                    header = ByteArray(bytes.remaining()).also { bytes.get(it) }
                }
                // One record per codec generation, containing codec metadata only.
                // The actual SPS lets native receivers diagnose unsupported color
                // without inventing a matrix from the picture dimensions.
                android.util.Log.i("WeaveloMedia", "HEVC output $format csd=" +
                    header.take(1024).joinToString("") { "%02x".format(it.toInt() and 255) })
            } else if (index >= 0) {
                try {
                    if (info.size > 0) {
                        val buffer = checkNotNull(codec.getOutputBuffer(index)).duplicate()
                        buffer.position(info.offset); buffer.limit(info.offset + info.size)
                        val bytes = ByteArray(info.size); buffer.get(bytes)
                        if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0) header = bytes
                        else {
                            val payload = if (info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0) header + bytes else bytes
                            accepted = send(payload, info.presentationTimeUs) && accepted
                        }
                    }
                    if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) ended = true
                } finally { codec.releaseOutputBuffer(index, false) }
            }
        }
        return accepted
    }
    fun finish(send: (ByteArray, Long) -> Boolean): Boolean {
        codec.signalEndOfInputStream()
        val deadline=android.os.SystemClock.elapsedRealtimeNanos()+2_000_000_000
        var accepted=true
        while (!ended && android.os.SystemClock.elapsedRealtimeNanos()<deadline) accepted=drain(send=send)&&accepted
        check(ended) { "Hardware screen encoder did not finish its reference frames" }
        return accepted
    }
    override fun close() {
        runCatching { codec.stop() }
        try { codec.release() } finally { surface.release() }
    }
}
