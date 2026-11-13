package `fun`.hackerlife.weavelo.managed

import android.content.Context
import android.hardware.display.DisplayManager
import android.os.Handler
import android.os.HandlerThread
import android.util.DisplayMetrics
import android.view.Display
import `fun`.hackerlife.weavelo.PublisherClient
import `fun`.hackerlife.weavelo.RemoteControlService
import org.json.JSONObject
import java.util.concurrent.FutureTask
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

@android.annotation.TargetApi(34)
internal class ManagedFallbackInput(
    private val context: Context,
    private val managed: IManagedScreen,
    private val isSessionRunning: () -> Boolean,
) : AutoCloseable {
    private val active = AtomicBoolean(true)
    private val thread = HandlerThread("weavelo-managed-input").apply { start() }
    private val handler = Handler(thread.looper)
    private var advertised = false
    private var generation = -1L
    private var currentInput: ManagedApplicationInput? = null

    fun start() {
        handler.post(loop)
    }

    private val loop = object : Runnable {
        override fun run() {
            if (!active.get() || !isSessionRunning()) return
            if (RemoteControlService.connected.value) {
                currentInput?.release()
                currentInput = null
                generation = -1L
                advertised = false
                handler.postDelayed(this, 250L)
                return
            }
            if (!advertised) {
                if (runCatching { PublisherClient.action(8, "true") }.isSuccess) {
                    advertised = true
                }
            }
            if (PublisherClient.controlSize == null) {
                currentInput?.release()
                currentInput = null
                generation = -1L
                handler.postDelayed(this, 100L)
                return
            }
            runCatching {
                val raw = PublisherClient.action(9)
                if (raw != "null") {
                    val job = JSONObject(raw)
                    when (job.getString("kind")) {
                        "Reset" -> {
                            currentInput?.release()
                            currentInput = null
                            generation = job.getLong("generation")
                        }
                        "Events" -> {
                            val jobId = job.getLong("token")
                            val gen = job.getLong("generation")
                            if (gen != generation) {
                                complete(gen, jobId, false)
                            } else {
                                val target = resolveTarget(job.get("source"))
                                if (target == null) {
                                    complete(gen, jobId, false)
                                } else {
                                    if (currentInput?.target != target) {
                                        currentInput?.release()
                                        currentInput = ManagedApplicationInput(target)
                                    }
                                    val dispatcher = checkNotNull(currentInput)
                                    val events = job.getJSONArray("events")
                                    var ok = true
                                    for (i in 0 until events.length()) {
                                        if (!stillValid(gen, jobId) || !dispatcher.execute(events.getJSONObject(i))) {
                                            ok = false
                                            break
                                        }
                                    }
                                    complete(gen, jobId, ok)
                                }
                            }
                        }
                    }
                }
            }.onFailure {
                currentInput?.release()
                currentInput = null
                generation = -1L
            }
            if (active.get() && isSessionRunning()) {
                handler.postDelayed(this, 12L)
            }
        }
    }

    private fun resolveTarget(source: Any?): ManagedApplications.Target? {
        if (source == "MainDisplay") {
            val size = PublisherClient.controlSize ?: return null
            val mainDisplay = context.getSystemService(DisplayManager::class.java)
                ?.getDisplay(Display.DEFAULT_DISPLAY) ?: return null
            val metrics = DisplayMetrics()
            @Suppress("DEPRECATION")
            mainDisplay.getRealMetrics(metrics)
            if (size.first != metrics.widthPixels || size.second != metrics.heightPixels) return null
            @Suppress("DEPRECATION")
            return ManagedApplications.Target(managed, Display.DEFAULT_DISPLAY, "", size.first, size.second, mainDisplay.rotation)
        }
        if (source is JSONObject) return ManagedApplications.target(source)
        return null
    }

    private fun stillValid(gen: Long, jobId: Long): Boolean =
        PublisherClient.action(11, JSONObject().put("generation", gen).put("token", jobId).toString()) == "true"

    private fun complete(gen: Long, jobId: Long, accepted: Boolean) {
        runCatching {
            PublisherClient.action(10, JSONObject().put("generation", gen).put("token", jobId).put("accepted", accepted).toString())
        }
    }

    override fun close() {
        active.set(false)
        val task = FutureTask {
            currentInput?.release()
            currentInput = null
            if (advertised && !RemoteControlService.connected.value) {
                runCatching { PublisherClient.action(8, "false") }
            }
            advertised = false
        }
        if (handler.post(task)) {
            runCatching { task.get(500, TimeUnit.MILLISECONDS) }
        }
        thread.quitSafely()
    }
}
