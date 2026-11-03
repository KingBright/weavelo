package `fun`.hackerlife.weavelo.ui

import android.app.UiAutomation
import android.os.Process
import android.os.SystemClock
import android.view.View
import android.view.ViewGroup
import android.view.SurfaceView
import android.view.MotionEvent
import android.view.InputDevice
import android.view.accessibility.AccessibilityNodeInfo
import android.view.inputmethod.EditorInfo
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import `fun`.hackerlife.weavelo.MainActivity
import `fun`.hackerlife.weavelo.SessionConnectionService
import `fun`.hackerlife.weavelo.WeaveloClient
import `fun`.hackerlife.weavelo.WorkspaceClient
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.io.File

/** Explicit owned-window acceptance only; ordinary offline test runs skip this. */
class LiveWorkspaceKeyboardTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    private fun await(message: String, condition: () -> Boolean) {
        val deadline = SystemClock.uptimeMillis() + 30_000
        while (!condition()) {
            check(SystemClock.uptimeMillis() < deadline) {
                val state = WorkspaceClient.state.value
                "$message: connected=${state.connected}, active=${WorkspaceClient.active}, status=${state.status.take(160)}"
            }
            SystemClock.sleep(50)
        }
    }

    private fun editor(view: View): RemoteImeView? = when (view) {
        is RemoteImeView -> view.takeIf { it.isFocused && it.acceptingInput }
        is ViewGroup -> (0 until view.childCount).firstNotNullOfOrNull { editor(view.getChildAt(it)) }
        else -> null
    }

    private fun screen(view: View): SurfaceView? = when (view) {
        is SurfaceView -> view.takeIf { it.contentDescription == "Remote screen" && it.isShown }
        is ViewGroup -> (0 until view.childCount).firstNotNullOfOrNull { screen(view.getChildAt(it)) }
        else -> null
    }

    private fun click(label: String): Boolean {
        fun find(node: AccessibilityNodeInfo): AccessibilityNodeInfo? {
            if (node.text?.toString() == label || node.contentDescription?.toString() == label) return node
            for (index in 0 until node.childCount) node.getChild(index)?.let { child -> find(child)?.let { return it } }
            return null
        }
        val root = instrumentation.getUiAutomation(UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)
            .rootInActiveWindow ?: return false
        check(root.packageName?.toString() == "fun.hackerlife.weavelo") { "Owned app is not foreground" }
        var node = find(root) ?: return false
        while (!node.isClickable) node = node.parent ?: return false
        check(node.isEnabled && node.performAction(AccessibilityNodeInfo.ACTION_CLICK)) { "Owned control is unavailable" }
        return true
    }

    @Test fun committedUnicodeTraversesTheProductionWorkspaceEditor() {
        val args = InstrumentationRegistry.getArguments()
        val deviceId = args.getString("fixture_device_id")
        val windowId = args.getString("fixture_window_id")?.toLongOrNull()
        assumeTrue("Requires the explicitly selected owned Mac test window",
            deviceId == "67fd93f6629227c034d1354f60164480" && windowId != null && windowId > 0)
        val context = instrumentation.targetContext
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            await("Owned Studio discovery unavailable") { WeaveloClient.devices.value.any { it.id == deviceId && it.canViewScreen } }
            val device = WeaveloClient.devices.value.single { it.id == deviceId }
            try {
                // Enter through the real device page, retaining its routing,
                // lifecycle and Service ownership instead of replacing its root.
                await("Owned device action unavailable") { click(device.name) }
                await("Workspace connection unavailable") {
                    WorkspaceClient.state.value.connected && WorkspaceClient.requestedSession()?.deviceId == device.id
                }
                val sessionKey = WorkspaceClient.state.value.sessionKey
                WorkspaceClient.refreshSources(sessionKey, WorkspaceClient.state.value.connectionGeneration)
                await("Exact owned fixture source unavailable") { WorkspaceClient.state.value.sources.any {
                    it.title == "Weavelo input check" && it.application == "WeaveloInputFixture" &&
                        (it.source as? JSONObject)?.optLong("Window") == windowId
                } }
                val source = WorkspaceClient.state.value.sources.single {
                    it.title == "Weavelo input check" && it.application == "WeaveloInputFixture" &&
                        (it.source as? JSONObject)?.optLong("Window") == windowId
                }
                WorkspaceClient.add(source, sessionKey, WorkspaceClient.state.value.connectionGeneration)
                val diagnostic = File(context.externalCacheDir, "workspace-video.json")
                await("No decoded frame for this exact native session") {
                    runCatching { JSONObject(diagnostic.readText()).let {
                        it.optInt("pid") == Process.myPid() && it.optString("stage") == "first-decoder-output" &&
                            it.optJSONObject("source")?.optLong("Window") == windowId
                    } }.getOrDefault(false)
                }
                // Use the owner's normal remote click to focus the fixture's
                // editor. A selected window need not be the host's foreground
                // window, and keyboard safety must not depend on test setup.
                assertEquals(640, source.width)
                assertEquals(452, source.height)
                var point = 0f to 0f
                scenario.onActivity { activity ->
                    val view = checkNotNull(screen(activity.window.decorView))
                    check(view.hasWindowFocus() && view.width > 0 && view.height > 0)
                    val location = IntArray(2); view.getLocationOnScreen(location)
                    point = location[0] + view.width * .48f to location[1] + view.height * .68f
                }
                val down = SystemClock.uptimeMillis()
                for (action in listOf(MotionEvent.ACTION_DOWN, MotionEvent.ACTION_UP)) {
                    val event = MotionEvent.obtain(down, SystemClock.uptimeMillis(), action, point.first, point.second, 0)
                    event.source = InputDevice.SOURCE_TOUCHSCREEN
                    try {
                        assertTrue(instrumentation.getUiAutomation(UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)
                            .injectInputEvent(event, true))
                    } finally { event.recycle() }
                    SystemClock.sleep(30)
                }
                SystemClock.sleep(200)
                if (!click("Show keyboard")) {
                    assertTrue(click("Show stream controls"))
                    await("Keyboard control not visible") { click("Show keyboard") }
                }
                var focused: RemoteImeView? = null
                await("Production remote editor did not acquire focus") {
                    scenario.onActivity { focused = editor(it.window.decorView) }
                    focused != null
                }
                scenario.onActivity {
                    val connection = checkNotNull(focused).onCreateInputConnection(EditorInfo())
                    assertTrue(connection.setComposingText("nihao", 1))
                    assertTrue(connection.commitText("你好🙂", 1))
                    assertTrue(connection.finishComposingText())
                }
                SystemClock.sleep(1500)
                val pane = WorkspaceClient.state.value.panes.single()
                assertNull(pane.inputError)
                File(context.getExternalFilesDir(null), "live-workspace-keyboard.json").writeText(JSONObject()
                    .put("schema", 1).put("connection_id", WorkspaceClient.state.value.connectionId)
                    .put("subscription_id", pane.id).put("window_id", windowId)
                    .put("input_connection_commit_accepted", true).toString())
            } finally {
                WorkspaceClient.requestedSession()?.takeIf { it.deviceId == device.id }?.let {
                    SessionConnectionService.closeWorkspace(context, it.sessionKey)
                }
            }
        }
    }
}
