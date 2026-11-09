package `fun`.hackerlife.weavelo

import android.hardware.DataSpace
import org.json.JSONArray
import org.json.JSONObject
import java.util.TreeMap

/** One authorized raw capture texture, independent native encoders per view. */
internal class GpuScreenPublisher(private val width: Int, private val height: Int, rotation: Int = 0) : AutoCloseable {
    private val gpu=GpuVideoSurface(null,width,height,PublisherClient::nativeClockMicros,sourceRotation=rotation)
    val input get()=gpu.input
    private val views=mutableMapOf<Long,View>()
    private val failed=mutableSetOf<Long>()
    private var nextPts=0L
    private var lastDataSpace: Int?=null

    internal data class Demand(val stream: Long,val grant: Long,val keyframe: Long,val fps: Int,
        val bitrateKbps: Int,val geometry: VideoFrameGeometry,val deliveryId: Long,val qualityRevision: Long,
        val qualityPending: Boolean) {
        companion object {
            fun parse(value: JSONObject)=Demand(value.getLong("stream"),value.getLong("grant"),value.getLong("keyframe"),
                value.getInt("fps"),value.getInt("bitrate_kbps"),VideoFrameGeometry.fromJson(value.getJSONObject("geometry")),
                value.optLong("delivery_id"),value.optLong("quality_revision"),value.optBoolean("quality_pending")).also {
                require(it.stream>0 && it.grant>0 && it.fps in 1..120 && it.bitrateKbps>0)
            }
        }
    }
    private data class Stamp(val stream: Long,val grant: Long,val geometry: VideoFrameGeometry,
        val captureUs: Long,val admittedUs: Long,val qualityRevision: Long) {
        fun json()=JSONObject().put("stream",stream).put("grant",grant).put("geometry",geometry.toJson())
            .put("capture_ts_us",captureUs).put("admitted_at_us",admittedUs).toString()
    }
    private data class EncoderOutput(val codec: SurfaceHevcEncoder,val surface: GpuVideoSurface.Output)
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
        private var rejectedPlan: SurfaceHevcEncoder.Plan?=null
        private var reportedDelivery=0L
        private var acknowledgedRevision=0L
        private var acknowledgedQuality=false
        private fun acknowledgeQuality(accepted: Boolean): Boolean {
            if (!demand.qualityPending) return true
            if (demand.deliveryId<=0 || demand.qualityRevision<=0) return false
            if (acknowledgedRevision!=demand.qualityRevision) {
                acknowledgedQuality=PublisherClient.action(38,JSONObject().put("id",demand.deliveryId).put("stream",demand.stream)
                    .put("grant",demand.grant).put("revision",demand.qualityRevision).put("accepted",accepted).toString()) == "true" && accepted
                acknowledgedRevision=demand.qualityRevision
            }
            return acknowledgedQuality
        }

