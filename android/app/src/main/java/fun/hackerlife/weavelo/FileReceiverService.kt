package `fun`.hackerlife.weavelo

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import `fun`.hackerlife.weavelo.ui.LanguageNotificationObserver
import `fun`.hackerlife.weavelo.ui.notificationText
import android.content.Intent
import android.os.IBinder
import kotlinx.coroutines.*

/** User-started, visible file receiver; no MediaProjection or microphone access. */
class FileReceiverService : Service() {
    private var languageObserver: LanguageNotificationObserver? = null
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val token = Any()
    private var worker: Job? = null

    private fun notification(): Notification {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel("file-receiver", notificationText("Receive files"), NotificationManager.IMPORTANCE_LOW))
        val open = PendingIntent.getActivity(this, 5, Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val stop = PendingIntent.getService(this, 6, Intent(this, FileReceiverService::class.java).setAction("stop"),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        return Notification.Builder(this, "file-receiver")
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle(notificationText("Weavelo file reception"))
            .setContentText(notificationText("Device-network members can send files. No screen permission is granted."))
            .setContentIntent(open).setOngoing(true)
            .addAction(Notification.Action.Builder(null, notificationText("Stop standalone receiver"), stop).build()).build()
    }
    override fun onCreate() {
        super.onCreate()
        try {
            startForeground(8128, notification())
        } catch (error: RuntimeException) {
            android.util.Log.w("WeaveloFileReceiver", "Foreground start declined in onCreate", error)
            stopSelf()
            return
        }
        languageObserver = LanguageNotificationObserver(this) { getSystemService(NotificationManager::class.java).notify(8128, notification()) }
    }
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == "stop") { stopSelf(); return START_NOT_STICKY }
        if (worker == null) worker = scope.launch {
            var failure: String? = null
            var failureText: AppText? = null
            try {
                FileReceiverClient.acquire(applicationContext, token)
                while (isActive) { delay(2000); FileReceiverClient.checkHealth(token) }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (error: Exception) {
                failure = error.message ?: "File reception failed"
                failureText = appTextOf(error) ?: if (error.message == null) AppText("File reception failed") else null
            } finally {
                withContext(NonCancellable) { FileReceiverClient.release(token, failure, failureText) }
                stopSelf()
            }
        }
        return START_NOT_STICKY
    }
    override fun onDestroy() { languageObserver?.close(); scope.cancel(); super.onDestroy() }
    override fun onBind(intent: Intent?): IBinder? = null
}
