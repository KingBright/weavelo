package `fun`.hackerlife.weavelo

import android.content.Context
import org.json.JSONObject
import java.io.File

/** Tickets remain process-local; Intents and saved UI state carry only an opaque ID. */
object DirectedDelivery {
    private var foregroundOwner: Any? = null
    private var listening = false
    private var incoming: JSONObject? = null
    private var submitted = mutableSetOf<Int>()
    private var submittedGeneration = 0L
    @Volatile var request: WorkspaceSessionRequest? = null
        private set

    @Synchronized fun listen(context: Context, owner: Any) {
        if (!listening) {
            val directory = File(context.filesDir, "received").apply { mkdirs() }
            PublisherClient.action(20, JSONObject().put("directory", directory.absolutePath).toString())
            listening = true
        }
        foregroundOwner = owner
    }

    @Synchronized fun leave(owner: Any) {
        if (foregroundOwner !== owner) return
        foregroundOwner = null
        stopIdleListener()
    }

    /** Network changes are allowed while idle, without retaining an old endpoint. */
    @Synchronized fun pauseForNetworkChange() {
        check(incoming == null) { "Stop the incoming screen before changing networks" }
        if (listening) PublisherClient.action(21)
        listening = false
    }

    private fun stopIdleListener() {
        if (foregroundOwner == null && incoming == null && listening) {
            PublisherClient.action(21)
            listening = false
        }
    }

    /** Called only while the Activity is foreground; never launches it from background. */
    @Synchronized fun poll(busy: Boolean): WorkspaceSessionRequest? {
        incoming?.optString("id")?.takeIf { !alive(it) }?.let { finish(it, "Sending stopped") }
        val raw = PublisherClient.action(16)
        if (raw == "null") return null
        val offer = JSONObject(raw)
        val id = offer.getString("id")
        val sources = offer.getJSONArray("sources")
        val device = WeaveloClient.devices.value.firstOrNull {
            it.id == offer.getString("source_device_id") && it.canOpenWorkspace
        }
        if (busy || incoming != null || sources.length() !in 1..4 || device == null) {
            update(id, "Rejected", if (busy || incoming != null) "Phone is busy with another connection"
                else if (device == null) "Source device is no longer reachable" else "Phone supports up to four incoming sources")
            return null
        }
        update(id, "Accepted")
        incoming = offer
        submitted.clear()
        submittedGeneration = 0
        return WorkspaceSessionRequest(device.endpoint, device.id, java.util.UUID.randomUUID().toString(), id)
            .also { request = it }
    }

    @Synchronized fun alive(id: String): Boolean = incoming?.optString("id") == id &&
        runCatching { PublisherClient.action(18, id) == "true" }.getOrDefault(false)

    @Synchronized fun claim(id: String): JSONObject {
        check(alive(id)) { "Sending intent ended" }
        return JSONObject(incoming!!.getJSONObject("claim").toString())
    }

    @Synchronized fun sources(id: String): List<String> {
        if (!alive(id)) return emptyList()
        val sources = incoming!!.getJSONArray("sources")
        return (0 until sources.length()).map { sources.get(it).toString() }
    }

    @Synchronized fun rendered(id: String, generation: Long, pane: Int, panes: List<WorkspacePane>) {
        if (!alive(id)) return
        if (submittedGeneration != generation) { submitted.clear(); submittedGeneration = generation }
        submitted.add(pane)
        val expected = sources(id)
        if (expected.isNotEmpty() && expected.all { source -> panes.any {
                it.source.source.toString() == source && it.accepted && it.id in submitted
            } }) update(id, "Receiving") // native Surface submission, not physical scanout
    }

    @Synchronized fun finish(id: String, reason: String = "") {
        if (incoming?.optString("id") != id) return
        runCatching { update(id, "Cancelled", reason) }
        incoming = null
        request = null
        submitted.clear()
        stopIdleListener()
    }

    private fun update(id: String, state: String, reason: String = "") {
        PublisherClient.action(17, JSONObject().put("id", id).put("state", state).put("reason", reason).toString())
    }
}
