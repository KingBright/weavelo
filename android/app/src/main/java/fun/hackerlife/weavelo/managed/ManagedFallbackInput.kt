package `fun`.hackerlife.weavelo.managed

import android.content.Context
import android.hardware.display.DisplayManager
import android.os.Handler
import android.os.HandlerThread
import android.util.DisplayMetrics
import android.view.Display
import `fun`.hackerlife.weavelo.PublisherClient
import `fun`.hackerlife.weavelo.RemoteControlService
import `fun`.hackerlife.weavelo.octendra.OctendraAiInput
import `fun`.hackerlife.weavelo.octendra.OctendraAiViews
import org.json.JSONObject
import org.json.JSONArray
import java.util.concurrent.FutureTask
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

@android.annotation.TargetApi(34)
internal class ManagedFallbackInput(
    private val context: Context,
    private val managed: IManagedScreen,
    private val isSessionRunning: () -> Boolean,
) : AutoCloseable {
    companion object {
        internal fun shouldPollInput(controlSize: Pair<Int, Int>?, aiControlEnabled: Boolean, aiHasActiveTargets: Boolean) =
            controlSize != null || (aiControlEnabled && aiHasActiveTargets)

        internal fun dispatchAiEvents(events: JSONArray, dispatcher: OctendraAiInput, valid: () -> Boolean): Boolean {
            for (i in 0 until events.length()) {
                if (!valid() || !dispatcher.execute(events.getJSONObject(i))) return false
            }
            return true
        }
    }

    private val active = AtomicBoolean(true)
    private val thread = HandlerThread("weavelo-managed-input").apply { start() }
    private val handler = Handler(thread.looper)
    private var advertised = false
    private var generation = -1L
    private var currentInput: ManagedApplicationInput? = null
    private var aiInput: OctendraAiInput? = null
    private var aiSource: Any? = null

    private fun releaseInputs() {
        currentInput?.release(); currentInput = null
        aiInput?.release(); aiInput = null; aiSource = null
    }

    fun start() {
        handler.post(loop)
    }

    private val loop = object : Runnable {
        override fun run() {
            if (!active.get() || !isSessionRunning()) return
            if (RemoteControlService.connected.value) {
                releaseInputs()
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
            if (aiInput != null && OctendraAiViews.target(aiSource) != aiInput?.target) {
                aiInput?.release(); aiInput = null; aiSource = null
            }
            if (!shouldPollInput(PublisherClient.controlSize, OctendraAiViews.controlEnabled, OctendraAiViews.hasActiveTargets())) {
                releaseInputs()
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
                            releaseInputs()
                            generation = job.getLong("generation")
                        }
                        "Events" -> {
                            val jobId = job.getLong("token")
                            val gen = job.getLong("generation")
                            if (gen != generation) {
                                complete(gen, jobId, false)
                            } else {
                                val source = job.get("source")
                                val aiTarget = OctendraAiViews.target(source)
                                val target = if (aiTarget == null) resolveTarget(source) else null
                                if (aiTarget != null && OctendraAiViews.controlEnabled && aiTarget.supportsInput) {
                                    currentInput?.release(); currentInput = null
                                    val client = OctendraAiViews.clientOrNull()
                                    if (client == null) {
                                        releaseInputs(); complete(gen, jobId, false)
                                    } else {
                                        if (aiInput?.target != aiTarget) {
                                            aiInput?.release()
                                            aiInput = OctendraAiInput(aiTarget, client) { stale, force ->
                                                OctendraAiViews.invalidateTarget(stale, force)
                                            }
                                        }
                                        aiSource = source
                                        val dispatcher = checkNotNull(aiInput)
                                        val events = job.getJSONArray("events")
                                        val ok = dispatchAiEvents(events, dispatcher) {
                                            stillValid(gen, jobId) && OctendraAiViews.target(source) == aiTarget
                                        }
                                        if (!ok) releaseInputs()
                                        complete(gen, jobId, ok)
                                    }
                                } else if (target != null && aiTarget == null) {
                                    aiInput?.release(); aiInput = null; aiSource = null
                                    if (currentInput?.target != target) {
                                        currentInput?.release()
                                        currentInput = ManagedApplicationInput(target)
                                    }
                                    val dispatcher = checkNotNull(currentInput)
                                    val events = job.getJSONArray("events")
                                    var ok = true
                                    for (i in 0 until events.length()) {
                                        if (!stillValid(gen, jobId) || !dispatcher.execute(events.getJSONObject(i))) {
                                            ok = false; break
                                        }
                                    }
                                    if (!ok) releaseInputs()
                                    complete(gen, jobId, ok)
                                } else {
                                    releaseInputs(); complete(gen, jobId, false)
                                }
                            }
                        }
                    }
                }
            }.onFailure {
                releaseInputs()
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
            releaseInputs()
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
