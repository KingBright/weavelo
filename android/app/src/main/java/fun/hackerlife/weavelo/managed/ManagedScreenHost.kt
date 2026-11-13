package `fun`.hackerlife.weavelo.managed

import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.app.ActivityManager
import android.app.ActivityOptions
import android.content.Intent
import android.os.Binder
import android.os.Bundle
import android.os.IBinder
import android.os.Handler
import android.os.HandlerThread
import android.os.Parcel
import android.os.IInterface
import android.os.SystemClock
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

/** Refresh the dispatch timestamp without changing the identity of a held gesture. */
internal fun cancelManagedMotion(event: MotionEvent, now: Long): MotionEvent {
    val properties = Array(event.pointerCount) { index ->
        MotionEvent.PointerProperties().also { event.getPointerProperties(index,it) }
    }
    val coords = Array(event.pointerCount) { index ->
        MotionEvent.PointerCoords().also { event.getPointerCoords(index,it) }
    }
    return MotionEvent.obtain(event.downTime,now,MotionEvent.ACTION_CANCEL,event.pointerCount,properties,coords,
        event.metaState,event.buttonState,event.xPrecision,event.yPrecision,event.deviceId,event.edgeFlags,event.source,event.flags)
}

/** A canceled key-up releases Android's held state without activating the key. */
internal fun cancelManagedKey(event: KeyEvent, now: Long): KeyEvent = KeyEvent(
    event.downTime,now,KeyEvent.ACTION_UP,event.keyCode,0,event.metaState,event.deviceId,
    event.scanCode,event.flags or KeyEvent.FLAG_CANCELED,event.source)

