package `fun`.hackerlife.weavelo.octendra

import `fun`.hackerlife.weavelo.RemoteTextEdit
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Input router for an Octendra AI virtual display.
 * All events — including committed UTF-8 Text, multi-touch TouchFrame, Touch, Mouse*, and Key —
 * are forwarded directly to Octendra via `dispatch_input` so human takeover never falls back
 * to main-display accessibility gestures or main-screen IME.
 */
internal class OctendraAiInput(
    val target: OctendraAiViews.Target,
    private val client: OctendraAiClient,
    private val onStaleTarget: (OctendraAiViews.Target, Boolean) -> Unit = { _, _ -> },
) {
    companion object {
        private const val MAX_TEXT_BYTES = 1024
        private val SUPPORTED_TOUCH_ACTIONS = setOf("Down", "Move", "Up", "Cancel")

        internal fun macosKeyCodeToAndroidKeyCode(keyCode: Int): Int? = when (keyCode) {
            0x35 -> 111 // KEYCODE_ESCAPE
            0x24 -> 66  // KEYCODE_ENTER
            0x30 -> 61  // KEYCODE_TAB
            0x31 -> 62  // KEYCODE_SPACE
            0x33 -> 67  // KEYCODE_DEL
            0x7B -> 21  // KEYCODE_DPAD_LEFT
            0x7C -> 22  // KEYCODE_DPAD_RIGHT
            0x7D -> 20  // KEYCODE_DPAD_DOWN
            0x7E -> 19  // KEYCODE_DPAD_UP
            0x73 -> 122 // KEYCODE_MOVE_HOME
            0x77 -> 123 // KEYCODE_MOVE_END
            0x74 -> 92  // KEYCODE_PAGE_UP
            0x79 -> 93  // KEYCODE_PAGE_DOWN
            0x75 -> 112 // KEYCODE_FORWARD_DEL
            else -> when (val ch = RemoteTextEdit.baseCharacter(keyCode)) {
                in 'a'..'z' -> 29 + (ch!! - 'a')
                in '0'..'9' -> 7 + (ch!! - '0')
                '-' -> 69
                '=' -> 70
                '[' -> 71
                ']' -> 72
                '\\' -> 73
                ';' -> 74
                '\'' -> 75
                ',' -> 55
                '.' -> 56
                '/' -> 76
                '`' -> 68
                ' ' -> 62
                else -> null
            }
        }

        private fun validTouchContact(contact: JSONObject): Boolean {
            val action = contact.optString("action", "")
            if (action !in SUPPORTED_TOUCH_ACTIONS) return false
            val pointerId = contact.optLong("pointer_id", -1L)
            if (pointerId !in 0L..0xFFFFFFFFL) return false
            val nx = contact.optDouble("normalized_x", Double.NaN)
            val ny = contact.optDouble("normalized_y", Double.NaN)
            val pressure = contact.optDouble("pressure", 1.0)
            return nx.isFinite() && nx in 0.0..1.0 &&
                ny.isFinite() && ny in 0.0..1.0 &&
                pressure.isFinite() && pressure in 0.0..1.0
        }
    }

    private val invalidated = AtomicBoolean(false)
    private val released = AtomicBoolean(false)

    val isActive: Boolean
        get() = !invalidated.get() && !released.get()

    fun execute(event: JSONObject): Boolean {
        if (invalidated.get() || released.get() || !target.supportsInput) return false
        val keys = event.keys()
        if (!keys.hasNext()) return false
        val name = keys.next()
        val payload = prepareEventPayload(name, event) ?: return false

        val result = client.dispatchInput(
            consumerId = target.consumerId,
            viewId = target.viewId,
            generation = target.generation,
            sourceRevision = target.sourceRevision,
            eventJson = payload.toString(),
        )
        if (!result.ok) {
            when (result.error) {
                "stale_view_or_consumer" -> {
                    invalidated.set(true)
                    onStaleTarget(target, true)
                }
                "source_revision_mismatch" -> {
                    invalidated.set(true)
                    onStaleTarget(target, false)
                }
            }
            return false
        }
        return result.accepted
    }

    fun release() {
        if (!released.compareAndSet(false, true)) return
        runCatching { client.releaseInput(target.consumerId) }
    }

    private fun prepareEventPayload(name: String, original: JSONObject): JSONObject? {
        return when (name) {
            "Text" -> {
                val text = original.opt("Text") as? String ?: return null
                if (text.isEmpty() || text.toByteArray(Charsets.UTF_8).size > MAX_TEXT_BYTES) return null
                original
            }
            "TouchFrame" -> {
                val frame = original.optJSONArray("TouchFrame") ?: return null
                if (frame.length() !in 1..10) return null
                for (i in 0 until frame.length()) {
                    val contact = frame.optJSONObject(i) ?: return null
                    if (!validTouchContact(contact)) return null
                }
                original
            }
            "Touch" -> {
                val contact = original.optJSONObject("Touch") ?: return null
                if (!validTouchContact(contact)) return null
                original
            }
            "MouseMoveAbsolute" -> {
                val pt = original.optJSONObject("MouseMoveAbsolute") ?: return null
                if (!pt.has("x") || !pt.has("y")) return null
                original
            }
            "MouseMove" -> {
                val pt = original.optJSONObject("MouseMove") ?: return null
                if (!pt.has("dx") || !pt.has("dy")) return null
                original
            }
            "MouseDown", "MouseUp" -> {
                if (original.opt(name) !is Number) return null
                original
            }
            "MouseScroll" -> {
                original.optJSONObject("MouseScroll") ?: return null
                original
            }
            "ModifiersChanged" -> {
                if (original.opt("ModifiersChanged") !is Number) return null
                original
            }
            "Key" -> {
                val info = original.optJSONObject("Key") ?: return null
                if (!info.has("key_code") || !info.has("pressed")) return null
                val androidCode = macosKeyCodeToAndroidKeyCode(info.optInt("key_code", -1))
                if (androidCode == null || info.has("android_key_code")) {
                    original
                } else {
                    JSONObject(original.toString()).apply {
                        getJSONObject("Key").put("android_key_code", androidCode)
                    }
                }
            }
            else -> null
        }
    }
}
