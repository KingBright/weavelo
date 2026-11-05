package `fun`.hackerlife.weavelo.ui

import android.app.UiAutomation
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.os.SystemClock
import android.util.DisplayMetrics
import android.view.MotionEvent
import android.view.View
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import `fun`.hackerlife.weavelo.MainActivity
import `fun`.hackerlife.weavelo.PublisherClient
import `fun`.hackerlife.weavelo.RemoteControlService
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Test
import java.io.File
import java.util.UUID
import java.util.concurrent.CopyOnWriteArrayList

/** Native events: isolated input-only touch fixture, or the existing real screen for mouse. */
class LivePublisherTouchTest {
    @Test fun simultaneousContactsReachTheNativeView() = nativeFixture(false)
    @Test fun mouseContinuationReachesTheNativeView() = nativeFixture(true)
    private fun nativeFixture(mouse: Boolean) {
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        assumeTrue(InstrumentationRegistry.getArguments().getString(if (mouse) "native_mouse_fixture" else "native_touch_fixture") == "true")
        instrumentation.getUiAutomation(UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)
        val context = instrumentation.targetContext
        val root = checkNotNull(context.getExternalFilesDir(null))
        val ready = File(root, "native-touch-ready.json")
        val report = File(root, "native-touch-result.json")
        val senderFinished = File(root, "native-touch-sender-finished.txt")
        ready.delete(); report.delete(); senderFinished.delete()
        val records = CopyOnWriteArrayList<JSONObject>()
        val nonce = UUID.randomUUID().toString()
        val armed = java.util.concurrent.atomic.AtomicBoolean(false)
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            try {
                val deadline = SystemClock.uptimeMillis() + 15_000
                while (!RemoteControlService.connected.value && SystemClock.uptimeMillis() < deadline) SystemClock.sleep(50)
                assertTrue("Existing user-approved accessibility service required", RemoteControlService.connected.value)
                if (mouse) assertTrue("Mouse fixture observes the existing real screen publisher", PublisherClient.active)
                else assertFalse("Do not interrupt an existing screen publisher", PublisherClient.active)
                val metrics = DisplayMetrics()
                lateinit var fixture: View
                scenario.onActivity { activity ->
                    @Suppress("DEPRECATION")
                    activity.windowManager.defaultDisplay.getRealMetrics(metrics)
                    fixture = object : View(activity) {
                        private val paint = Paint().apply { color = Color.WHITE; textSize = 42f }
                        override fun onDraw(canvas: Canvas) {
                            canvas.drawColor(Color.rgb(25, 35, 45))
                            canvas.drawText("Weavelo touch acceptance", 40f, 180f, paint)
                            canvas.drawText("Native events: ${records.size}", 40f, 245f, paint)
                        }
                        override fun onTouchEvent(event: MotionEvent): Boolean {
                            val location = IntArray(2); getLocationOnScreen(location)
                            val points = JSONArray()
                            for (i in 0 until event.pointerCount) points.put(JSONObject().put("id", event.getPointerId(i))
                                .put("x", event.getX(i) + location[0]).put("y", event.getY(i) + location[1]))
                            records.add(JSONObject().put("action", event.actionMasked).put("index", event.actionIndex)
                                .put("time", event.eventTime).put("points", points))
                            postInvalidate(); return true
                        }
                        override fun onWindowFocusChanged(focused: Boolean) {
                            super.onWindowFocusChanged(focused)
                            if (!focused && armed.get() && !mouse) {
                                PublisherClient.controlSize = null
                                runCatching { PublisherClient.action(1) }
                            }
                        }
                    }
                    activity.setContentView(fixture)
                }
                instrumentation.waitForIdleSync()
                assertTrue("Fixture must own the visible window", fixture.hasWindowFocus())
                // Touch uses an explicit input-only publisher. Mouse borrows
                // the real screen publisher without replacing its lifecycle.
                if (!mouse) {
                  PublisherClient.action(0, JSONObject().put("bind", "0.0.0.0:5001")
                    .put("directory", File(context.filesDir, "received").apply { mkdirs() }.absolutePath)
                    .put("audio", false).put("control", true).toString())
                val grant = JSONObject(PublisherClient.action(2)).getLong("grant")
                PublisherClient.action(3, JSONObject().put("grant", grant).put("width", metrics.widthPixels)
                    .put("height", metrics.heightPixels).toString())
                PublisherClient.controlSize = metrics.widthPixels to metrics.heightPixels
                } else assertEquals(metrics.widthPixels to metrics.heightPixels, PublisherClient.controlSize)
                armed.set(true)
                ready.writeText(JSONObject().put("nonce", nonce).put("width", metrics.widthPixels)
                    .put("height", metrics.heightPixels).put("capture", mouse).put("mouse", mouse).toString())
                val end = SystemClock.uptimeMillis() + 60_000
                while (SystemClock.uptimeMillis() < end && records.none {
                    it.getInt("action") in listOf(MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL)
                }) SystemClock.sleep(25)
                val proof = JSONObject().put("nonce", nonce).put("events", JSONArray(records.toList()))
                report.writeText(proof.toString())
                // The harness writes this nonce after close_verified returns.
                // A cleared peer list alone can race the final close receipt.
                val cleanupDeadline = SystemClock.uptimeMillis() + 5_000
                while (SystemClock.uptimeMillis() < cleanupDeadline &&
                    runCatching { senderFinished.readText().trim() }.getOrNull() != nonce
                ) SystemClock.sleep(25)
                val down = records.firstOrNull { it.getInt("action") == MotionEvent.ACTION_DOWN }
                assertNotNull("Native DOWN was not observed", down)
                val held = down!!.getJSONArray("points").getJSONObject(0).getInt("id")
                if (!mouse) {
                  assertTrue("Two contacts must reach the OS together", records.any { it.getJSONArray("points").length() == 2 })
                val secondUp = records.indexOfFirst { it.getInt("action") == MotionEvent.ACTION_POINTER_UP }
                assertTrue("Second finger must release independently", secondUp >= 0)
                assertTrue("First finger must still move after the second releases", records.drop(secondUp + 1).any {
                    it.getInt("action") == MotionEvent.ACTION_MOVE && it.getJSONArray("points").length() == 1 &&
                        it.getJSONArray("points").getJSONObject(0).getInt("id") == held
                })
                }
                assertEquals("Final release required", MotionEvent.ACTION_UP, records.last().getInt("action"))
                assertFalse("Gesture must not be cancelled between samples", records.any { it.getInt("action") == MotionEvent.ACTION_CANCEL })
                val first = down.getJSONArray("points").getJSONObject(0)
                assertEquals((metrics.widthPixels - 1) * .3, first.getDouble("x"), 3.0)
                assertEquals((metrics.heightPixels - 1) * .4, first.getDouble("y"), 3.0)
            } finally {
                ready.delete()
                senderFinished.delete()
                if (armed.get() && !mouse) {
                    PublisherClient.controlSize = null
                    PublisherClient.action(1)
                }
            }
        }
    }
}
