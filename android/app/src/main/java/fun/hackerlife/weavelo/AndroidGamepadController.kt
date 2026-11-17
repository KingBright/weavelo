package `fun`.hackerlife.weavelo

import android.content.Context
import android.os.Build
import android.os.SystemClock
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import org.json.JSONArray
import org.json.JSONObject
import kotlin.math.abs
import kotlin.math.roundToInt

data class Ps5ConsoleSummary(
    val consoleId: Long,
    val hostId: String,
    val nickname: String,
    val hostAddr: String?,
    val status: String,
    val paired: Boolean,
)

data class Ps5PairResult(
    val ok: Boolean,
    val console: Ps5ConsoleSummary?,
    val error: String?,
)

object AndroidGamepadController {
    const val PSN_AUTHORIZE_URL: String =
        "https://auth.api.sonyentertainmentnetwork.com/2.0/oauth/authorize?service_entity=urn:service-entity:psn&response_type=code&client_id=ba495a24-818c-472b-b12d-ff231c1b5745&redirect_uri=https://remoteplay.dl.playstation.net/remoteplay/redirect&scope=psn:clientapp&request_locale=en_US&ui=pr&service_logo=ps&layout_type=popup&smcid=remoteplay&prompt=always&PlatformPrivacyWs1=minimal&"

    private const val BTN_CROSS = 1 shl 0
    private const val BTN_CIRCLE = 1 shl 1
    private const val BTN_SQUARE = 1 shl 2
    private const val BTN_TRIANGLE = 1 shl 3
    private const val BTN_DPAD_LEFT = 1 shl 4
    private const val BTN_DPAD_RIGHT = 1 shl 5
    private const val BTN_DPAD_UP = 1 shl 6
    private const val BTN_DPAD_DOWN = 1 shl 7
    private const val BTN_L1 = 1 shl 8
    private const val BTN_R1 = 1 shl 9
    private const val BTN_L3 = 1 shl 10
    private const val BTN_R3 = 1 shl 11
    private const val BTN_OPTIONS = 1 shl 12
    private const val BTN_SHARE = 1 shl 13
    private const val BTN_TOUCHPAD = 1 shl 14
    private const val BTN_PS = 1 shl 15

    private const val STICK_DEADZONE = 1800

    private val lock = Any()
    private var kind: String = "Generic"
    private var keyButtons: Int = 0
    private var hatButtons: Int = 0
    private var digitalL2: Boolean = false
    private var digitalR2: Boolean = false
    private var analogL2: Int = 0
    private var analogR2: Int = 0
    private var leftX: Int = 0
    private var leftY: Int = 0
    private var rightX: Int = 0
    private var rightY: Int = 0
    private var lastDeviceId: Int = -1
    private var lastRumbleIntensity: Int = 0
    private var lastRumbleAtMs: Long = 0L

    fun parseConsoles(rawJson: String): List<Ps5ConsoleSummary> {
        if (rawJson.isBlank()) return emptyList()
        val arr = JSONArray(rawJson)
        return (0 until arr.length()).mapNotNull { idx ->
            arr.optJSONObject(idx)?.let(::parseConsoleSummary)
        }
    }

    fun parsePairResult(rawJson: String): Ps5PairResult {
        val obj = JSONObject(rawJson)
        return Ps5PairResult(
            ok = obj.optBoolean("ok", false),
            console = obj.optJSONObject("console")?.let(::parseConsoleSummary),
            error = obj.optString("error").takeIf { !obj.isNull("error") && it.isNotBlank() },
        )
    }

    private fun parseConsoleSummary(obj: JSONObject): Ps5ConsoleSummary = Ps5ConsoleSummary(
        consoleId = obj.getLong("console_id"),
        hostId = obj.optString("host_id", ""),
        nickname = obj.optString("nickname", "PlayStation 5"),
        hostAddr = obj.optString("host_addr").takeIf { !obj.isNull("host_addr") && it.isNotBlank() },
        status = obj.optString("status", "offline"),
        paired = obj.optBoolean("paired", false),
    )

