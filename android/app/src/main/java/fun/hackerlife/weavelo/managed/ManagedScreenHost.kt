package `fun`.hackerlife.weavelo.managed

import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.os.Binder
import android.os.Handler
import android.os.HandlerThread
import android.util.DisplayMetrics
import android.view.Display
import android.view.Surface
import org.json.JSONObject
import java.util.concurrent.FutureTask
import java.util.concurrent.TimeUnit

/** Runs only inside Weavelo's already authorized shell process. */
internal class ManagedScreenHost(private val appUid: Int) : IManagedScreen.Stub() {
    private val thread = HandlerThread("weavelo-display").apply { start() }
    private val handler = Handler(thread.looper)
    private val context by lazy { ManagedShellContext.create() }
    private val manager by lazy {
        DisplayManager::class.java.getDeclaredConstructor(android.content.Context::class.java)
            .apply { isAccessible = true }.newInstance(context)
    }
    private var mirror: VirtualDisplay? = null
    @Volatile private var failed = false

    private fun <T> call(block: () -> T): T {
        check(Binder.getCallingUid() == appUid) { "Weavelo UID required" }
        check(!failed) { "Display operation timed out; reconnect the helper" }
        val task = FutureTask { check(!failed); block() }
        check(handler.post(task))
        return try { task.get(2, TimeUnit.SECONDS) } catch (error: java.util.concurrent.TimeoutException) {
            // A timed-out creation may already have entered the OS. Serialize
            // cleanup after it and reject new work, rather than orphan a display.
            failed = true
            handler.post { clear() }
            throw IllegalStateException("Display operation timed out", error)
        } catch (error: java.util.concurrent.ExecutionException) {
            throw IllegalStateException(error.cause?.message ?: "Display operation failed", error.cause)
        }
    }
    private fun metrics(): DisplayMetrics = DisplayMetrics().also {
        @Suppress("DEPRECATION")
        checkNotNull(manager.getDisplay(Display.DEFAULT_DISPLAY)).getRealMetrics(it)
    }
    override fun info(): String = call {
        val size = metrics()
        JSONObject().put("api", 1).put("width", size.widthPixels).put("height", size.heightPixels)
            .put("density", size.densityDpi).put("pid", android.os.Process.myPid())
            .put("version", `fun`.hackerlife.weavelo.BuildConfig.VERSION_CODE).toString()
    }
    private fun validate(width: Int, height: Int, density: Int) {
        val actual = metrics()
        require(width == actual.widthPixels && height == actual.heightPixels && density == actual.densityDpi) {
            "Physical display changed; refresh its geometry"
        }
        require(width in 2..8192 && height in 2..8192 && density in 72..1000)
    }
    override fun createMirror(width: Int, height: Int, density: Int): Int = call {
        validate(width, height, density)
        check(mirror == null) { "Screen capture already owned" }
        val display = checkNotNull(manager.createVirtualDisplay("Weavelo screen", width, height, density,
            null, DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR)) { "Screen mirror unavailable" }
        mirror = display
        display.display.displayId
    }
    private fun owned(id: Int): VirtualDisplay = checkNotNull(mirror?.takeIf { it.display.displayId == id }) {
        "Screen capture is no longer owned"
    }
    override fun setSurface(id: Int, surface: Surface?) = call {
        val display = owned(id)
        val old = display.surface
        try { display.surface = surface } catch (error: Exception) { surface?.release(); throw error }
        old?.release()
        Unit
    }
    override fun resize(id: Int, width: Int, height: Int, density: Int) = call {
        validate(width, height, density)
        owned(id).resize(width, height, density)
    }
    override fun release(id: Int) = call { owned(id); clear() }
    private fun clear() {
        val previous = mirror; mirror = null
        val surface = previous?.surface
        try { previous?.release() } finally { surface?.release() }
    }
    fun clientDied() { handler.post { try { clear() } finally { kotlin.system.exitProcess(0) } } }
}
