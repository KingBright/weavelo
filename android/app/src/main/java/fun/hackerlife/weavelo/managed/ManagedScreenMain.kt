package `fun`.hackerlife.weavelo.managed

import android.net.Uri
import android.os.Bundle
import android.os.Looper
import android.os.Process
import `fun`.hackerlife.weavelo.BuildConfig

/** Launched from this installed APK by an already authorized local ADB shell. */
object ManagedScreenMain {
    @JvmStatic fun main(args: Array<String>) {
        check(args.isEmpty() && Process.myUid() == 2000) { "Authorized local shell required" }
        check(android.os.Build.VERSION.SDK_INT >= 34) { "Automatic capture requires Android 14 or later" }
        Looper.prepareMainLooper()
        val context = ManagedShellContext.create()
        val uid = context.packageManager.getApplicationInfo(BuildConfig.APPLICATION_ID, 0).uid
        check(uid >= 10_000)
        val host = ManagedScreenHost(uid)
        val result = context.contentResolver.call(Uri.parse("content://${BuildConfig.APPLICATION_ID}.managed"),
            "attach", uid.toString(), Bundle().apply { putBinder("service", host) })
        val client = checkNotNull(result?.getBinder("client")) { "Weavelo attachment failed" }
        client.linkToDeath({ host.clientDied() }, 0)
        println("WEAVELO_SCREEN_ATTACHED version=${BuildConfig.VERSION_CODE}")
        Looper.loop()
    }
}
