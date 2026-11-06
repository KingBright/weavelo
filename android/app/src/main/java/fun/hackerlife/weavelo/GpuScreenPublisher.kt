package `fun`.hackerlife.weavelo

import android.hardware.DataSpace
import org.json.JSONArray
import org.json.JSONObject
import java.util.TreeMap

/** One authorized raw capture texture, independent native encoders per view. */
internal class GpuScreenPublisher(private val width: Int, private val height: Int) : AutoCloseable {
    private val gpu=GpuVideoSurface(null,width,height,PublisherClient::nativeClockMicros)
    val input get()=gpu.input
    private val views=mutableMapOf<Long,View>()
    private val failed=mutableSetOf<Long>()
    private var nextPts=0L
    private var lastDataSpace: Int?=null

    internal data class Demand(val stream: Long,val grant: Long,val keyframe: Long,val fps: Int,
        val bitrateKbps: Int,val geometry: VideoFrameGeometry) {
        companion object {
            fun parse(value: JSONObject)=Demand(value.getLong("stream"),value.getLong("grant"),value.getLong("keyframe"),
                value.getInt("fps"),value.getInt("bitrate_kbps"),VideoFrameGeometry.fromJson(value.getJSONObject("geometry"))).also {
                require(it.stream>0 && it.grant>0 && it.fps in 1..120 && it.bitrateKbps>0)
            }
        }
    }
    private data class Stamp(val stream: Long,val grant: Long,val geometry: VideoFrameGeometry,
        val captureUs: Long,val admittedUs: Long) {
        fun json()=JSONObject().put("stream",stream).put("grant",grant).put("geometry",geometry.toJson())
            .put("capture_ts_us",captureUs).put("admitted_at_us",admittedUs).toString()
    }
    private inner class View(var demand: Demand) : AutoCloseable {
        var codec: SurfaceHevcEncoder?=null
        var output: GpuVideoSurface.Output?=null
        val pending=TreeMap<Long,Stamp>()
        var captureUs=0L
        var geometryRevision=0L
        var keyframe=-1L
        var lastSubmitUs=0L
        private var selectionKey=""
        private var selection: SurfaceHevcEncoder.Plan?=null

        private fun send(bytes: ByteArray,ptsUs: Long): Boolean {
            val stamp=checkNotNull(pending.remove(ptsUs)) { "Screen output has no matching input geometry" }
            return PublisherClient.nativeScreenFrame(stamp.json(),bytes)
        }
        fun finish(): Boolean {
            if (pending.isEmpty()) return true
            val accepted=checkNotNull(codec).finish(::send)
            check(pending.isEmpty()) { "Screen encoder lost an admitted frame during resize" }
            return accepted
        }
        fun update(): Boolean {
            val g=demand.geometry
            if (g.sourceWidth!=width || g.sourceHeight!=height) return false
            // Keep producer metadata. Do not relabel unknown or wide-gamut RGB
            // as sRGB just because this is a screen texture.
            val dataSpace=gpu.sourceDataSpace
            require(dataSpace==0 || dataSpace==DataSpace.DATASPACE_SRGB) { "Unsupported screen color space: $dataSpace" }
            val requested="${g.width}:${g.height}:${demand.fps}:${demand.bitrateKbps}:$dataSpace"
            if (selectionKey!=requested) {
                // The common planner deliberately stabilizes coded dimensions
                // while the crop aspect changes. GPU scaling preserves that;
                // receiver geometry restores the actual source aspect.
                selection=SurfaceHevcEncoder.fit(g.width,g.height,g.width,g.height,demand.fps,demand.bitrateKbps)
                    ?.copy(srgb=dataSpace==DataSpace.DATASPACE_SRGB)
                selectionKey=requested
            }
            val plan=selection ?: error("No hardware HEVC encoder supports this screen view")
            var accepted=true
            if (codec?.plan!=plan) {
                if (pending.isNotEmpty()) {
                    accepted=finish()
                }
                closeEncoder()
                val encoder=SurfaceHevcEncoder(plan)
                codec=encoder;output=gpu.addOutput(encoder.surface)
                captureUs=0;geometryRevision=0;keyframe=-1;lastSubmitUs=0
            }
            val encoder=checkNotNull(codec)
            accepted=encoder.drain(0,::send)&&accepted
            val now=PublisherClient.nativeClockMicros()
            val redraw=geometryRevision!=g.revision || keyframe!=demand.keyframe
            if (pending.size<3 && now-lastSubmitUs>=1_000_000/demand.fps && (redraw || captureUs!=gpu.sourceCaptureUs)) {
                encoder.requestKeyframe(demand.keyframe)
                val geometry=g.copy(width=plan.width,height=plan.height)
                check(geometry.valid())
                nextPts=maxOf(nextPts+1,System.nanoTime()/1000)
                pending[nextPts]=Stamp(demand.stream,demand.grant,geometry,gpu.sourceCaptureUs,now)
                gpu.drawSource(checkNotNull(output),geometry,nextPts)
                captureUs=gpu.sourceCaptureUs;geometryRevision=g.revision;keyframe=demand.keyframe;lastSubmitUs=now
            }
            return accepted
        }
        private fun closeEncoder() {
            val target=output;val encoder=codec;output=null;codec=null
            try { target?.let(gpu::removeOutput) } finally { encoder?.close() }
        }
        override fun close() { try { closeEncoder() } finally { pending.clear() } }
    }

    fun update(demands: JSONArray): Boolean {
        require(demands.length()<=64) { "Too many screen views" }
        val wanted=(0 until demands.length()).map { Demand.parse(demands.getJSONObject(it)) }.associateBy { it.stream }
        require(wanted.size==demands.length()) { "Duplicate screen view identity" }
        failed.retainAll(wanted.keys)
        for (stream in views.keys.toList()) {
            val view=views.getValue(stream)
            if (wanted[stream]?.grant!=view.demand.grant) { views.remove(stream);runCatching { view.close() } }
        }
        gpu.latchSource()
        if (!gpu.sourceReady) return true
        if (lastDataSpace!=gpu.sourceDataSpace) {
            lastDataSpace=gpu.sourceDataSpace
            android.util.Log.i("WeaveloMedia","Screen GPU ingress ${width}x$height dataSpace=$lastDataSpace")
        }
        var accepted=true
        for ((stream,demand) in wanted) {
            if (stream in failed) continue
            val view=views.getOrPut(stream) { View(demand) };view.demand=demand
            try { accepted=view.update()&&accepted }
            catch (error: Exception) {
                failView(stream,view,error)
            }
        }
        return accepted
    }
    fun awaitFrame(timeoutMs: Long)=gpu.awaitSource(timeoutMs)
    private fun failView(stream: Long,view: View,error: Exception) {
        views.remove(stream);failed.add(stream);runCatching { view.close() }
        PublisherClient.action(36,JSONObject().put("stream",stream).put("grant",view.demand.grant)
            .put("reason",(error.message ?: "Hardware screen view failed").take(200)).toString())
    }
    fun finish() {
        for ((stream,view) in views.toList()) try { view.finish() } catch (error: Exception) { failView(stream,view,error) }
    }
    val pendingFrames get()=views.values.any { it.pending.isNotEmpty() }
    override fun close() {
        try { for (view in views.values) runCatching { view.close() };views.clear() }
        finally { gpu.close() }
    }
}
