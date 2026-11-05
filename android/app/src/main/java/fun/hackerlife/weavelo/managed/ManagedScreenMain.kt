package `fun`.hackerlife.weavelo.managed

import android.content.AttributionSource
import android.content.pm.ProviderInfo
import android.os.Binder
import android.os.Bundle
import android.os.IBinder
import android.os.Looper
import android.os.Process
import `fun`.hackerlife.weavelo.BuildConfig

/** Launched from this installed APK by an already authorized local ADB shell. */
@android.annotation.TargetApi(34)
object ManagedScreenMain {
    @JvmStatic fun main(args: Array<String>) {
        try { run(args) } catch (error: Throwable) {
            error.printStackTrace()
            kotlin.system.exitProcess(1)
        }
    }
    private fun run(args: Array<String>) {
        check(args.isEmpty() && Process.myUid() == 2000) { "Authorized local shell required" }
        check(android.os.Build.VERSION.SDK_INT >= 34) { "Automatic capture requires Android 14 or later" }
        Looper.prepareMainLooper()
        val context = ManagedShellContext.create()
        val uid = context.packageManager.getApplicationInfo(BuildConfig.APPLICATION_ID, 0).uid
        check(uid >= 10_000)
        val host = ManagedScreenHost(uid)
        val client = attach(uid, host)
        client.linkToDeath({ host.clientDied() }, 0)
        println("WEAVELO_SCREEN_ATTACHED version=${BuildConfig.VERSION_CODE}")
        Looper.loop()
    }
    private fun attach(uid: Int, host: IBinder): IBinder {
        // An app_process shell is not an ActivityManager-registered application.
        // Use Android's content shell command path, with the real caller UID.
        val authority = "${BuildConfig.APPLICATION_ID}.managed"
        val user = uid / 100_000
        val token = Binder()
        val type = Class.forName("android.app.IActivityManager")
        val manager = Class.forName("android.app.ActivityManager").getMethod("getService").invoke(null)
        var acquired = false
        try {
            val holder = checkNotNull(type.getMethod("getContentProviderExternal", String::class.java,
                Int::class.javaPrimitiveType, IBinder::class.java, String::class.java)
                .invoke(manager, authority, user, token, "weavelo"))
            acquired = true
            val info = holder.javaClass.getField("info").get(holder) as ProviderInfo
            check(info.applicationInfo.uid == uid && info.packageName == BuildConfig.APPLICATION_ID) { "Weavelo provider identity changed" }
            val provider = checkNotNull(holder.javaClass.getField("provider").get(holder))
            val result = Class.forName("android.content.IContentProvider").getMethod("call",
                AttributionSource::class.java, String::class.java, String::class.java, String::class.java, Bundle::class.java)
                .invoke(provider, AttributionSource.Builder(Process.myUid()).setPackageName("com.android.shell").build(),
                    authority, "attach", uid.toString(), Bundle().apply { putBinder("service", host) }) as? Bundle
            return checkNotNull(result?.getBinder("client")) { "Weavelo attachment failed" }
        } finally {
            if (acquired) runCatching {
                type.getMethod("removeContentProviderExternalAsUser", String::class.java, IBinder::class.java,
                    Int::class.javaPrimitiveType).invoke(manager, authority, token, user)
            }
        }
    }
}
