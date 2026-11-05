package `fun`.hackerlife.weavelo.managed

import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.app.ActivityManager
import android.app.ActivityOptions
import android.content.Intent
import android.os.Binder
import android.os.Handler
import android.os.HandlerThread
import android.util.DisplayMetrics
import android.view.Display
import android.view.Surface
import android.view.MotionEvent
import android.view.KeyEvent
import android.view.InputEvent
import android.view.InputDevice
import org.json.JSONArray
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
    private val applications = linkedMapOf<Int, VirtualDisplay>()
    private val applicationSizes = mutableMapOf<Int, Pair<Int,Int>>()
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
        JSONObject().put("api", 2).put("width", size.widthPixels).put("height", size.heightPixels)
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
    private fun launchers() = context.packageManager.queryIntentActivities(
        Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER), 0)
        .filter { it.activityInfo.exported && it.activityInfo.enabled && it.activityInfo.applicationInfo.enabled }
        .distinctBy { it.activityInfo.packageName }

    override fun applications(): String = call {
        JSONArray().apply {
            for (entry in launchers()) {
                val info = entry.activityInfo
                if (info.packageName == `fun`.hackerlife.weavelo.BuildConfig.APPLICATION_ID) continue
                put(JSONObject().put("package", info.packageName)
                    .put("name", entry.loadLabel(context.packageManager).toString().take(200)))
            }
        }.toString()
    }
    @android.annotation.SuppressLint("WrongConstant") // AOSP's documented hidden display flags above API 34.
    override fun createApplication(packageName: String, width: Int, height: Int, density: Int): Int = call {
        require(packageName.length in 1..255)
        require(width in 2..8192 && height in 2..8192 && width.toLong()*height <= 33_554_432 && density in 72..1000)
        check(applications.size < 8) { "Eight independent apps are already open" }
        val entry = checkNotNull(launchers().find { it.activityInfo.packageName == packageName }) { "App is not launchable" }
        require(packageName != `fun`.hackerlife.weavelo.BuildConfig.APPLICATION_ID) { "Cannot launch the sharing app inside itself" }
        // A single-task app may reuse and move an existing activity despite
        // MULTIPLE_TASK. Do not steal the user's main-screen task.
        @Suppress("DEPRECATION")
        val tasks = context.getSystemService(ActivityManager::class.java).getRunningTasks(256)
        check(tasks.none { it.baseActivity?.packageName == packageName || it.topActivity?.packageName == packageName }) {
            "App is already open; close its existing task before opening an independent app"
        }
        // AOSP Android 14 flags: own content, touch, destroy owned content on
        // removal, trusted display, independent focus, never steal top focus.
        // Do not set ALWAYS_UNLOCKED, SECURE or keyguard-bypass flags.
        val flags = DisplayManager.VIRTUAL_DISPLAY_FLAG_OWN_CONTENT_ONLY or
            (1 shl 6) or (1 shl 8) or (1 shl 10) or (1 shl 14) or (1 shl 16)
        val display = checkNotNull(manager.createVirtualDisplay("Weavelo · ${entry.loadLabel(context.packageManager)}",
            width, height, density, null, flags)) { "Independent display unavailable" }
        val id = display.display.displayId
        applications[id] = display
        applicationSizes[id] = width to height
        try {
            val intent = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
                .setClassName(packageName, entry.activityInfo.name)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_MULTIPLE_TASK)
            context.startActivity(intent, ActivityOptions.makeBasic().setLaunchDisplayId(id).toBundle())
            id
        } catch (error: Exception) { releaseOwned(id); throw error }
    }
    private fun owned(id: Int): VirtualDisplay = checkNotNull(applications[id] ?: mirror?.takeIf { it.display.displayId == id }) {
        "Screen capture is no longer owned"
    }
    private val inputManager by lazy {
        Class.forName("android.hardware.input.InputManager").getMethod("getInstance").invoke(null)
    }
    private fun inject(id: Int, event: InputEvent): Boolean {
        check(applications.containsKey(id)) { "Input requires an owned independent display" }
        val size=DisplayMetrics()
        @Suppress("DEPRECATION")
        owned(id).display.getRealMetrics(size)
        check(applicationSizes[id] == (size.widthPixels to size.heightPixels)) {
            "Independent display geometry changed; reopen this app before controlling it"
        }
        InputEvent::class.java.getMethod("setDisplayId", Int::class.javaPrimitiveType).invoke(event, id)
        return inputManager.javaClass.getMethod("injectInputEvent", InputEvent::class.java, Int::class.javaPrimitiveType)
            .invoke(inputManager, event, 1) as Boolean
    }
    override fun injectMotion(id: Int, event: MotionEvent): Boolean = call {
        try {
            val size = DisplayMetrics()
            @Suppress("DEPRECATION")
            owned(id).display.getRealMetrics(size)
            require(event.pointerCount in 1..10 && event.source == InputDevice.SOURCE_TOUCHSCREEN)
            require(event.actionMasked in listOf(MotionEvent.ACTION_DOWN, MotionEvent.ACTION_UP, MotionEvent.ACTION_MOVE,
                MotionEvent.ACTION_CANCEL, MotionEvent.ACTION_POINTER_DOWN, MotionEvent.ACTION_POINTER_UP))
            for (pointer in 0 until event.pointerCount) {
                require(event.getX(pointer).isFinite() && event.getY(pointer).isFinite() &&
                    event.getX(pointer) in 0f..(size.widthPixels-1).toFloat() && event.getY(pointer) in 0f..(size.heightPixels-1).toFloat())
            }
            inject(id, event)
        } finally { event.recycle() }
    }
    override fun injectKey(id: Int, event: KeyEvent): Boolean = call {
        require(event.action == KeyEvent.ACTION_DOWN || event.action == KeyEvent.ACTION_UP)
        require(event.keyCode in KeyEvent.KEYCODE_A..KeyEvent.KEYCODE_Z ||
            event.keyCode in KeyEvent.KEYCODE_0..KeyEvent.KEYCODE_9 ||
            event.keyCode in KeyEvent.KEYCODE_F1..KeyEvent.KEYCODE_F12 ||
            event.keyCode in KeyEvent.KEYCODE_NUMPAD_0..KeyEvent.KEYCODE_NUMPAD_EQUALS ||
            event.keyCode in listOf(KeyEvent.KEYCODE_BACK,KeyEvent.KEYCODE_ESCAPE,KeyEvent.KEYCODE_ENTER,
                KeyEvent.KEYCODE_TAB,KeyEvent.KEYCODE_SPACE,KeyEvent.KEYCODE_DEL,KeyEvent.KEYCODE_FORWARD_DEL,
                KeyEvent.KEYCODE_DPAD_LEFT,KeyEvent.KEYCODE_DPAD_RIGHT,KeyEvent.KEYCODE_DPAD_UP,KeyEvent.KEYCODE_DPAD_DOWN,
                KeyEvent.KEYCODE_MOVE_HOME,KeyEvent.KEYCODE_MOVE_END,KeyEvent.KEYCODE_PAGE_UP,KeyEvent.KEYCODE_PAGE_DOWN,
                KeyEvent.KEYCODE_MINUS,KeyEvent.KEYCODE_EQUALS,KeyEvent.KEYCODE_LEFT_BRACKET,KeyEvent.KEYCODE_RIGHT_BRACKET,
                KeyEvent.KEYCODE_BACKSLASH,KeyEvent.KEYCODE_SEMICOLON,KeyEvent.KEYCODE_APOSTROPHE,
                KeyEvent.KEYCODE_COMMA,KeyEvent.KEYCODE_PERIOD,KeyEvent.KEYCODE_SLASH,KeyEvent.KEYCODE_GRAVE)) {
            "Global device keys are not independent-app input"
        }
        inject(id, event)
    }
    override fun setSurface(id: Int, surface: Surface?) = call {
        val display = owned(id)
        val old = display.surface
        try { display.surface = surface } catch (error: Exception) { surface?.release(); throw error }
        old?.release()
        Unit
    }
    override fun resize(id: Int, width: Int, height: Int, density: Int) = call {
        // App geometry is fixed for its lifetime until per-display rotation and
        // resize revisions are wired into capture and input together.
        check(!applications.containsKey(id)) { "Independent display geometry is fixed" }
        validate(width, height, density)
        owned(id).resize(width, height, density)
    }
    override fun release(id: Int) = call { owned(id); releaseOwned(id) }
    private fun releaseOwned(id: Int) {
        applicationSizes.remove(id)
        val display = applications.remove(id) ?: mirror?.takeIf { it.display.displayId == id }?.also { mirror = null }
        val surface = display?.surface
        try { display?.release() } finally { surface?.release() }
    }
    private fun clear() {
        val ids = applications.keys.toList() + listOfNotNull(mirror?.display?.displayId)
        ids.forEach { runCatching { releaseOwned(it) } }
    }
    fun clientDied() { handler.post { try { clear() } finally { kotlin.system.exitProcess(0) } } }
}
