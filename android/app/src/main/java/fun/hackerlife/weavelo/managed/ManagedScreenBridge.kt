package `fun`.hackerlife.weavelo.managed

import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.net.Uri
import android.os.Binder
import android.os.Bundle
import android.os.IBinder
import android.os.Process
import `fun`.hackerlife.weavelo.PublisherClient
import org.json.JSONObject

internal object ManagedScreenBridge {
    private var current: IManagedScreen? = null
    private var clientLife = Binder()
    @Synchronized fun attached(): IManagedScreen? = current?.takeIf { it.asBinder().isBinderAlive }
    @Synchronized fun attach(context: Context, binder: IBinder): IBinder {
        if (current?.asBinder() === binder) return clientLife
        check(attached() == null) { "Weavelo capture helper is already attached" }
        val service = checkNotNull(IManagedScreen.Stub.asInterface(binder))
        val info = JSONObject(service.info())
        check(info.getInt("api") == 4) { "Unsupported Weavelo capture helper" }
        clientLife = Binder()
        current = service
        binder.linkToDeath({
            synchronized(this) {
                if (current?.asBinder() === binder) {
                    current = null
                    PublisherClient.stop(context, disableManaged = false)
                    PublisherClient.status("Managed screen disconnected", false)
                }
            }
        }, 0)
        // Attaching this provider requires an already authorized shell caller.
        // No MediaProjection token or system setting is fabricated here.
        val preferences = context.getSharedPreferences("managed_screen", Context.MODE_PRIVATE)
        if (!preferences.contains("enabled")) preferences.edit().putBoolean("enabled", true).apply()
        resume(context)
        return clientLife
    }
    fun enable(context: Context, enabled: Boolean) {
        context.getSharedPreferences("managed_screen", Context.MODE_PRIVATE).edit().putBoolean("enabled", enabled).apply()
    }
    fun resume(context: Context) {
        if (attached() == null || PublisherClient.active ||
            !context.getSharedPreferences("managed_screen", Context.MODE_PRIVATE).getBoolean("enabled", false)) return
        runCatching { PublisherClient.startManaged(context) }
            .onFailure { PublisherClient.status("Open Weavelo to resume managed screen sharing", false) }
    }
}

/** Only the authorized local shell can attach; no exported control methods. */
class ManagedScreenProvider : ContentProvider() {
    override fun onCreate() = true
    override fun call(method: String, arg: String?, extras: Bundle?): Bundle {
        if (Binder.getCallingUid() != 2000) throw SecurityException("Authorized shell required")
        if (method == "status") {
            val identity = Binder.clearCallingIdentity()
            return try { Bundle().apply { putString("status", ManagedScreenBridge.attached()?.info() ?: "unavailable") } }
            finally { Binder.restoreCallingIdentity(identity) }
        }
        require(method == "attach" && arg == Process.myUid().toString())
        val service = checkNotNull(extras?.getBinder("service"))
        // Outbound Binder calls must carry Weavelo's own UID, not the inbound
        // shell identity. The helper independently checks that exact UID.
        val identity = Binder.clearCallingIdentity()
        return try { Bundle().apply { putBinder("client", ManagedScreenBridge.attach(checkNotNull(context), service)) } }
        finally { Binder.restoreCallingIdentity(identity) }
    }
    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, args: Array<out String>?, order: String?): Cursor? = throw UnsupportedOperationException()
    override fun getType(uri: Uri): String? = null
    override fun insert(uri: Uri, values: ContentValues?): Uri? = throw UnsupportedOperationException()
    override fun delete(uri: Uri, selection: String?, args: Array<out String>?): Int = throw UnsupportedOperationException()
    override fun update(uri: Uri, values: ContentValues?, selection: String?, args: Array<out String>?): Int = throw UnsupportedOperationException()
}
