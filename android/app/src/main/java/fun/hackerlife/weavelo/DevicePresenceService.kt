package `fun`.hackerlife.weavelo

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.net.wifi.WifiManager
import android.os.IBinder
import android.util.Log
import java.io.File
import `fun`.hackerlife.weavelo.ui.LanguageNotificationObserver
import `fun`.hackerlife.weavelo.ui.notificationText
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Owns the paired network listener independently of the Activity and media sessions. */
class DevicePresenceService : Service() {
    companion object {
        private const val CHANNEL = "device-presence"
        private const val NOTIFICATION_ID = 8130
        private const val STOP = "stop-device-presence"
        internal const val PREFERENCE = "keep_device_online"

        internal fun enabled(context: Context): Boolean = context
            .getSharedPreferences("stream", Context.MODE_PRIVATE).getBoolean(PREFERENCE, true)

        internal fun setEnabled(context: Context, enabled: Boolean) {
            context.getSharedPreferences("stream", Context.MODE_PRIVATE).edit().putBoolean(PREFERENCE, enabled).apply()
            if (enabled) startIfEnabled(context) else context.stopService(Intent(context, DevicePresenceService::class.java))
        }

        internal fun startIfEnabled(context: Context) {
            if (!enabled(context) || !WeaveloClient.nativeAvailable) return
            try {
                context.startForegroundService(Intent(context, DevicePresenceService::class.java))
            } catch (error: RuntimeException) {
                Log.w("WeaveloPresence", "Android declined the background connection service", error)
            }
        }

        internal fun restoreBackgroundServices(context: Context) {
            val app = context.applicationContext
            if (!File(app.filesDir, "mesh.conf").exists()) return
            startIfEnabled(app)
            val autoReceive = app.getSharedPreferences("stream", Context.MODE_PRIVATE)
                .getBoolean("auto_receive_files", true)
            if (autoReceive && WeaveloClient.nativeAvailable && !FileReceiverClient.ownsNetwork) {
                FileReceiverClient.start(app)
            }
            `fun`.hackerlife.weavelo.managed.ManagedScreenBridge.resume(app)
        }
    }

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val owner = Any()
    private var worker: Job? = null
    private var languageObserver: LanguageNotificationObserver? = null
    private var multicastLock: WifiManager.MulticastLock? = null
    private var message = "Starting background connection…"

    private fun notification(): Notification {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(CHANNEL, notificationText("Background connection"), NotificationManager.IMPORTANCE_LOW))
        val open = PendingIntent.getActivity(this, 7, Intent(this, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP), PendingIntent.FLAG_IMMUTABLE)
        val stop = PendingIntent.getService(this, 8, Intent(this, DevicePresenceService::class.java).setAction(STOP), PendingIntent.FLAG_IMMUTABLE)
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_upload_done)
            .setContentTitle(notificationText("Weavelo background connection"))
            .setContentText(notificationText(message)).setContentIntent(open).setOngoing(true).setOnlyAlertOnce(true)
            .addAction(Notification.Action.Builder(null, notificationText("Stop background connection"), stop).build())
            .build()
    }

    private fun refreshNotification() {
        getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, notification())
    }

    override fun onCreate() {
        super.onCreate()
        try {
            startForeground(NOTIFICATION_ID, notification())
        } catch (error: RuntimeException) {
            Log.w("WeaveloPresence", "Foreground start declined in onCreate", error)
            stopSelf()
            return
        }
        runCatching {
            getSystemService(WifiManager::class.java)
                ?.createMulticastLock("weavelo-presence-lan")
                ?.apply { setReferenceCounted(false); acquire() }
                ?.also { multicastLock = it }
        }
        languageObserver = LanguageNotificationObserver(this, ::refreshNotification)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == STOP || !enabled(this)) {
            if (intent?.action == STOP) {
                getSharedPreferences("stream", MODE_PRIVATE).edit().putBoolean(PREFERENCE, false).apply()
            }
            stopSelfResult(startId)
            return START_NOT_STICKY
        }
        if (worker == null) worker = scope.launch {
            try {
                while (isActive) {
                    val next = try {
                        check(WeaveloClient.initialize(applicationContext))
                        DirectedDelivery.listen(applicationContext, owner, checkHealth = true)
                        val autoReceive = getSharedPreferences("stream", MODE_PRIVATE)
                            .getBoolean("auto_receive_files", true)
                        if (autoReceive && !FileReceiverClient.ownsNetwork &&
                            File(applicationContext.filesDir, "mesh.conf").exists()
                        ) {
                            FileReceiverClient.start(applicationContext)
                        }
                        `fun`.hackerlife.weavelo.managed.ManagedScreenBridge.resume(applicationContext)
                        if (WeaveloClient.backgrounded && DirectedDelivery.hasPendingOffer()) "Incoming screen. Tap to open Weavelo."
                        else "Paired devices can connect while Weavelo is in the background."
                    } catch (cancelled: CancellationException) {
                        throw cancelled
                    } catch (_: Exception) {
                        "Background connection is retrying…"
                    }
                    withContext(Dispatchers.Main.immediate) {
                        if (message != next) {
                            message = next
                            refreshNotification()
                        }
                    }
                    delay(2000)
                }
            } finally {
                runCatching { DirectedDelivery.leave(owner) }
            }
        }
        return START_STICKY
    }

    override fun onDestroy() {
        runCatching { multicastLock?.let { if (it.isHeld) it.release() } }
        multicastLock = null
        languageObserver?.close()
        scope.cancel()
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null
}

class PresenceStartupReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent?) {
        when (intent?.action) {
            Intent.ACTION_MY_PACKAGE_REPLACED,
            Intent.ACTION_BOOT_COMPLETED -> DevicePresenceService.restoreBackgroundServices(context)
        }
    }
}
