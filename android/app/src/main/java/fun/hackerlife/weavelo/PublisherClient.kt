package `fun`.hackerlife.weavelo

import android.content.Context
import android.content.Intent
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.json.JSONObject

data class OutgoingScreenState(val id: Long = 0, val active: Boolean = false, val target: String = "", val status: String = "", val peers: Map<String, Int> = emptyMap(), val unattributedSharing: Int = 0)

object PublisherClient {
    external fun nativeAction(operation: Int, payload: String): String
    const val SCREEN_SOURCE = "\"MainDisplay\""
    external fun nativeFrame(source: String, epoch: Long, bytes: ByteArray, ptsUs: Long): Boolean
    external fun nativeScreenFrame(metadata: String, bytes: ByteArray): Boolean
    external fun nativeClockMicros(): Long
    external fun nativePcm(grant: Long, samples: ShortArray): Boolean
    @Volatile var controlSize: Pair<Int,Int>? = null
        internal set
    private val mutable = MutableStateFlow("")
    val status = mutable.asStateFlow()
    private val mutableOutgoing = MutableStateFlow(OutgoingScreenState())
    val outgoing = mutableOutgoing.asStateFlow()
    @Volatile var active = false
        private set
    fun status(message: String, active: Boolean = this.active) { this.active = active; mutable.value = message }
    fun action(operation: Int, payload: String = ""): String {
        val body = if (operation in listOf(0, 5, 20, 30)) {
            org.json.JSONObject(payload).put("device_badge", DeviceArtwork.savedJson()).toString()
        } else payload
        val result = nativeAction(operation, body)
        if (result.startsWith("{\"error\"")) throw IllegalStateException(JSONObject(result).getString("error"))
        return result
    }
    fun refreshOutgoing() {
        val result = JSONObject(action(23))
        val peers = mutableMapOf<String, Int>()
        var unattributed = 0
        result.optJSONArray("peers")?.let { entries ->
            for (index in 0 until entries.length()) {
                val entry = entries.getJSONArray(index)
                if (entry.isNull(0)) unattributed += entry.getInt(1)
                else peers[entry.getString(0)] = entry.getInt(1)
            }
        }
        mutableOutgoing.value = OutgoingScreenState(result.optLong("id"), result.optBoolean("active"), result.optString("target"), result.optString("status"), peers, unattributed)
    }
    fun sendScreen(deviceId: String) {
        WeaveloClient.refreshDevices()
        val endpoint = resolveScreenDeliveryEndpoint(deviceId, WeaveloClient.devices.value)
        action(22, JSONObject().put("target", deviceId).put("endpoint", endpoint).toString())
        refreshOutgoing()
    }
    fun stopSending(id: Long) { action(24, id.toString()); refreshOutgoing() }
    fun start(context: Context, resultCode: Int, data: Intent, audio: Boolean, control: Boolean = false, sendTarget: String? = null) {
        context.startForegroundService(Intent(context, ProjectionService::class.java)
            .putExtra("result", resultCode).putExtra("consent", data).putExtra("audio", audio).putExtra("control", control)
            .putExtra("send-target", sendTarget))
    }
    internal fun startManaged(context: Context, control: Boolean = true, sendTarget: String? = null) {
        context.startForegroundService(Intent(context, ProjectionService::class.java)
            .setAction("managed").putExtra("control", control).putExtra("send-target", sendTarget))
    }
    fun stop(context: Context, disableManaged: Boolean = true) {
        if (disableManaged) `fun`.hackerlife.weavelo.managed.ManagedScreenBridge.enable(context, false)
        context.stopService(Intent(context, ProjectionService::class.java))
    }
}
