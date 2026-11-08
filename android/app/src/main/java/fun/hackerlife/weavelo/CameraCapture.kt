package `fun`.hackerlife.weavelo

import android.annotation.SuppressLint
import android.graphics.ColorSpace
import android.graphics.ImageFormat
import android.hardware.camera2.*
import android.hardware.camera2.params.OutputConfiguration
import android.hardware.camera2.params.DynamicRangeProfiles
import android.hardware.camera2.params.SessionConfiguration
import android.os.Build
import android.os.Handler
import android.util.Range
import android.util.Size
import org.json.JSONObject
import java.util.concurrent.CompletableFuture
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlin.math.abs

internal data class CameraDescription(val id: String, val name: String, val facing: String,
    val sensor: Int, val sizes: List<Size>, val rates: List<Range<Int>>, val manager: CameraManager) {
    fun rotation(displayDegrees: Int): Int = (sensor + (if (facing == "Front") displayDegrees else -displayDegrees) + 360) % 360
    fun json(displayDegrees: Int): JSONObject {
        val largest = sizes.maxByOrNull { it.width.toLong() * it.height } ?: appError("Camera has no video modes")
        return JSONObject().put("key", id).put("name", name).put("facing", facing)
            .put("switch_group", if (facing == "Front" || facing == "Back") "phone-cameras" else JSONObject.NULL)
            .put("width", largest.width).put("height", largest.height).put("rotation", rotation(displayDegrees))
    }
    data class Configuration(val encoder: SurfaceHevcEncoder.Plan, val range: Range<Int>)
    fun configure(demand: JSONObject): Configuration {
        val native = sizes.maxByOrNull { it.width.toLong() * it.height } ?: appError("Camera has no video modes")
        val targetWidth = demand.getInt("width")
        val targetHeight = demand.getInt("height")
        val fit = VideoDimensions.fitEncoder(native.width, native.height, targetWidth, targetHeight, 2, 2)
            ?: appError("Camera viewport is too small")
        val wanted = fit.width.toLong() * fit.height
        val ordered = sizes.sortedWith(compareBy<Size> {
            abs(it.width.toDouble() / it.height - native.width.toDouble() / native.height)
        }.thenBy { if (it.width.toLong() * it.height >= wanted) 0 else 1 }
            .thenBy { abs(it.width.toLong() * it.height - wanted) })
        val fps = demand.getInt("fps")
        val characteristics = manager.getCameraCharacteristics(id)
        val map = characteristics.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
            ?: appError("Camera video formats are unavailable")
        val srgb = Build.VERSION.SDK_INT >= 34 && characteristics
            .get(CameraCharacteristics.REQUEST_AVAILABLE_COLOR_SPACE_PROFILES)
            ?.getSupportedColorSpacesForDynamicRange(ImageFormat.PRIVATE, DynamicRangeProfiles.STANDARD)
            ?.contains(ColorSpace.Named.SRGB) == true
        for (size in ordered) {
            val duration = map.getOutputMinFrameDuration(android.media.MediaCodec::class.java, size)
            val maximum = if (duration > 0) (1_000_000_000L / duration).toInt().coerceAtLeast(1) else fps
            val requested = minOf(fps, maximum)
            val rate = rates.filter { it.upper <= requested }.maxWithOrNull(
                compareBy<Range<Int>> { if (it.upper <= minOf(requested, 30)) 1 else 0 }
                    .thenBy { minOf(it.upper, 30) }
                    .thenByDescending { it.lower }
                    .thenBy { it.upper }) ?: continue
            val actual = rate.upper
            val encoder = SurfaceHevcEncoder.exact(size.width, size.height, actual, demand.getInt("bitrate_kbps")) ?: continue
            return Configuration(encoder.copy(srgb = srgb), rate)
        }
        appError("No native camera mode and hardware HEVC encoder support this stream")
    }
    companion object {
        fun enumerate(manager: CameraManager): List<CameraDescription> {
            val ids = manager.cameraIdList.toList()
            val physical = if (Build.VERSION.SDK_INT >= 28) ids.flatMap { id ->
                val info = manager.getCameraCharacteristics(id)
                if (info.get(CameraCharacteristics.REQUEST_AVAILABLE_CAPABILITIES)?.contains(CameraCharacteristics.REQUEST_AVAILABLE_CAPABILITIES_LOGICAL_MULTI_CAMERA) == true)
                    info.physicalCameraIds else emptySet()
            }.toSet() else emptySet()
            return ids.filterNot { it in physical }.mapNotNull { id ->
                val info = manager.getCameraCharacteristics(id)
                val facing = when (info.get(CameraCharacteristics.LENS_FACING)) {
                    CameraCharacteristics.LENS_FACING_FRONT -> "Front"
                    CameraCharacteristics.LENS_FACING_BACK -> "Back"
                    CameraCharacteristics.LENS_FACING_EXTERNAL -> "External"
                    else -> "Unspecified"
                }
                val map = info.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP) ?: return@mapNotNull null
                val sizes = map.getOutputSizes(android.media.MediaCodec::class.java)?.filter {
                    it.width in 2..4096 && it.height in 2..4096 && it.width.toLong() * it.height <= 8_388_608
                }.orEmpty()
                if (sizes.isEmpty()) return@mapNotNull null
                CameraDescription(id, "${facing.replace("Unspecified", "Device")} camera", facing,
                    info.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 0, sizes,
                    info.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES)?.toList().orEmpty(), manager)
            }
        }
    }
}

