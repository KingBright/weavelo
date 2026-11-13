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
    @Synchronized fun attach(context: Context, binder: IBinder, forceEnable: Boolean = false): IBinder {
        if (current?.asBinder() === binder) return clientLife
        check(attached() == null) { "Weavelo capture helper is already attached" }
        val service = checkNotNull(IManagedScreen.Stub.asInterface(binder))
        val info = JSONObject(service.info())
        check(info.getInt("api") == 5) { "Unsupported Weavelo capture helper" }
        clientLife = Binder()
        current = service
        val appContext = context.applicationContext
        binder.linkToDeath({
            synchronized(this) {
                if (current?.asBinder() === binder) {
                    current = null
                    PublisherClient.stop(appContext, disableManaged = false)
                    PublisherClient.status("Managed screen disconnected", false, `fun`.hackerlife.weavelo.AppText("Managed screen disconnected"))
                }
            }
        }, 0)
        val preferences = appContext.getSharedPreferences("managed_screen", Context.MODE_PRIVATE)
        if (forceEnable || !preferences.contains("enabled")) {
            preferences.edit().putBoolean("enabled", true).apply()
        }
        `fun`.hackerlife.weavelo.WeaveloClient.initialize(appContext)
        `fun`.hackerlife.weavelo.DevicePresenceService.restoreBackgroundServices(appContext)
        resume(appContext)
        return clientLife
    }
    fun enable(context: Context, enabled: Boolean) {
        context.getSharedPreferences("managed_screen", Context.MODE_PRIVATE).edit().putBoolean("enabled", enabled).apply()
    }
    fun resume(context: Context) {
        val appContext = context.applicationContext
        if (attached() == null || PublisherClient.active ||
            !appContext.getSharedPreferences("managed_screen", Context.MODE_PRIVATE).getBoolean("enabled", false)) return
        if (!`fun`.hackerlife.weavelo.WeaveloClient.initialize(appContext)) return
        runCatching { PublisherClient.startManaged(appContext) }
            .onFailure { PublisherClient.status("Open Weavelo to resume managed screen sharing", false, `fun`.hackerlife.weavelo.AppText("Open Weavelo to resume managed screen sharing")) }
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
        if (method == "resume") {
            val prev = Binder.clearCallingIdentity()
            return try {
                val appContext = checkNotNull(context).applicationContext
                ManagedScreenBridge.enable(appContext, true)
                `fun`.hackerlife.weavelo.WeaveloClient.initialize(appContext)
                `fun`.hackerlife.weavelo.DevicePresenceService.restoreBackgroundServices(appContext)
                ManagedScreenBridge.resume(appContext)
                Bundle().apply {
                    putString("status", ManagedScreenBridge.attached()?.info() ?: "unavailable")
                    putBoolean("active", PublisherClient.active)
                }
            } finally { Binder.restoreCallingIdentity(prev) }
        }
        require(method == "attach" && arg == Process.myUid().toString())
        val service = checkNotNull(extras?.getBinder("service"))
        val forceEnable = extras?.getBoolean("enable", false) == true
        val identity = Binder.clearCallingIdentity()
        return try { Bundle().apply { putBinder("client", ManagedScreenBridge.attach(checkNotNull(context), service, forceEnable)) } }
        finally { Binder.restoreCallingIdentity(identity) }
    }
    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, args: Array<out String>?, order: String?): Cursor? = throw UnsupportedOperationException()
    override fun getType(uri: Uri): String? = null
    override fun insert(uri: Uri, values: ContentValues?): Uri? = throw UnsupportedOperationException()
    override fun delete(uri: Uri, selection: String?, args: Array<out String>?): Int = throw UnsupportedOperationException()
    override fun update(uri: Uri, values: ContentValues?, selection: String?, args: Array<out String>?): Int = throw UnsupportedOperationException()
}
