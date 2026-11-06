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
import `fun`.hackerlife.weavelo.managed.ManagedApplications
import `fun`.hackerlife.weavelo.managed.ManagedApplicationInput

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
    private var appInput: ManagedApplicationInput? = null
    private var appSource: Any? = null
    private data class ContactStroke(val x: Float, val y: Float, val stroke: GestureDescription.StrokeDescription)
    private var contacts = linkedMapOf<Long, ContactStroke>()
    private var pollScheduled = false
    private fun schedulePoll(delay: Long) {
        if (!running) return
        handler.removeCallbacks(poll)
        pollScheduled = true
        handler.postDelayed(poll, delay)
    }
    private val poll = object : Runnable {
        override fun run() {
            pollScheduled = false
            if (!running) return
            if (!mutableConnected.value || PublisherClient.controlSize == null) {
                if (generation != -1L) reset(-1)
                schedulePoll(250)
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
                                val source=job.get("source")
                                val target=ManagedApplications.target(source)
                                if(source is JSONObject && target==null) { finish(gen,token,false);schedulePoll(0);return }
                                if(appInput?.target!=target) { appInput?.release();appInput=target?.let(::ManagedApplicationInput) }
                                appSource=source
                                busy = true
                                execute(job.getJSONArray("events"),0,gen,token,work)
                            }
                        }
                    }
                }
            }.onFailure { reset(-1) }
            if (!pollScheduled) schedulePoll(if (contacts.isNotEmpty() || stroke != null) 4 else 16)
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
        schedulePoll(0)
    }
    override fun onAccessibilityEvent(event: AccessibilityEvent?) = Unit
    override fun onInterrupt() {
        // Interrupt current feedback/gestures; the system still owns this binding.
        // Keep its generation so later fresh input can resume after cancellation.
        reset(generation)
    }
    override fun onDestroy() {
        running = false; handler.removeCallbacks(poll)
        runCatching { PublisherClient.action(8,"false") }
        mutableConnected.value = false
        reset(-1)
        super.onDestroy()
    }
    private fun dimensions(): Pair<Int,Int>? {
        appInput?.let { input -> return if(ManagedApplications.target(appSource)==input.target)
            input.target.width to input.target.height else null }
        val size = PublisherClient.controlSize ?: return null
        val metrics = DisplayMetrics()
        @Suppress("DEPRECATION")
        getSystemService(DisplayManager::class.java).getDisplay(Display.DEFAULT_DISPLAY)?.getRealMetrics(metrics)
        return size.takeIf { it.first == metrics.widthPixels && it.second == metrics.heightPixels && it.first > 1 && it.second > 1 }
    }
    private fun reset(next: Long) {
        appInput?.release();appInput=null;appSource=null
        generation = next; val version = ++work; busy = true
        val old = stroke; stroke = null
        val oldContacts = contacts; contacts = linkedMapOf()
        fun released() {
            if (work == version) {
                dimensions()?.let { x = it.first/2f; y = it.second/2f }
                busy = false
                schedulePoll(0)
            }
        }
        val releases = oldContacts.values.map { contact ->
            contact.stroke.continueStroke(Path().apply { moveTo(contact.x,contact.y) },0,1,false)
        }.toMutableList()
        if (old != null) {
            val path = Path().apply { moveTo(x,y) }
            releases.add(old.continueStroke(path,0,1,false))
        }
        if (releases.isEmpty()) released() else dispatch(releases) { released() }
    }
    private fun finish(gen: Long, token: Long, accepted: Boolean) {
        runCatching { PublisherClient.action(10,JSONObject().put("generation",gen).put("token",token).put("accepted",accepted).toString()) }
        if (gen == generation) { busy = false; schedulePoll(0) }
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
        val native=appInput
        if(native!=null && name!="Text") { done(native.execute(event));return }
        fun move(nx: Float, ny: Float) {
            if (stroke == null) { x = nx; y = ny; done(true) }
            else continueTouch(nx,ny,false,::done)
        }
        if (contacts.isNotEmpty() && name.startsWith("Mouse")) { done(false); return }
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
                if(native!=null) {
                    // The accessibility IME connection is global, so it cannot
                    // prove which display owns a committed text event. Resolve
                    // the editor from this exact owned display instead.
                    val target=native.target
                    val windows=windowsOnAllDisplays[target.displayId].orEmpty()
                    val roots=windows.mapNotNull {it.root}
                    val focused=roots.mapNotNull {it.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)}
                    val node=focused.firstOrNull { it.packageName?.toString()==target.packageName && it.isEditable && !it.isPassword }
                    if(node==null) {
                        android.util.Log.w("Weavelo","App text target unavailable display=${target.displayId} windows=${windows.size} roots=${roots.size} focused=${focused.size}")
                        done(false);return
                    }
                    val before=node.text?.toString() ?: ""
                    val edit=RemoteTextEdit.insert(before,node.textSelectionStart,node.textSelectionEnd,text)
                    if(edit==null || !node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT,
                            Bundle().apply { putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,edit.text) })) {
                        android.util.Log.w("Weavelo","App text commit failed display=${target.displayId} selection=${node.textSelectionStart}:${node.textSelectionEnd} editable=${node.isEditable}")
                        done(false);return
                    }
                    done(node.performAction(AccessibilityNodeInfo.ACTION_SET_SELECTION,Bundle().apply {
                        putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT,edit.start)
                        putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT,edit.end)
                    }));return
                }
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
            "TouchFrame" -> dispatchContacts(value as JSONArray,size,::done)
            else -> done(false)
        }
    }
    /** Continue every held finger in one OS gesture, including stationary ones. */
    private fun dispatchContacts(frame: JSONArray, size: Pair<Int,Int>, done: (Boolean) -> Unit) {
        if (stroke != null || frame.length() !in 1..10) { done(false); return }
        data class Change(val action: String, val x: Float, val y: Float)
        val changes = linkedMapOf<Long,Change>()
        for (i in 0 until frame.length()) {
            val c=frame.getJSONObject(i);val id=c.getLong("pointer_id");val action=c.getString("action")
            val nx=c.getDouble("normalized_x").toFloat();val ny=c.getDouble("normalized_y").toFloat()
            if (id !in 0..0xffffffffL || changes.containsKey(id) || !nx.isFinite() || !ny.isFinite()
                || nx !in 0f..1f || ny !in 0f..1f || action !in listOf("Down","Move","Up","Cancel")
                || (action=="Down") == contacts.containsKey(id)) { done(false); return }
            changes[id]=Change(action,nx*(size.first-1),ny*(size.second-1))
        }
        val ids=(contacts.keys+changes.keys).distinct()
        if (ids.size > GestureDescription.getMaxStrokeCount()) { done(false); return }
        // Android rejects a continuation with no MotionEvents. Retain the
        // previous stroke IDs for the next real move/up instead of dispatching
        // a stationary frame and revoking an otherwise healthy input lease.
        if (changes.all { (id, change) -> change.action == "Move" &&
            contacts[id]?.let { !nativeGestureMoved(it.x,it.y,change.x,change.y) } == true
        }) { done(true); return }
        val previous=contacts
        val next=linkedMapOf<Long,ContactStroke>()
        val strokes=ArrayList<GestureDescription.StrokeDescription>(ids.size)
        for (id in ids) {
            val old=previous[id];val change=changes[id]
            val nx=if(change?.action=="Cancel") old!!.x else change?.x ?: old!!.x
            val ny=if(change?.action=="Cancel") old!!.y else change?.y ?: old!!.y
            val keep=change?.action !in listOf("Up","Cancel")
            val path=Path().apply { moveTo(old?.x ?: nx,old?.y ?: ny); if(old!=null) lineTo(nx,ny) }
            // Android requires the first continuation step to contain exactly
            // the previously held strokes. Adding a new finger at t=0 cancels
            // the entire gesture (MotionEventInjector.prepareToContinueOldGesture).
            // Start new contacts one millisecond later, with the same end time.
            val start=if(previous.isNotEmpty()) 1L else 0L
            val updated=old?.stroke?.continueStroke(path,0,8,keep)
                ?: GestureDescription.StrokeDescription(path,start,8-start,keep)
            strokes.add(updated)
            if(keep) next[id]=ContactStroke(nx,ny,updated)
        }
        contacts=next
        dispatch(strokes) { ok ->
            if(!ok && contacts===next) contacts=previous
            done(ok)
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
        if (!release && !nativeGestureMoved(x,y,nx,ny)) { done(true); return }
        val path = Path().apply { moveTo(x,y); lineTo(nx,ny) }
        val next = previous.continueStroke(path,0,8,!release)
        x = nx; y = ny; stroke = if (release) null else next
        dispatch(next) { ok -> if (!ok && stroke === next) stroke = null; done(ok) }
    }
    private fun dispatch(stroke: GestureDescription.StrokeDescription, done: (Boolean) -> Unit) {
        dispatch(listOf(stroke),done)
    }
    private fun dispatch(strokes: List<GestureDescription.StrokeDescription>, done: (Boolean) -> Unit) {
        val callback = object : GestureResultCallback() {
            override fun onCompleted(gestureDescription: GestureDescription?) { done(true) }
            override fun onCancelled(gestureDescription: GestureDescription?) { done(false) }
        }
        val builder=GestureDescription.Builder()
        strokes.forEach { builder.addStroke(it) }
        if (!dispatchGesture(builder.build(),callback,handler)) done(false)
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
