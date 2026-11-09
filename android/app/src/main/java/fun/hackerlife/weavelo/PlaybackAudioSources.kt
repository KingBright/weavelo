package `fun`.hackerlife.weavelo

import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Process
import android.os.UserHandle

data class PlaybackAudioTarget(val packageName: String, val uid: Int, val updated: Long)
internal data class PlaybackAudioChoice(val label: String, val target: PlaybackAudioTarget)

internal fun safePlaybackAudioIdentity(packageName: String, ownPackage: String, enabled: Boolean,
    sameProfile: Boolean, sharedUserId: String?, uidPackages: Array<String>?): Boolean =
    packageName != ownPackage && enabled && sameProfile && sharedUserId == null &&
        uidPackages?.toSet() == setOf(packageName)

/** Public package APIs only. A declared shared UID is rejected even when its
 * other packages are hidden by Android's package-visibility filtering. */
internal object PlaybackAudioSources {
    @Suppress("DEPRECATION")
    fun binding(context: Context, packageName: String): PlaybackAudioTarget? = runCatching {
        val pm = context.packageManager
        val info = pm.getPackageInfo(packageName, 0)
        val app = checkNotNull(info.applicationInfo)
        val uid = app.uid
        check(safePlaybackAudioIdentity(packageName, context.packageName, app.enabled,
            UserHandle.getUserHandleForUid(uid) == UserHandle.getUserHandleForUid(Process.myUid()),
            info.sharedUserId, pm.getPackagesForUid(uid)))
        PlaybackAudioTarget(packageName, uid, info.lastUpdateTime)
    }.getOrNull()

    @Suppress("DEPRECATION")
    fun choices(context: Context): List<PlaybackAudioChoice> {
        val pm = context.packageManager
        val launcher = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
        return pm.queryIntentActivities(launcher, 0).mapNotNull { entry ->
            val name = entry.activityInfo?.packageName ?: return@mapNotNull null
            val target = binding(context, name) ?: return@mapNotNull null
            PlaybackAudioChoice(entry.loadLabel(pm).toString(), target)
        }.distinctBy { it.target.packageName }.sortedBy { it.label.lowercase() }
    }

    fun matches(context: Context, target: PlaybackAudioTarget): Boolean = binding(context, target.packageName) == target
}