        private fun send(bytes: ByteArray,ptsUs: Long): Boolean {
            val stamp=checkNotNull(pending.remove(ptsUs)) { "Screen output has no matching input geometry" }
            val confirmsQuality=demand.qualityPending && stamp.qualityRevision==demand.qualityRevision
            if (confirmsQuality && acknowledgedRevision==stamp.qualityRevision && !acknowledgedQuality) return false
            val accepted=PublisherClient.nativeScreenFrame(stamp.json(),bytes)
            if (accepted && confirmsQuality) acknowledgeQuality(true)
            return accepted
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
            if (demand.qualityPending && acknowledgedRevision==demand.qualityRevision && !acknowledgedQuality) return false
            val requested="${g.width}:${g.height}:${demand.fps}:${demand.bitrateKbps}:$dataSpace:${demand.qualityRevision}"
            if (selectionKey!=requested) {
                // The common planner deliberately stabilizes coded dimensions
                // while the crop aspect changes. GPU scaling preserves that;
                // receiver geometry restores the actual source aspect.
                selection=SurfaceHevcEncoder.fitAtMost(g.width,g.height,g.width,g.height,demand.fps,demand.bitrateKbps)
                    ?.copy(srgb=dataSpace==DataSpace.DATASPACE_SRGB)
                selectionKey=requested
                rejectedPlan=null
            }
            // Keep an already working encoder when the requested candidate is
            // unsupported. It must still fit the crop and the network cap.
            fun oldPlan()=codec?.plan?.takeIf { it.width<=g.width && it.height<=g.height &&
                it.width<=g.cropWidth && it.height<=g.cropHeight && it.fps<=demand.fps &&
                it.bitrateKbps<=demand.bitrateKbps && it.srgb==(dataSpace==DataSpace.DATASPACE_SRGB) }
            if (selection==null && demand.qualityPending) {
                acknowledgeQuality(false)
                return false
            }
            var plan=selection ?: oldPlan() ?: error("No hardware HEVC encoder supports this screen view")
            var accepted=true
            if (codec?.plan!=plan) {
                val previous=codec?.plan
                val fallback=oldPlan()
                var candidate=if (plan==rejectedPlan) null else try { openEncoder(plan) } catch (error: Exception) {
                    rejectedPlan=plan
                    android.util.Log.w("WeaveloMedia","Requested screen encoder unavailable",error)
                    null
                }
                try {
                    if (candidate==null && previous!=null && (demand.qualityPending || fallback==null)) {
                        // Some codecs have one hardware slot. Drain its references before freeing it.
                        accepted=finish()
                        closeEncoder()
                        candidate=openEncoder(plan)
                    }
                    if (candidate!=null) {
                        accepted=finish() && accepted
                        closeEncoder()
                        installEncoder(candidate)
                    } else {
                        plan=fallback ?: error("No hardware HEVC encoder could be started")
                    }
                } catch (error: Exception) {
                    candidate?.let { runCatching { closeOutput(it) } }
                    if (!demand.qualityPending || previous==null) throw error
                    runCatching { closeEncoder() }
                    pending.clear()
                    acknowledgeQuality(false)
                    installEncoder(openEncoder(previous))
                    return false
                }
            }
            if (reportedDelivery!=demand.deliveryId) {
                OutgoingVideoRuntime.clear(reportedDelivery,demand.stream)
                reportedDelivery=demand.deliveryId
            }
            OutgoingVideoRuntime.report(OutgoingEncoderState(demand.deliveryId,demand.stream,plan.width,plan.height,
                minOf(plan.fps,demand.fps),plan.bitrateKbps))
            val encoder=checkNotNull(codec)
            accepted=encoder.drain(0,::send)&&accepted
            if (demand.qualityPending && acknowledgedRevision==demand.qualityRevision && !acknowledgedQuality) return false
            val now=PublisherClient.nativeClockMicros()
            val awaitingQualityFrame=demand.qualityPending && acknowledgedRevision!=demand.qualityRevision &&
                pending.values.none { it.qualityRevision==demand.qualityRevision }
            val redraw=geometryRevision!=g.revision || keyframe!=demand.keyframe || awaitingQualityFrame
            if (pending.size<3 && now-lastSubmitUs>=1_000_000/minOf(demand.fps,plan.fps) && (redraw || captureUs!=gpu.sourceCaptureUs)) {
                encoder.requestKeyframe(demand.keyframe)
                val geometry=g.copy(width=plan.width,height=plan.height)
                check(geometry.valid())
                nextPts=maxOf(nextPts+1,System.nanoTime()/1000)
                pending[nextPts]=Stamp(demand.stream,demand.grant,geometry,gpu.sourceCaptureUs,now,
                    if (demand.qualityPending) demand.qualityRevision else 0)
                gpu.drawSource(checkNotNull(output),geometry,nextPts)
                captureUs=gpu.sourceCaptureUs;geometryRevision=g.revision;keyframe=demand.keyframe;lastSubmitUs=now
            }
            return accepted
        }
        private fun openEncoder(plan: SurfaceHevcEncoder.Plan): EncoderOutput {
            val next=SurfaceHevcEncoder(plan)
            return try { EncoderOutput(next,gpu.addOutput(next.surface)) }
            catch (error: Exception) { next.close();throw error }
        }
        private fun installEncoder(next: EncoderOutput) {
            codec=next.codec;output=next.surface
            captureUs=0;geometryRevision=0;keyframe=-1;lastSubmitUs=0
        }
        private fun closeOutput(target: EncoderOutput) {
            try { gpu.removeOutput(target.surface) } finally { target.codec.close() }
        }
        private fun closeEncoder() {
            val target=output;val encoder=codec;output=null;codec=null
            try { target?.let(gpu::removeOutput) } finally { encoder?.close() }
        }
        override fun close() { try { closeEncoder() } finally { pending.clear();OutgoingVideoRuntime.clear(reportedDelivery,demand.stream) } }
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
