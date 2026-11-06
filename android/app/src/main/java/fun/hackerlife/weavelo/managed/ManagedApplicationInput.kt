package `fun`.hackerlife.weavelo.managed

import android.os.SystemClock
import android.view.InputDevice
import android.view.KeyEvent
import android.view.KeyCharacterMap
import android.view.MotionEvent
import org.json.JSONArray
import org.json.JSONObject

/** Native events bound to one owned virtual display, never the default display. */
internal class ManagedApplicationInput(val target: ManagedApplications.Target) {
    private data class Contact(val slot: Int, var x: Float, var y: Float, var pressure: Float)
    private val contacts = linkedMapOf<Long,Contact>()
    private var x = target.width/2f
    private var y = target.height/2f
    private var downTime = 0L
    private val heldKeys = mutableSetOf<Int>()
    private fun position(value: Float, extent: Int): Float {
        require(value.isFinite() && value in 0f..1f)
        return value*(extent-1)
    }
    private fun motion(action: Int): Boolean {
        if (contacts.isEmpty()) return true
        val properties = contacts.values.map { contact -> MotionEvent.PointerProperties().apply {
            id=contact.slot;toolType=MotionEvent.TOOL_TYPE_FINGER
        } }.toTypedArray()
        val coords = contacts.values.map { contact -> MotionEvent.PointerCoords().apply {
            x=contact.x;y=contact.y;pressure=contact.pressure;size=1f
        } }.toTypedArray()
        val event = MotionEvent.obtain(downTime,SystemClock.uptimeMillis(),action,contacts.size,
            properties,coords,0,0,1f,1f,0,0,InputDevice.SOURCE_TOUCHSCREEN,0)
        return try { target.host.injectMotion(target.displayId,event) } finally { event.recycle() }
    }
    private fun start(id: Long, px: Float, py: Float, pressure: Float): Boolean {
        if (id in contacts || contacts.size >= 10) return false
        val slot=(0..9).first { candidate -> contacts.values.none { it.slot==candidate } }
        if (contacts.isEmpty()) downTime=SystemClock.uptimeMillis()
        contacts[id]=Contact(slot,px,py,pressure)
        return motion(if (contacts.size==1) MotionEvent.ACTION_DOWN
            else MotionEvent.ACTION_POINTER_DOWN or ((contacts.size-1) shl MotionEvent.ACTION_POINTER_INDEX_SHIFT))
    }
    private fun end(id: Long): Boolean {
        val index=contacts.keys.indexOf(id)
        if (index<0) return false
        val accepted=motion(if (contacts.size==1) MotionEvent.ACTION_UP
            else MotionEvent.ACTION_POINTER_UP or (index shl MotionEvent.ACTION_POINTER_INDEX_SHIFT))
        contacts.remove(id)
        return accepted
    }
    private fun key(code: Int, down: Boolean, modifiers: Int=0): Boolean {
        val now=SystemClock.uptimeMillis()
        val event=KeyEvent(now,now,if(down) KeyEvent.ACTION_DOWN else KeyEvent.ACTION_UP,code,0,
            (if(modifiers and 1!=0) KeyEvent.META_SHIFT_ON else 0) or
            (if(modifiers and 0x0A!=0) KeyEvent.META_CTRL_ON else 0) or
            (if(modifiers and 0x14!=0) KeyEvent.META_ALT_ON else 0),
            -1,0,0,InputDevice.SOURCE_KEYBOARD)
        val accepted=target.host.injectKey(target.displayId,event)
        if (accepted) { if(down) heldKeys.add(code) else heldKeys.remove(code) }
        return accepted
    }
    private fun press(code: Int)=key(code,true)&&key(code,false)
    fun execute(event: JSONObject): Boolean {
        val name=event.keys().next();val value=event.get(name)
        return when(name) {
            "MouseMoveAbsolute", "MouseMove" -> {
                val point=value as JSONObject
                if(name=="MouseMoveAbsolute") {
                    x=position(point.getInt("x")/65535f,target.width);y=position(point.getInt("y")/65535f,target.height)
                } else {
                    x=(x+point.getInt("dx")).coerceIn(0f,(target.width-1).toFloat())
                    y=(y+point.getInt("dy")).coerceIn(0f,(target.height-1).toFloat())
                }
                contacts[-1]?.let { it.x=x;it.y=y;motion(MotionEvent.ACTION_MOVE) } ?: contacts.isEmpty()
            }
            "MouseDown" -> when((value as Number).toInt()) {
                0 -> contacts.isEmpty() && start(-1,x,y,1f)
                1 -> press(KeyEvent.KEYCODE_BACK)
                else -> false
            }
            "MouseUp" -> when((value as Number).toInt()) {0 -> end(-1);1 -> true;else -> false}
            "MouseScroll" -> {
                if(contacts.isNotEmpty()) false else {
                    val delta=value as JSONObject
                    if(!start(-1,x,y,1f)) false else {
                        contacts.getValue(-1).apply {
                            x=(x+delta.getInt("delta_x")*3f).coerceIn(0f,(target.width-1).toFloat())
                            y=(y+delta.getInt("delta_y")*3f).coerceIn(0f,(target.height-1).toFloat())
                        }
                        motion(MotionEvent.ACTION_MOVE)&&end(-1)
                    }
                }
            }
            "TouchFrame" -> touch(value as JSONArray)
            "Touch" -> touch(JSONArray().put(value))
            "ModifiersChanged" -> true
            "Key" -> {
                val info=value as JSONObject
                val code=when(info.getInt("key_code")) {
                    0x35 -> KeyEvent.KEYCODE_ESCAPE;0x24 -> KeyEvent.KEYCODE_ENTER;0x30 -> KeyEvent.KEYCODE_TAB
                    0x31 -> KeyEvent.KEYCODE_SPACE;0x33 -> KeyEvent.KEYCODE_DEL
                    0x7B -> KeyEvent.KEYCODE_DPAD_LEFT;0x7C -> KeyEvent.KEYCODE_DPAD_RIGHT
                    0x7D -> KeyEvent.KEYCODE_DPAD_DOWN;0x7E -> KeyEvent.KEYCODE_DPAD_UP
                    0x73 -> KeyEvent.KEYCODE_MOVE_HOME;0x77 -> KeyEvent.KEYCODE_MOVE_END
                    0x74 -> KeyEvent.KEYCODE_PAGE_UP;0x79 -> KeyEvent.KEYCODE_PAGE_DOWN
                    0x75 -> KeyEvent.KEYCODE_FORWARD_DEL
                    else -> {
                        val char=`fun`.hackerlife.weavelo.RemoteTextEdit.baseCharacter(info.getInt("key_code")) ?: return false
                        KeyCharacterMap.load(KeyCharacterMap.VIRTUAL_KEYBOARD).getEvents(charArrayOf(char))
                            ?.firstOrNull { it.action==KeyEvent.ACTION_DOWN }?.keyCode ?: return false
                    }
                }
                key(code,info.getBoolean("pressed"),info.optInt("modifiers"))
            }
            else -> false
        }
    }
    private fun touch(frame: JSONArray): Boolean {
        if(frame.length() !in 1..10 || contacts.containsKey(-1)) return false
        val changes=(0 until frame.length()).map { frame.getJSONObject(it) }
        if(changes.map { it.getLong("pointer_id") }.distinct().size!=changes.size) return false
        for(change in changes) {
            val id=change.getLong("pointer_id");val action=change.getString("action")
            if(id<0 || (action=="Down") == contacts.containsKey(id)) return false
            if(action !in listOf("Down","Move","Up","Cancel")) return false
            val pressure=change.getDouble("pressure").toFloat()
            require(pressure.isFinite()&&pressure in 0f..1f)
            val px=position(change.getDouble("normalized_x").toFloat(),target.width)
            val py=position(change.getDouble("normalized_y").toFloat(),target.height)
            contacts[id]?.apply { x=px;y=py;this.pressure=pressure }
        }
        if(changes.any { it.getString("action")=="Cancel" }) {
            // Android cancels a gesture as a whole. Turning cancellation into
            // ACTION_UP could activate a button the user deliberately left.
            val accepted=motion(MotionEvent.ACTION_CANCEL)
            contacts.clear()
            return accepted
        }
        for(change in changes.filter { it.getString("action")=="Down" }) {
            if(!start(change.getLong("pointer_id"),position(change.getDouble("normalized_x").toFloat(),target.width),
                position(change.getDouble("normalized_y").toFloat(),target.height),change.getDouble("pressure").toFloat())) return false
        }
        if(changes.any { it.getString("action")=="Move" } && !motion(MotionEvent.ACTION_MOVE)) return false
        for(change in changes.filter { it.getString("action")=="Up" }) if(!end(change.getLong("pointer_id"))) return false
        return true
    }
    fun release() {
        if(contacts.isNotEmpty()) runCatching { motion(MotionEvent.ACTION_CANCEL) }
        contacts.clear()
        heldKeys.toList().forEach { runCatching { key(it,false) } };heldKeys.clear()
    }
}
