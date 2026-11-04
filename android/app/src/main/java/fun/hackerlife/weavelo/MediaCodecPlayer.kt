package `fun`.hackerlife.weavelo

import android.media.MediaCodec
import android.media.MediaFormat
import android.media.MediaCodecList
import android.media.MediaCodecInfo
import android.os.Build
import android.util.Log
import android.view.Surface

/**
 * Decodes the negotiated stream directly to a Surface. Device latency is measured at runtime.
 */
internal class MediaCodecPlayer(
    private val surface: Surface,
    private val frameTarget: GpuVideoSurface? = null,
    private val onVideoSizeChanged: (VideoDimensions) -> Unit = {}
) {
    companion object {
        private val screenDecoder by lazy {
            MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.firstOrNull { info ->
                !info.isEncoder && info.supportedTypes.any { it.equals(MediaFormat.MIMETYPE_VIDEO_HEVC,true) } &&
                    (if (Build.VERSION.SDK_INT>=29) info.isHardwareAccelerated else !info.name.startsWith("OMX.google.") && !info.name.startsWith("c2.android."))
            }
        }
        fun screenSize(sourceWidth:Int,sourceHeight:Int):VideoDimensions? {
            val caps=screenDecoder?.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_HEVC)?.videoCapabilities ?: return null
            var width=minOf(4096,caps.supportedWidths.upper);var height=minOf(4096,caps.supportedHeights.upper)
            repeat(16) {
                val size=VideoDimensions.fitEncoder(sourceWidth,sourceHeight,width,height,caps.widthAlignment,caps.heightAlignment) ?: return null
                if (caps.isSizeSupported(size.width,size.height)) return size
                width=width*7/8;height=height*7/8
            }
            return null
        }
    }
    private var decoder: MediaCodec? = null
    private var decodedFrames = 0L
    val outputFrames: Long get() = frameTarget?.outputFrames ?: decodedFrames
    private var lastInputPts = Long.MIN_VALUE
    @Synchronized
    fun start(width: Int = 1920, height: Int = 1080, mimeType: String = MediaFormat.MIMETYPE_VIDEO_HEVC) {
        stop()
        decodedFrames = 0
        frameTarget?.reset(resetCount = true)
        require(width > 0 && height > 0 && surface.isValid) { "Invalid video surface or dimensions" }
        var candidate: MediaCodec? = null
        try {
            val codec = if (frameTarget != null) MediaCodec.createByCodecName(checkNotNull(screenDecoder) { "No native HEVC screen decoder" }.name)
                else MediaCodec.createDecoderByType(mimeType)
            candidate = codec
            val format = MediaFormat.createVideoFormat(mimeType, width, height).apply {
                if (frameTarget != null && codec.codecInfo.getCapabilitiesForType(mimeType)
                        .isFeatureSupported(MediaCodecInfo.CodecCapabilities.FEATURE_AdaptivePlayback)) {
                    setInteger(MediaFormat.KEY_MAX_WIDTH,width);setInteger(MediaFormat.KEY_MAX_HEIGHT,height)
                }
                if (Build.VERSION.SDK_INT >= 30 && codec.codecInfo
                        .getCapabilitiesForType(mimeType).isFeatureSupported("low-latency")) {
                    setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
                }
                setInteger(MediaFormat.KEY_PRIORITY, 0) // Realtime priority
            }
            codec.configure(format, surface, null, 0)
            codec.start()
            decoder = codec
        } catch (e: Exception) {
            runCatching { candidate?.release() }
                .onFailure { Log.w("WeaveloVideo", "Decoder cleanup failed", it) }
            // Changing the decoder alone cannot turn an incoming HEVC stream into AVC.
            throw IllegalStateException("Cannot decode $mimeType on this device", e)
        }
    }

    /**
     * 将从 Rust 提取出的 NALU 数据块送入解码器
     */
    @Synchronized
    fun feedNalu(data: ByteArray, isKeyframe: Boolean, ptsUs: Long, dataOffset: Int = 0): Boolean {
        require(dataOffset in 0 until data.size) { "Invalid video payload offset" }
        val (geometry, payloadOffset) = VideoFrameGeometry.accessUnit(data,dataOffset)
        val payloadSize = data.size - payloadOffset
        require(frameTarget != null || geometry == null || geometry.x == 0 && geometry.y == 0 &&
            geometry.cropWidth == geometry.sourceWidth && geometry.cropHeight == geometry.sourceHeight) { "Viewport requires its native frame presenter" }
        val codec = decoder ?: return false
        drain()
        val inputIndex = codec.dequeueInputBuffer(0)
        if (inputIndex < 0) return false
        val input = checkNotNull(codec.getInputBuffer(inputIndex)) { "Missing decoder input buffer" }
        check(payloadSize <= input.capacity()) { "Video frame exceeds decoder input capacity" }
        input.clear()
        input.put(data, payloadOffset, payloadSize)
        // An untimed source may repeat zero. Give every decoder input a unique
        // media timestamp, then match the actual latched SurfaceTexture image.
        val queuedPts = maxOf(ptsUs,lastInputPts + 1)
        frameTarget?.register(queuedPts,geometry)
        codec.queueInputBuffer(inputIndex, 0, payloadSize, queuedPts,
            if (isKeyframe) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0)
        lastInputPts = queuedPts
        drain()
        return true
    }

    @Synchronized
    fun drain() {
        val codec = decoder ?: return
        val info = MediaCodec.BufferInfo()
        try { repeat(16) {
            val index = codec.dequeueOutputBuffer(info, 0)
            when {
                index >= 0 -> {
                    val render = surface.isValid && info.size > 0
                    codec.releaseOutputBuffer(index, render)
                    if (render) decodedFrames++
                }
                index == MediaCodec.INFO_TRY_AGAIN_LATER -> return
                index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                    val format = codec.outputFormat
                    val width = format.getInteger(MediaFormat.KEY_WIDTH)
                    val height = format.getInteger(MediaFormat.KEY_HEIGHT)
                    fun dimension(key: String, fallback: Int) =
                        if (format.containsKey(key)) format.getInteger(key) else fallback
                    VideoDimensions.fromDecodedFrame(width, height,
                        dimension("crop-left", 0), dimension("crop-top", 0),
                        dimension("crop-right", width - 1), dimension("crop-bottom", height - 1))
                        ?.let(onVideoSizeChanged)
                }
            }
        } } finally { frameTarget?.renderPending() }
    }

    @Synchronized
    fun flush() {
        decoder?.flush()
        frameTarget?.reset()
    }

    @Synchronized
    fun stop() {
        val codec = decoder
        decoder = null
        runCatching { codec?.stop() }.onFailure { Log.w("WeaveloVideo", "Decoder stop failed", it) }
        runCatching { codec?.release() }.onFailure { Log.w("WeaveloVideo", "Decoder release failed", it) }
    }
}