    fun isGamepadKeyEvent(event: KeyEvent): Boolean {
        val src = event.source
        val fromPad = (src and InputDevice.SOURCE_GAMEPAD) == InputDevice.SOURCE_GAMEPAD ||
            (src and InputDevice.SOURCE_JOYSTICK) == InputDevice.SOURCE_JOYSTICK ||
            (src and InputDevice.SOURCE_DPAD) == InputDevice.SOURCE_DPAD && event.device?.let(::isGameControllerDevice) == true
        if (!fromPad) return false
        return mapKeyCodeMask(event.keyCode) != 0 ||
            event.keyCode == KeyEvent.KEYCODE_BUTTON_L2 ||
            event.keyCode == KeyEvent.KEYCODE_BUTTON_R2
    }

    fun isGamepadMotionEvent(event: MotionEvent): Boolean {
        val src = event.source
        return ((src and InputDevice.SOURCE_JOYSTICK) == InputDevice.SOURCE_JOYSTICK ||
            (src and InputDevice.SOURCE_GAMEPAD) == InputDevice.SOURCE_GAMEPAD) &&
            event.actionMasked == MotionEvent.ACTION_MOVE
    }

    private fun isGameControllerDevice(device: InputDevice): Boolean {
        val sources = device.sources
        return (sources and InputDevice.SOURCE_GAMEPAD) == InputDevice.SOURCE_GAMEPAD ||
            (sources and InputDevice.SOURCE_JOYSTICK) == InputDevice.SOURCE_JOYSTICK
    }

    private fun detectKind(device: InputDevice?): String {
        if (device == null) return "Generic"
        return when (device.vendorId) {
            0x054C -> if (device.productId == 0x0CE6 || device.productId == 0x0DF2) "DualSense" else "DualShock4"
            0x045E -> "Xbox"
            0x057E -> "SwitchPro"
            else -> {
                val lower = device.name.lowercase()
                when {
                    "dualsense" in lower -> "DualSense"
                    "dualshock" in lower || "wireless controller" in lower -> "DualShock4"
                    "xbox" in lower -> "Xbox"
                    "switch" in lower || "pro controller" in lower -> "SwitchPro"
                    else -> "Generic"
                }
            }
        }
    }

    private fun mapKeyCodeMask(keyCode: Int): Int = when (keyCode) {
        KeyEvent.KEYCODE_BUTTON_A -> BTN_CROSS
        KeyEvent.KEYCODE_BUTTON_B -> BTN_CIRCLE
        KeyEvent.KEYCODE_BUTTON_X -> BTN_SQUARE
        KeyEvent.KEYCODE_BUTTON_Y -> BTN_TRIANGLE
        KeyEvent.KEYCODE_DPAD_LEFT -> BTN_DPAD_LEFT
        KeyEvent.KEYCODE_DPAD_RIGHT -> BTN_DPAD_RIGHT
        KeyEvent.KEYCODE_DPAD_UP -> BTN_DPAD_UP
        KeyEvent.KEYCODE_DPAD_DOWN -> BTN_DPAD_DOWN
        KeyEvent.KEYCODE_BUTTON_L1 -> BTN_L1
        KeyEvent.KEYCODE_BUTTON_R1 -> BTN_R1
        KeyEvent.KEYCODE_BUTTON_THUMBL -> BTN_L3
        KeyEvent.KEYCODE_BUTTON_THUMBR -> BTN_R3
        KeyEvent.KEYCODE_BUTTON_START -> BTN_OPTIONS
        KeyEvent.KEYCODE_BUTTON_SELECT, KeyEvent.KEYCODE_BACK -> BTN_SHARE
        KeyEvent.KEYCODE_BUTTON_MODE -> BTN_PS
        KeyEvent.KEYCODE_BUTTON_1 -> BTN_TOUCHPAD
        else -> 0
    }

    private fun applyStickDeadzone(raw: Int): Int {
        val clamped = raw.coerceIn(-32767, 32767)
        val absVal = abs(clamped)
        if (absVal <= STICK_DEADZONE) return 0
        val sign = if (clamped < 0) -1 else 1
        val rescaled = ((absVal - STICK_DEADZONE) * 32767) / (32767 - STICK_DEADZONE)
        return (sign * rescaled).coerceIn(-32767, 32767)
    }