/** One real camera owner. Closing is acknowledged by CameraDevice.onClosed, not by close() returning. */
internal class CameraCapture(private val description: CameraDescription, val configuration: CameraDescription.Configuration,
    private val handler: Handler) : AutoCloseable {
    companion object { private val owner = AtomicReference<CameraCapture?>(null) }
    private val encoder = SurfaceHevcEncoder(configuration.encoder)
    private val codecLock = Any()
    private val device = AtomicReference<CameraDevice?>(null)
    private val session = AtomicReference<CameraCaptureSession?>(null)
    private val closing = AtomicBoolean(false)
    private val released = AtomicBoolean(false)
    private val requested = AtomicBoolean(false)
    private val opened = CompletableFuture<CameraDevice>()
    private val configured = CompletableFuture<CameraCaptureSession>()
    val closed = CompletableFuture<Unit>()
    val failure = AtomicReference<Throwable?>(null)

    private fun release() {
        if (released.compareAndSet(false, true)) {
            val result = runCatching { synchronized(codecLock) { encoder.close() } }
            result.fold(onSuccess = { owner.compareAndSet(this, null); closed.complete(Unit) },
                onFailure = { failure.set(it); closed.completeExceptionally(it) })
        }
    }
    @SuppressLint("MissingPermission")
    fun start() {
        if (!owner.compareAndSet(null, this)) { release(); appError("The previous camera is still closing") }
        try {
            requested.set(true)
            description.manager.openCamera(description.id, object : CameraDevice.StateCallback() {
                override fun onOpened(camera: CameraDevice) {
                    device.set(camera)
                    if (closing.get()) { camera.close(); opened.completeExceptionally(AppTextException(AppText("Camera start cancelled"))) }
                    else opened.complete(camera)
                }
                override fun onDisconnected(camera: CameraDevice) { failed(camera, AppText("Camera disconnected")) }
                override fun onError(camera: CameraDevice, error: Int) { failed(camera, AppText("Camera unavailable (%d)", listOf(error))) }
                private fun failed(camera: CameraDevice, text: AppText) {
                    device.set(camera); val error = AppTextException(text)
                    if (!closing.get()) failure.set(error)
                    opened.completeExceptionally(error); configured.completeExceptionally(error); camera.close()
                }
                override fun onClosed(camera: CameraDevice) { release() }
            }, handler)
            val camera = opened.get(5, TimeUnit.SECONDS)
            val callback = object : CameraCaptureSession.StateCallback() {
                override fun onConfigured(value: CameraCaptureSession) {
                    session.set(value)
                    if (closing.get()) { value.close(); configured.completeExceptionally(AppTextException(AppText("Camera start cancelled"))) }
                    else configured.complete(value)
                }
                override fun onConfigureFailed(value: CameraCaptureSession) {
                    val error = AppTextException(AppText("Camera capture session could not be configured"))
                    failure.set(error); configured.completeExceptionally(error)
                }
            }
            if (Build.VERSION.SDK_INT >= 28) {
                val output = OutputConfiguration(encoder.surface)
                if (Build.VERSION.SDK_INT >= 33) output.setMirrorMode(OutputConfiguration.MIRROR_MODE_NONE)
                val session = SessionConfiguration(SessionConfiguration.SESSION_REGULAR,
                    listOf(output), { task -> handler.post(task) }, callback)
                if (Build.VERSION.SDK_INT >= 34 && configuration.encoder.srgb) {
                    // The same hardware-verified color choice configures both
                    // the camera pixels and encoder; neither infers the other.
                    session.setColorSpace(ColorSpace.Named.SRGB)
                }
                camera.createCaptureSession(session)
            } else @Suppress("DEPRECATION") camera.createCaptureSession(listOf(encoder.surface), callback, handler)
            val ready = configured.get(5, TimeUnit.SECONDS)
            appCheck(!closing.get(), "Camera start cancelled")
            val capture = camera.createCaptureRequest(CameraDevice.TEMPLATE_RECORD).apply {
                addTarget(encoder.surface)
                set(CaptureRequest.CONTROL_MODE, CaptureRequest.CONTROL_MODE_AUTO)
                set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON)
                set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, configuration.range)
                val focus = description.manager.getCameraCharacteristics(description.id).get(CameraCharacteristics.CONTROL_AF_AVAILABLE_MODES)
                if (focus?.contains(CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO) == true)
                    set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO)
            }
            ready.setRepeatingRequest(capture.build(), null, handler)
        } catch (error: Throwable) {
            if (device.get() == null && !opened.isDone) {
                // A synchronous open failure has no pending SDK owner; a timeout may still deliver onOpened.
                if (error !is java.util.concurrent.TimeoutException) { requested.set(false); release() }
            }
            requestClose(); throw error
        }
    }
    fun drain(keyframe: Long, send: (ByteArray, Long) -> Boolean): Boolean = synchronized(codecLock) {
        if (closing.get()) return@synchronized false
        encoder.requestKeyframe(keyframe); encoder.drain(send)
    }
    fun requestClose() {
        closing.set(true)
        runCatching { session.get()?.close() }; runCatching { device.get()?.close() }
        if (!requested.get()) release()
    }
    override fun close() { requestClose(); closed.get(5, TimeUnit.SECONDS) }
}
