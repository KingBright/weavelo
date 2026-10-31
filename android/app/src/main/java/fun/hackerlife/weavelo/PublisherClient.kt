package `fun`.hackerlife.weavelo

import android.content.Context
import android.content.Intent
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.json.JSONObject

data class OutgoingScreenState(val id: Long = 0, val active: Boolean = false, val target: String = "", val status: String = "")

object PublisherClient {
    external fun nativeAction(operation: Int, payload: String): String
    external fun nativeFrame(bytes: ByteArray, ptsUs: Long)
    external fun nativePcm(samples: ShortArray): Boolean
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
        val result = nativeAction(operation, payload)
        if (result.startsWith("{\"error\"")) throw IllegalStateException(JSONObject(result).getString("error"))
        return result
    }
    fun refreshOutgoing() {
        val result = JSONObject(action(23))
        mutableOutgoing.value = OutgoingScreenState(result.optLong("id"), result.optBoolean("active"), result.optString("target"), result.optString("status"))
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
    fun stop(context: Context) { context.stopService(Intent(context, ProjectionService::class.java)) }
}
