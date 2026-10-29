package `fun`.hackerlife.weavelo

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.content.Context
import android.content.Intent
import android.graphics.Path
import android.hardware.display.DisplayManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.DisplayMetrics
import android.view.Display
import android.view.View
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.json.JSONArray
import org.json.JSONObject

/** Android owns the grant. The service never enables itself or records input. */
class RemoteControlService : AccessibilityService() {
    companion object {
        private val mutableConnected = MutableStateFlow(false)
        val connected = mutableConnected.asStateFlow()
        fun openSettings(context: Context) {
            context.startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
    }
    private val handler = Handler(Looper.getMainLooper())
    private var running = false
    private var generation = -1L
    private var work = 0L
    private var busy = false
    private var x = 0f
    private var y = 0f
    private var stroke: GestureDescription.StrokeDescription? = null
    private var touchPointer: Int? = null
    private val poll = object : Runnable {
        override fun run() {
            if (!running) return
            if (!mutableConnected.value || PublisherClient.controlSize == null) {
                if (generation != -1L) reset(-1)
                handler.postDelayed(this,250)
                return
            }
            runCatching {
                val raw = PublisherClient.action(9)
                if (raw != "null") {
                    val job = JSONObject(raw)
                    when (job.getString("kind")) {
                        "Reset" -> if (generation != job.getLong("generation")) reset(job.getLong("generation"))
                        "Events" -> {
                            val token = job.getLong("token"); val gen = job.getLong("generation")
                            if (busy || gen != generation) finish(gen,token,false)
                            else {
                                busy = true
                                execute(job.getJSONArray("events"),0,gen,token,work)
                            }
                        }
                    }
                }
            }.onFailure { reset(-1) }
            handler.postDelayed(this,16)
        }
    }
    override fun onServiceConnected() {
        super.onServiceConnected()
        if (!WeaveloClient.initialize(this)) return
        PublisherClient.action(8,"true")
        if (android.os.Build.VERSION.SDK_INT >= 33) {
            serviceInfo = serviceInfo.apply { flags = flags or android.accessibilityservice.AccessibilityServiceInfo.FLAG_INPUT_METHOD_EDITOR }
        }
        mutableConnected.value = true
        running = true
        handler.removeCallbacks(poll); handler.post(poll)
    }
    override fun onAccessibilityEvent(event: AccessibilityEvent?) = Unit
    override fun onInterrupt() {
        runCatching { PublisherClient.action(8,"false") }
        mutableConnected.value = false
        reset(-1)
    }
    override fun onDestroy() {
        running = false; handler.removeCallbacks(poll)
        runCatching { PublisherClient.action(8,"false") }
        mutableConnected.value = false
        reset(-1)
        super.onDestroy()
    }
    private fun dimensions(): Pair<Int,Int>? {
        val size = PublisherClient.controlSize ?: return null
        val metrics = DisplayMetrics()
        @Suppress("DEPRECATION")
        getSystemService(DisplayManager::class.java).getDisplay(Display.DEFAULT_DISPLAY)?.getRealMetrics(metrics)
        return size.takeIf { it.first == metrics.widthPixels && it.second == metrics.heightPixels && it.first > 1 && it.second > 1 }
    }
    private fun reset(next: Long) {
        generation = next; val version = ++work; busy = true; touchPointer = null
        val old = stroke; stroke = null
        fun released() {
            if (work == version) {
                dimensions()?.let { x = it.first/2f; y = it.second/2f }
                busy = false
            }
        }
        if (old == null) released()
        else {
            val path = Path().apply { moveTo(x,y) }
            val release = old.continueStroke(path,0,1,false)
            dispatch(release) { released() }
        }
    }
    private fun finish(gen: Long, token: Long, accepted: Boolean) {
        runCatching { PublisherClient.action(10,JSONObject().put("generation",gen).put("token",token).put("accepted",accepted).toString()) }
        if (gen == generation) busy = false
    }
    private fun valid(gen: Long, token: Long) = PublisherClient.action(11,
        JSONObject().put("generation",gen).put("token",token).toString()) == "true"
    private fun execute(events: JSONArray, index: Int, gen: Long, token: Long, version: Long) {
        if (version != work) return
        if (!valid(gen,token)) { finish(gen,token,false); return }
        if (index == events.length()) { finish(gen,token,true); return }
        val size = dimensions()
        if (size == null) { finish(gen,token,false); return }
        val event = events.getJSONObject(index); val name = event.keys().next()
        val value = event.get(name)
        fun done(accepted: Boolean) {
            if (version != work) return
            if (accepted) execute(events,index+1,gen,token,version) else finish(gen,token,false)
        }
        fun move(nx: Float, ny: Float) {
            if (stroke == null) { x = nx; y = ny; done(true) }
            else continueTouch(nx,ny,false,::done)
        }
        when (name) {
            "MouseMoveAbsolute" -> {
                val p = value as JSONObject
                move(p.getInt("x")/65535f*(size.first-1),p.getInt("y")/65535f*(size.second-1))
            }
            "MouseMove" -> {
                val p = value as JSONObject
                move((x+p.getInt("dx")).coerceIn(0f,(size.first-1).toFloat()),(y+p.getInt("dy")).coerceIn(0f,(size.second-1).toFloat()))
            }
            "MouseDown" -> when ((value as Number).toInt()) {
                0 -> if (stroke == null) startTouch(::done) else done(false)
                1 -> done(performGlobalAction(GLOBAL_ACTION_BACK))
                2 -> done(performGlobalAction(GLOBAL_ACTION_HOME))
                else -> done(false)
            }
            "MouseUp" -> if ((value as Number).toInt() == 0) {
                if (stroke != null) continueTouch(x,y,true,::done) else done(false)
            } else done((value as Number).toInt() in 1..2)
            "MouseScroll" -> {
                if (stroke != null) { done(false); return }
                val delta = value as JSONObject
                val nx = (x+delta.getInt("delta_x")*3f).coerceIn(1f,(size.first-2).toFloat())
                val ny = (y+delta.getInt("delta_y")*3f).coerceIn(1f,(size.second-2).toFloat())
                val path = Path().apply { moveTo(x,y); lineTo(nx,ny) }
                dispatch(GestureDescription.StrokeDescription(path,0,80),::done)
            }
            "Text" -> {
                val text = value as? String
                if (android.os.Build.VERSION.SDK_INT < 33 || text.isNullOrEmpty()) { done(false); return }
                // The existing accessibility grant remains necessary. This is an
                // input-method connection to the focused editor, never SET_TEXT
                // on an inspected field or a read/replace of its contents.
                val connection = inputMethod?.currentInputConnection
                if (connection == null) { done(false); return }
                connection.commitText(text, 1, null)
                done(true)
            }
            "ModifiersChanged" -> done(true) // Chord ownership is enforced by the native InputLease.
            "Key" -> {
                val key = value as JSONObject
                done(!key.getBoolean("pressed") || this.key(key.getInt("key_code"),key.getInt("modifiers")))
            }
            "Touch" -> {
                val touch = value as JSONObject; val pointer = touch.getInt("pointer_id")
                val nx = touch.getDouble("normalized_x").toFloat()*(size.first-1)
                val ny = touch.getDouble("normalized_y").toFloat()*(size.second-1)
                when (touch.getString("action")) {
                    "Down" -> if (touchPointer == null && stroke == null) { touchPointer = pointer; x = nx; y = ny; startTouch(::done) } else done(false)
                    "Move" -> if (touchPointer == pointer && stroke != null) continueTouch(nx,ny,false,::done) else done(false)
                    "Up", "Cancel" -> if (touchPointer == pointer && stroke != null) { touchPointer = null; continueTouch(nx,ny,true,::done) } else done(false)
                    else -> done(false)
                }
            }
            else -> done(false)
        }
    }
    private fun startTouch(done: (Boolean) -> Unit) {
        val path = Path().apply { moveTo(x,y) }
        val next = GestureDescription.StrokeDescription(path,0,8,true)
        stroke = next
        dispatch(next) { ok -> if (!ok && stroke === next) stroke = null; done(ok) }
    }
    private fun continueTouch(nx: Float, ny: Float, release: Boolean, done: (Boolean) -> Unit) {
        val previous = stroke ?: return done(false)
        val path = Path().apply { moveTo(x,y); lineTo(nx,ny) }
        val next = previous.continueStroke(path,0,8,!release)
        x = nx; y = ny; stroke = if (release) null else next
        dispatch(next) { ok -> if (!ok && stroke === next) stroke = null; done(ok) }
    }
    private fun dispatch(stroke: GestureDescription.StrokeDescription, done: (Boolean) -> Unit) {
        val callback = object : GestureResultCallback() {
            override fun onCompleted(gestureDescription: GestureDescription?) { done(true) }
            override fun onCancelled(gestureDescription: GestureDescription?) { done(false) }
        }
        if (!dispatchGesture(GestureDescription.Builder().addStroke(stroke).build(),callback,handler)) done(false)
    }
    private fun key(code: Int, modifiers: Int): Boolean {
        if (code == 0x35 && modifiers == 0) return performGlobalAction(GLOBAL_ACTION_BACK)
        val node = rootInActiveWindow?.findFocus(AccessibilityNodeInfo.FOCUS_INPUT) ?: return false
        if (code == 0x30 && modifiers and 0x1E == 0) {
            return node.focusSearch(if (modifiers and 1 == 0) View.FOCUS_FORWARD else View.FOCUS_BACKWARD)
                ?.performAction(AccessibilityNodeInfo.ACTION_FOCUS) == true
        }
        if (!node.isEditable || node.isPassword) return false
        if (modifiers and 0x0A != 0 && modifiers and 0x14 == 0) {
            val action = when (code) { 0x08 -> AccessibilityNodeInfo.ACTION_COPY; 0x07 -> AccessibilityNodeInfo.ACTION_CUT; 0x09 -> AccessibilityNodeInfo.ACTION_PASTE; else -> null }
            if (action != null) return node.performAction(action)
        }
        if (code == 0x24 && modifiers == 0 && Build.VERSION.SDK_INT >= 30
            && node.actionList.any { it.id == AccessibilityNodeInfo.AccessibilityAction.ACTION_IME_ENTER.id })
            return node.performAction(AccessibilityNodeInfo.AccessibilityAction.ACTION_IME_ENTER.id)
        val before = node.text?.toString() ?: ""
        val edit = RemoteTextEdit.edit(before,node.textSelectionStart,node.textSelectionEnd,code,modifiers) ?: return false
        if (edit.text != before && !node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT,
                Bundle().apply { putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,edit.text) })) return false
        return node.performAction(AccessibilityNodeInfo.ACTION_SET_SELECTION,Bundle().apply {
            putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT,edit.start)
            putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT,edit.end)
        })
    }
}