/** Runs only inside Weavelo's already authorized shell process. */
internal class ManagedScreenHost(@Volatile private var appUid: Int) : IManagedScreen.Stub() {
    private val thread = HandlerThread("weavelo-display").apply { start() }
    private val handler = Handler(thread.looper)
    private val context by lazy { ManagedShellContext.create() }
    private val manager by lazy {
        DisplayManager::class.java.getDeclaredConstructor(android.content.Context::class.java)
            .apply { isAccessible = true }.newInstance(context)
    }
    private var mirror: VirtualDisplay? = null
    private val applications = linkedMapOf<Int, VirtualDisplay>()
    private val applicationPackages = mutableMapOf<Int, String>()
    private data class Geometry(val width: Int, val height: Int, val rotation: Int)
    private val applicationSizes = mutableMapOf<Int, Geometry>()
    private val motions = mutableMapOf<Int, MotionEvent>()
    private val keys = mutableMapOf<Int, MutableMap<Int, KeyEvent>>()
    private var taskListener: IInterface? = null
    private var applicationDisplayListener: DisplayManager.DisplayListener? = null
    private val taskEvents = mutableMapOf<Int, Long>()
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
            val cause = error.cause.let { if (it is java.lang.reflect.InvocationTargetException) it.targetException else it }
            throw IllegalStateException(cause?.message ?: "Display operation failed", cause)
        }
    }
    private fun metrics(): DisplayMetrics = DisplayMetrics().also {
        @Suppress("DEPRECATION")
        checkNotNull(manager.getDisplay(Display.DEFAULT_DISPLAY)).getRealMetrics(it)
    }
    override fun info(): String = call {
        val size = metrics()
        JSONObject().put("api", 5).put("width", size.widthPixels).put("height", size.heightPixels)
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
    override fun applicationIcon(packageName: String, edge: Int): ByteArray {
        check(Binder.getCallingUid() == appUid) { "Weavelo UID required" }
        require(packageName.length in 1..255 && edge in listOf(64,256))
        val entry=checkNotNull(launchers().find { it.activityInfo.packageName==packageName }) { "App is not launchable" }
        // Read-only icon work stays off the display/input thread. It runs once
        // for catalog changes or selected-app artwork, never on a media frame.
        val drawable=entry.loadIcon(context.packageManager)
        val bitmap=android.graphics.Bitmap.createBitmap(edge,edge,android.graphics.Bitmap.Config.ARGB_8888)
        return try {
            bitmap.setHasAlpha(true)
            drawable.setBounds(0,0,edge,edge)
            drawable.draw(android.graphics.Canvas(bitmap))
            val output=java.io.ByteArrayOutputStream()
            check(bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG,100,output))
            // Skia adds ancillary PNG chunks to our sRGB RGBA bitmap. The
            // shared artwork format deliberately carries only IHDR/IDAT/IEND.
            // Keep their original compressed pixels and CRCs; no image decode,
            // recompression or relaxed network validation is needed.
            val png=output.toByteArray()
            val normalized=java.io.ByteArrayOutputStream(png.size)
            normalized.write(png,0,8)
            var offset=8
            while(offset<png.size) {
                require(offset<=png.size-12)
                val length=java.nio.ByteBuffer.wrap(png,offset,4).int
                require(length>=0 && length<=png.size-offset-12)
                val type=String(png,offset+4,4,Charsets.US_ASCII)
                if(type in listOf("IHDR","IDAT","IEND")) normalized.write(png,offset,length+12)
                else require(png[offset+4].toInt() and 32 != 0) {"Unsupported critical native PNG chunk"}
                offset+=length+12
            }
            normalized.toByteArray().also {require(it.size<=if(edge==64) 16*1024 else 256*1024)}
        } finally {bitmap.recycle()}
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
        // Android accessibility ignores private virtual displays (except SystemUI).
        // Public makes this same-user display's editor available to the user's
        // authorized accessibility service; it does not mirror the main screen.
        // Keep own content, touch, destroy owned content on removal, trusted
        // display, independent focus, and never steal top focus.
        // Do not set ALWAYS_UNLOCKED, SECURE or keyguard-bypass flags.
        ensureTaskListener()
        val flags = DisplayManager.VIRTUAL_DISPLAY_FLAG_PUBLIC or DisplayManager.VIRTUAL_DISPLAY_FLAG_OWN_CONTENT_ONLY or
            (1 shl 6) or (1 shl 7) or (1 shl 8) or (1 shl 10) or (1 shl 14) or (1 shl 16)
        val display = checkNotNull(manager.createVirtualDisplay("Weavelo · ${entry.loadLabel(context.packageManager)}",
            width, height, density, null, flags)) { "Independent display unavailable" }
        val id = display.display.displayId
        applications[id] = display
        applicationPackages[id] = packageName
        applicationSizes[id] = geometry(id)
        try {
            val intent = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
                .setClassName(packageName, entry.activityInfo.name)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_MULTIPLE_TASK)
            // app_process has no Instrumentation or registered application thread.
            // Use the task service as the actual shell caller, restricted here to
            // this validated launcher, user and display owned by Weavelo.
            val service = Class.forName("android.app.ActivityTaskManager").getMethod("getService").invoke(null)
            val result = Class.forName("android.app.IActivityTaskManager").getMethod("startActivityAsUser",
                Class.forName("android.app.IApplicationThread"), String::class.java, String::class.java,
                Intent::class.java, String::class.java, IBinder::class.java, String::class.java,
                Int::class.javaPrimitiveType, Int::class.javaPrimitiveType, Class.forName("android.app.ProfilerInfo"),
                Bundle::class.java, Int::class.javaPrimitiveType).invoke(service, null,
                ManagedShellContext.SHELL_PACKAGE, null, intent, null, null, null, 0, 0, null,
                ActivityOptions.makeBasic().setLaunchDisplayId(id).toBundle(), appUid / 100_000) as Int
            check(ActivityManager::class.java.getMethod("isStartResultSuccessful", Int::class.javaPrimitiveType)
                .invoke(null, result) as Boolean) { "Android rejected independent app launch ($result)" }
            handler.postDelayed({ initialTaskOrientation(id,0) },100L)
            id
        } catch (error: Exception) { releaseOwned(id); throw error }
    }
    private fun initialTaskOrientation(id: Int, attempt: Int) {
        val packageName = applicationPackages[id] ?: return
        @Suppress("DEPRECATION")
        val task = context.getSystemService(ActivityManager::class.java).getRunningTasks(256).find {
            it.javaClass.getField("displayId").getInt(it) == id && it.topActivity?.packageName == packageName
        }
        if (task == null) {
            if (attempt < 10) handler.postDelayed({ initialTaskOrientation(id,attempt+1) },100L)
            return
        }
        if (taskEvents.containsKey(task.id)) return
        @Suppress("DEPRECATION")
        val orientation = runCatching { context.packageManager.getActivityInfo(checkNotNull(task.topActivity),0).screenOrientation }.getOrNull() ?: return
        taskEvents[task.id]=1L
        applyTaskOrientation(task.id,orientation,1L,0)
    }
    /** Android delivers orientation requests independently of default-display focus. */
    private fun ensureTaskListener() {
        if (taskListener != null) return
        val iface = Class.forName("android.app.ITaskStackListener")
        val stub = Class.forName("android.app.ITaskStackListener\$Stub")
        fun transaction(name: String) = stub.getDeclaredField("TRANSACTION_$name").apply { isAccessible = true }.getInt(null)
        val taskChange = transaction("onTaskRequestedOrientationChanged")
        val activityChange = transaction("onActivityRequestedOrientationChanged")
        val binder = object : Binder() {
            override fun onTransact(code: Int, data: Parcel, reply: Parcel?, flags: Int): Boolean {
                if (code == INTERFACE_TRANSACTION) { reply?.writeString("android.app.ITaskStackListener"); return true }
                if (code == taskChange || code == activityChange) {
                    if (Binder.getCallingUid() != android.os.Process.SYSTEM_UID) return false
                    data.enforceInterface("android.app.ITaskStackListener")
                    val task = data.readInt(); val orientation = data.readInt()
                    handler.post {
                        val sequence = (taskEvents[task] ?: 0L) + 1
                        taskEvents[task] = sequence
                        applyTaskOrientation(task,orientation,sequence,0)
                    }
                    return true
                }
                // These are system-server one-way notifications; no unrelated
                // event changes an owned display.
                return true
            }
        }
        val listener = java.lang.reflect.Proxy.newProxyInstance(iface.classLoader, arrayOf(iface)) { _, method, _ ->
            when (method.name) { "asBinder" -> binder; "toString" -> "Weavelo task listener"; else -> null }
        } as IInterface
        val service = Class.forName("android.app.ActivityTaskManager").getMethod("getService").invoke(null)
        service.javaClass.getMethod("registerTaskStackListener",iface).invoke(service,listener)
        taskListener = listener
        applicationDisplayListener = object : DisplayManager.DisplayListener {
            override fun onDisplayAdded(id: Int) = Unit
            override fun onDisplayRemoved(id: Int) = Unit
            override fun onDisplayChanged(id: Int) {
                if (id in applications && runCatching { geometry(id) != applicationSizes[id] }.getOrDefault(true))
                    cancelInput(id)
            }
        }.also { manager.registerDisplayListener(it,handler) }
    }
    private fun applyTaskOrientation(taskId: Int, orientation: Int, sequence: Long, attempt: Int) {
        if (taskEvents[taskId] != sequence || failed) return
        @Suppress("DEPRECATION")
        val task = context.getSystemService(ActivityManager::class.java).getRunningTasks(256).find { it.id == taskId }
        val id = task?.let { it.javaClass.getField("displayId").getInt(it) }
        if (id == null || applications[id] == null || task.topActivity?.packageName != applicationPackages[id]) {
            if (attempt < 3) handler.postDelayed({ applyTaskOrientation(taskId,orientation,sequence,attempt+1) },100L)
            else taskEvents.remove(taskId,sequence)
            return
        }
        val rotation = when (orientation) {
            android.content.pm.ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE,
            android.content.pm.ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE,
            android.content.pm.ActivityInfo.SCREEN_ORIENTATION_USER_LANDSCAPE -> Surface.ROTATION_90
            android.content.pm.ActivityInfo.SCREEN_ORIENTATION_REVERSE_LANDSCAPE -> Surface.ROTATION_270
            android.content.pm.ActivityInfo.SCREEN_ORIENTATION_PORTRAIT,
            android.content.pm.ActivityInfo.SCREEN_ORIENTATION_SENSOR_PORTRAIT,
            android.content.pm.ActivityInfo.SCREEN_ORIENTATION_USER_PORTRAIT,
            android.content.pm.ActivityInfo.SCREEN_ORIENTATION_UNSPECIFIED -> Surface.ROTATION_0
            android.content.pm.ActivityInfo.SCREEN_ORIENTATION_REVERSE_PORTRAIT -> Surface.ROTATION_180
            else -> return
        }
        if (geometry(id).rotation == rotation) return
        cancelInput(id)
        // Freeze only this owned virtual display; never change the phone's
        // physical display or the user's global rotation preference.
        try {
            val service = Class.forName("android.view.WindowManagerGlobal").getMethod("getWindowManagerService").invoke(null)
            val method = service.javaClass.methods.first { it.name == "freezeDisplayRotation" && it.parameterCount in 2..3 }
            if (method.parameterCount == 2) method.invoke(service,id,rotation)
            else method.invoke(service,id,rotation,"Weavelo independent app")
        } catch (error: Exception) {
            android.util.Log.e("Weavelo","Independent display rotation failed",error)
            failed=true; clear()
        }
    }
    private fun owned(id: Int): VirtualDisplay = checkNotNull(applications[id] ?: mirror?.takeIf { it.display.displayId == id }) {
        "Screen capture is no longer owned"
    }
    private val inputManager by lazy {
        Class.forName("android.hardware.input.InputManager").getMethod("getInstance").invoke(null)
    }
    private fun geometry(id: Int): Geometry {
        val display = if (id == Display.DEFAULT_DISPLAY) {
            check(mirror != null) { "Main display input requires active screen capture" }
            checkNotNull(manager.getDisplay(Display.DEFAULT_DISPLAY))
        } else owned(id).display
        val size = DisplayMetrics()
        @Suppress("DEPRECATION")
        display.getRealMetrics(size)
        return Geometry(size.widthPixels, size.heightPixels, display.rotation)
    }
    override fun applicationGeometry(id: Int): String = call {
        check(applications.containsKey(id)) { "Application display is no longer owned" }
        val g = geometry(id)
        JSONObject().put("width",g.width).put("height",g.height).put("rotation",g.rotation).toString()
    }
    override fun acceptApplicationGeometry(id: Int, width: Int, height: Int, rotation: Int) = call {
        check(applications.containsKey(id) && geometry(id) == Geometry(width,height,rotation)) { "Application geometry changed again" }
        applicationSizes[id] = Geometry(width,height,rotation)
    }
    private fun send(id: Int, event: InputEvent): Boolean {
        InputEvent::class.java.getMethod("setDisplayId", Int::class.javaPrimitiveType).invoke(event, id)
        return inputManager.javaClass.getMethod("injectInputEvent", InputEvent::class.java, Int::class.javaPrimitiveType)
            .invoke(inputManager, event, 1) as Boolean
    }
    private fun inject(id: Int, expected: Geometry, event: InputEvent): Boolean {
        if (id == Display.DEFAULT_DISPLAY) {
            check(mirror != null) { "Main display input requires active screen capture" }
            check(geometry(Display.DEFAULT_DISPLAY) == expected) {
                "Main display geometry changed"
            }
            return send(Display.DEFAULT_DISPLAY, event)
        }
        check(applications.containsKey(id)) { "Input requires an owned independent display" }
        check(applicationSizes[id] == expected && geometry(id) == expected) {
            "Independent display geometry changed"
        }
        return send(id, event)
    }
    override fun injectMotion(id: Int, width: Int, height: Int, rotation: Int, event: MotionEvent): Boolean = call {
        try {
            val size = DisplayMetrics()
            val targetDisplay = if (id == Display.DEFAULT_DISPLAY) {
                check(mirror != null) { "Main display input requires active screen capture" }
                checkNotNull(manager.getDisplay(Display.DEFAULT_DISPLAY))
            } else owned(id).display
            @Suppress("DEPRECATION")
            targetDisplay.getRealMetrics(size)
            require(event.pointerCount in 1..10 && event.source == InputDevice.SOURCE_TOUCHSCREEN)
            require(event.actionMasked in listOf(MotionEvent.ACTION_DOWN, MotionEvent.ACTION_UP, MotionEvent.ACTION_MOVE,
                MotionEvent.ACTION_CANCEL, MotionEvent.ACTION_POINTER_DOWN, MotionEvent.ACTION_POINTER_UP))
            for (pointer in 0 until event.pointerCount) {
                require(event.getX(pointer).isFinite() && event.getY(pointer).isFinite() &&
                    event.getX(pointer) in 0f..(size.widthPixels-1).toFloat() && event.getY(pointer) in 0f..(size.heightPixels-1).toFloat())
            }
            val accepted = inject(id, Geometry(width,height,rotation), event)
            if (accepted) {
                motions.remove(id)?.recycle()
                if (event.actionMasked != MotionEvent.ACTION_UP && event.actionMasked != MotionEvent.ACTION_CANCEL) {
                    motions[id] = if (event.actionMasked == MotionEvent.ACTION_POINTER_UP) {
                        val indices=(0 until event.pointerCount).filter { it != event.actionIndex }
                        val properties=indices.map { index -> MotionEvent.PointerProperties().also { event.getPointerProperties(index,it) } }.toTypedArray()
                        val coords=indices.map { index -> MotionEvent.PointerCoords().also { event.getPointerCoords(index,it) } }.toTypedArray()
                        MotionEvent.obtain(event.downTime,event.eventTime,MotionEvent.ACTION_MOVE,indices.size,properties,coords,
                            event.metaState,event.buttonState,event.xPrecision,event.yPrecision,event.deviceId,event.edgeFlags,event.source,event.flags)
                    } else MotionEvent.obtain(event)
                }
            }
            accepted
        } finally { event.recycle() }
    }
    override fun injectKey(id: Int, width: Int, height: Int, rotation: Int, event: KeyEvent): Boolean = call {
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
                KeyEvent.KEYCODE_COMMA,KeyEvent.KEYCODE_PERIOD,KeyEvent.KEYCODE_SLASH,KeyEvent.KEYCODE_GRAVE) ||
            (id == Display.DEFAULT_DISPLAY && event.keyCode == KeyEvent.KEYCODE_HOME)) {
            "Global device keys are not independent-app input"
        }
        val accepted = inject(id, Geometry(width,height,rotation), event)
        if (accepted) {
            val held = keys.getOrPut(id) { mutableMapOf() }
            if (event.action == KeyEvent.ACTION_DOWN) held[event.keyCode] = KeyEvent(event) else held.remove(event.keyCode)
        }
        accepted
    }
    private fun cancelInput(id: Int) {
        motions.remove(id)?.let { event ->
            try {
                val cancel = cancelManagedMotion(event,SystemClock.uptimeMillis())
                try { runCatching { send(id,cancel) } } finally { cancel.recycle() }
            } finally { event.recycle() }
        }
        keys.remove(id)?.values?.forEach { event ->
            runCatching { send(id,cancelManagedKey(event,SystemClock.uptimeMillis())) }
        }
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
        if (applications.containsKey(id)) {
            cancelInput(id)
        }
        val vd = owned(id)
        val alt = if (density > 120) density - 1 else density + 1
        vd.resize(width, height, alt)
        vd.resize(width, height, density)
    }
    override fun release(id: Int) = call { owned(id); releaseOwned(id) }
    private fun releaseOwned(id: Int) {
        cancelInput(id)
        applicationSizes.remove(id)
        applicationPackages.remove(id)
        val display = applications.remove(id) ?: mirror?.takeIf { it.display.displayId == id }?.also { mirror = null }
        val surface = display?.surface
        try { display?.release() } finally { surface?.release() }
    }
    override fun ensureMainDisplayAwake(): Boolean = call {
        val mainDisplay = checkNotNull(manager.getDisplay(Display.DEFAULT_DISPLAY))
        if (mainDisplay.state == Display.STATE_ON) return@call true
        val now = SystemClock.uptimeMillis()
        runCatching {
            val sm = Class.forName("android.os.ServiceManager").getMethod("getService", String::class.java)
            val powerBinder = sm.invoke(null, "power") as? IBinder
            if (powerBinder != null) {
                val power = Class.forName("android.os.IPowerManager\$Stub")
                    .getMethod("asInterface", IBinder::class.java).invoke(null, powerBinder)
                val wake4 = power?.javaClass?.methods?.firstOrNull { it.name == "wakeUp" && it.parameterCount == 4 }
                val wake3 = power?.javaClass?.methods?.firstOrNull { it.name == "wakeUp" && it.parameterCount == 3 }
                if (wake4 != null) {
                    wake4.invoke(power, now, 2, "weavelo:remote", ManagedShellContext.SHELL_PACKAGE)
                } else if (wake3 != null) {
                    wake3.invoke(power, now, "weavelo:remote", ManagedShellContext.SHELL_PACKAGE)
                }
            }
        }
        if (mainDisplay.state != Display.STATE_ON) {
            val down = KeyEvent(now, now, KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_WAKEUP, 0, 0, -1, 0, 0, InputDevice.SOURCE_KEYBOARD)
            val up = KeyEvent(now, now, KeyEvent.ACTION_UP, KeyEvent.KEYCODE_WAKEUP, 0, 0, -1, 0, 0, InputDevice.SOURCE_KEYBOARD)
            runCatching { send(Display.DEFAULT_DISPLAY, down); send(Display.DEFAULT_DISPLAY, up) }
        }
        var attempts = 0
        while (mainDisplay.state != Display.STATE_ON && attempts < 12) {
            Thread.sleep(25)
            attempts += 1
        }
        mainDisplay.state == Display.STATE_ON
    }
    private fun clear() {
        cancelInput(Display.DEFAULT_DISPLAY)
        val ids = applications.keys.toList() + listOfNotNull(mirror?.display?.displayId)
        ids.forEach { runCatching { releaseOwned(it) } }
        taskListener?.let { listener -> runCatching {
            val iface = Class.forName("android.app.ITaskStackListener")
            val service = Class.forName("android.app.ActivityTaskManager").getMethod("getService").invoke(null)
            service.javaClass.getMethod("unregisterTaskStackListener",iface).invoke(service,listener)
        } }
        taskListener = null
        applicationDisplayListener?.let { manager.unregisterDisplayListener(it) }
        applicationDisplayListener = null
        taskEvents.clear()
    }
    fun resetForReconnect(nextUid: Int = appUid) {
        val task = FutureTask {
            clear()
            appUid = nextUid
            failed = false
        }
        if (handler.post(task)) {
            runCatching { task.get(2, TimeUnit.SECONDS) }
        }
    }
    fun clientDied() = resetForReconnect()
}