    private fun axisToI16(value: Float): Int =
        applyStickDeadzone((value.coerceIn(-1f, 1f) * 32767f).roundToInt())

    fun onKeyEvent(event: KeyEvent): JSONObject? = synchronized(lock) {
        val pressed = event.action == KeyEvent.ACTION_DOWN
        if (!pressed && event.action != KeyEvent.ACTION_UP) return null
        if (pressed && event.repeatCount > 0) return null

        lastDeviceId = event.deviceId
        kind = detectKind(event.device)

        val before = snapshotTriple()
        when (event.keyCode) {
            KeyEvent.KEYCODE_BUTTON_L2 -> digitalL2 = pressed
            KeyEvent.KEYCODE_BUTTON_R2 -> digitalR2 = pressed
            else -> {
                val mask = mapKeyCodeMask(event.keyCode)
                if (mask == 0) return null
                keyButtons = if (pressed) (keyButtons or mask) else (keyButtons and mask.inv())
            }
        }
        if (snapshotTriple() == before) return null
        buildStateJsonLocked()
    }

    fun onMotionEvent(event: MotionEvent): JSONObject? = synchronized(lock) {
        val device = event.device
        val src = event.source
        lastDeviceId = event.deviceId
        kind = detectKind(device)
        val before = snapshotTriple()

        leftX = axisToI16(event.getAxisValue(MotionEvent.AXIS_X))
        leftY = axisToI16(event.getAxisValue(MotionEvent.AXIS_Y))

        val usesZ = device?.getMotionRange(MotionEvent.AXIS_Z, src) != null
        rightX = axisToI16(
            if (usesZ) event.getAxisValue(MotionEvent.AXIS_Z) else event.getAxisValue(MotionEvent.AXIS_RX)
        )
        val usesRz = device?.getMotionRange(MotionEvent.AXIS_RZ, src) != null
        rightY = axisToI16(
            if (usesRz) event.getAxisValue(MotionEvent.AXIS_RZ) else event.getAxisValue(MotionEvent.AXIS_RY)
        )

        val hatX = event.getAxisValue(MotionEvent.AXIS_HAT_X)
        val hatY = event.getAxisValue(MotionEvent.AXIS_HAT_Y)
        var hat = 0
        if (hatX < -0.5f) hat = hat or BTN_DPAD_LEFT
        else if (hatX > 0.5f) hat = hat or BTN_DPAD_RIGHT
        if (hatY < -0.5f) hat = hat or BTN_DPAD_UP
        else if (hatY > 0.5f) hat = hat or BTN_DPAD_DOWN
        hatButtons = hat

        val rawL2 = when {
            device?.getMotionRange(MotionEvent.AXIS_LTRIGGER, src) != null -> event.getAxisValue(MotionEvent.AXIS_LTRIGGER)
            device?.getMotionRange(MotionEvent.AXIS_BRAKE, src) != null -> event.getAxisValue(MotionEvent.AXIS_BRAKE)
            else -> 0f
        }
        val rawR2 = when {
            device?.getMotionRange(MotionEvent.AXIS_RTRIGGER, src) != null -> event.getAxisValue(MotionEvent.AXIS_RTRIGGER)
            device?.getMotionRange(MotionEvent.AXIS_GAS, src) != null -> event.getAxisValue(MotionEvent.AXIS_GAS)
            else -> 0f
        }
        analogL2 = (rawL2.coerceIn(0f, 1f) * 255f).roundToInt().coerceIn(0, 255)
        analogR2 = (rawR2.coerceIn(0f, 1f) * 255f).roundToInt().coerceIn(0, 255)

        if (snapshotTriple() == before) return null
        buildStateJsonLocked()
    }

    fun resetToNeutralJson(context: Context? = null): JSONObject? = synchronized(lock) {
        val wasActive = keyButtons != 0 || hatButtons != 0 || digitalL2 || digitalR2 ||
            analogL2 != 0 || analogR2 != 0 || leftX != 0 || leftY != 0 || rightX != 0 || rightY != 0
        keyButtons = 0
        hatButtons = 0
        digitalL2 = false
        digitalR2 = false
        analogL2 = 0
        analogR2 = 0
        leftX = 0
        leftY = 0
        rightX = 0
        rightY = 0
        stopVibrationLocked(context)
        if (wasActive) buildStateJsonLocked() else null
    }

