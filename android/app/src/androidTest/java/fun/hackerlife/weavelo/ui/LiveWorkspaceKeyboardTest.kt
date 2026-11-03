package `fun`.hackerlife.weavelo.ui

import android.os.Process
import android.os.SystemClock
import android.view.View
import android.view.ViewGroup
import android.view.accessibility.AccessibilityNodeInfo
import android.view.inputmethod.EditorInfo
import androidx.activity.compose.setContent
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import `fun`.hackerlife.weavelo.MainActivity
import `fun`.hackerlife.weavelo.SessionConnectionService
import `fun`.hackerlife.weavelo.WeaveloClient
import `fun`.hackerlife.weavelo.WorkspaceClient
import `fun`.hackerlife.weavelo.WorkspaceSessionRequest
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
            check(SystemClock.uptimeMillis() < deadline) { message }
            SystemClock.sleep(50)
        }
    }

    private fun editor(view: View): RemoteImeView? = when (view) {
        is RemoteImeView -> view.takeIf { it.isFocused && it.acceptingInput }
        is ViewGroup -> (0 until view.childCount).firstNotNullOfOrNull { editor(view.getChildAt(it)) }
        else -> null
    }

    private fun click(label: String): Boolean {
        fun find(node: AccessibilityNodeInfo): AccessibilityNodeInfo? {
            if (node.text?.toString() == label || node.contentDescription?.toString() == label) return node
            for (index in 0 until node.childCount) node.getChild(index)?.let { child -> find(child)?.let { return it } }
            return null
        }
        val root = instrumentation.uiAutomation.rootInActiveWindow ?: return false
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
            val sessionKey = WorkspaceClient.sessionKeyFor(device.endpoint, device.id)
            try {
                scenario.onActivity { activity -> activity.setContent {
                    WorkspaceScreen(device.endpoint, deviceId = device.id,
                        incoming = WorkspaceSessionRequest(device.endpoint, device.id, sessionKey), onClose = {})
                } }
                await("Workspace connection unavailable") { WorkspaceClient.state.value.let { it.sessionKey == sessionKey && it.connected } }
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
                SessionConnectionService.closeWorkspace(context, sessionKey)
            }
        }
    }
}