    private data class PackedSnapshot(
        val buttons: Int,
        val l2: Int,
        val r2: Int,
        val lx: Int,
        val ly: Int,
        val rx: Int,
        val ry: Int,
    )

    private fun snapshotTriple(): PackedSnapshot {
        val l2 = if (analogL2 > 0) analogL2 else if (digitalL2) 255 else 0
        val r2 = if (analogR2 > 0) analogR2 else if (digitalR2) 255 else 0
        return PackedSnapshot(keyButtons or hatButtons, l2, r2, leftX, leftY, rightX, rightY)
    }

    private fun buildStateJsonLocked(): JSONObject {
        val snap = snapshotTriple()
        val emptyTouch = JSONObject()
            .put("active", false)
            .put("id", 0)
            .put("x", 0)
            .put("y", 0)
        return JSONObject()
            .put("controller_id", 0)
            .put("kind", kind)
            .put("buttons", snap.buttons)
            .put("left_trigger", snap.l2)
            .put("right_trigger", snap.r2)
            .put("left_stick_x", snap.lx)
            .put("left_stick_y", snap.ly)
            .put("right_stick_x", snap.rx)
            .put("right_stick_y", snap.ry)
            .put("touches", JSONArray().put(emptyTouch).put(JSONObject(emptyTouch.toString())))
            .put("has_imu", false)
            .put("gyro", JSONArray().put(0.0).put(0.0).put(0.0))
            .put("accel", JSONArray().put(0.0).put(0.0).put(0.0))
            .put("quat", JSONArray().put(0.0).put(0.0).put(0.0).put(1.0))
    }

    fun applyFeedback(context: Context?, feedback: JSONObject) = synchronized(lock) {
        val rumble = feedback.optJSONObject("Rumble") ?: return@synchronized
        val left = rumble.optInt("left_large", 0).coerceIn(0, 255)
        val right = rumble.optInt("right_small", 0).coerceIn(0, 255)
        val intensity = maxOf(left, (right * 4) / 5).coerceIn(0, 255)
        val now = SystemClock.elapsedRealtime()
        if (intensity < 6) {
            if (lastRumbleIntensity != 0) {
                lastRumbleIntensity = 0
                stopVibrationLocked(context)
            }
            return@synchronized
        }
        if (abs(intensity - lastRumbleIntensity) < 12 && now - lastRumbleAtMs < 60L) {
            return@synchronized
        }
        lastRumbleIntensity = intensity
        lastRumbleAtMs = now
        val vib = resolveVibrator(context) ?: return@synchronized
        runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                val effect = if (vib.hasAmplitudeControl()) {
                    VibrationEffect.createOneShot(90L, intensity.coerceIn(1, 255))
                } else {
                    VibrationEffect.createOneShot(45L, VibrationEffect.DEFAULT_AMPLITUDE)
                }
                vib.vibrate(effect)
            } else {
                @Suppress("DEPRECATION")
                vib.vibrate(45L)
            }
        }
    }

    private fun stopVibrationLocked(context: Context?) {
        runCatching { resolveVibrator(context)?.cancel() }
    }

    private fun resolveVibrator(context: Context?): Vibrator? {
        if (lastDeviceId >= 0) {
            InputDevice.getDevice(lastDeviceId)?.let { dev ->
                val devVib = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                    dev.vibratorManager.defaultVibrator
                } else {
                    @Suppress("DEPRECATION")
                    dev.vibrator
                }
                if (devVib.hasVibrator()) return devVib
            }
        }
        val ctx = context ?: return null
        val sysVib = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            ctx.getSystemService(VibratorManager::class.java)?.defaultVibrator
        } else {
            @Suppress("DEPRECATION")
            ctx.getSystemService(Context.VIBRATOR_SERVICE) as? Vibrator
        }
        return sysVib?.takeIf { it.hasVibrator() }
    }
}
