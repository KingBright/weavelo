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

/** Keeps an explicitly connected remote-device session alive while media is paused. */
class SessionConnectionService : Service() {
    private var languageObserver: LanguageNotificationObserver? = null
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val workspaceOwner = Any()
    private var disconnectRequested = false
    @Volatile private var latestStartId = 0

    companion object {
        private const val OPEN_WORKSPACE = "open-workspace"
        private const val CLOSE_WORKSPACE = "close-workspace"
        fun openWorkspace(context: android.content.Context, request: WorkspaceSessionRequest, forceReconnect: Boolean = false) {
            context.startForegroundService(Intent(context, SessionConnectionService::class.java)
                .setAction(OPEN_WORKSPACE).putExtra("endpoint", request.endpoint)
                .putExtra("device", request.deviceId).putExtra("session", request.sessionKey)
                .putExtra("incoming", request.incomingId).putExtra("open-main-display", request.openMainDisplay)
                .putExtra("force-reconnect", forceReconnect))
        }
        fun closeWorkspace(context: android.content.Context, sessionKey: String) {
            context.startForegroundService(Intent(context, SessionConnectionService::class.java).setAction(CLOSE_WORKSPACE).putExtra("session", sessionKey))
        }
    }

    private fun notification(): Notification {
        val manager = getSystemService(NotificationManager::class.java)
        val channel = "remote-session"
        manager.createNotificationChannel(NotificationChannel(channel, notificationText("Remote connection"), NotificationManager.IMPORTANCE_LOW))
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        val stop = PendingIntent.getService(this, 1, Intent(this, SessionConnectionService::class.java).setAction("disconnect"), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        return Notification.Builder(this, channel)
            .setSmallIcon(android.R.drawable.stat_sys_upload_done)
            .setContentTitle(notificationText("Weavelo connection"))
            .setContentText(notificationText("Background media pauses automatically. Tap to return."))
            .setContentIntent(open).setOngoing(true)
            .addAction(Notification.Action.Builder(null, notificationText("Disconnect"), stop).build())
            .build()
    }

    override fun onCreate() {
        super.onCreate()
        startForeground(8123, notification())
        languageObserver = LanguageNotificationObserver(this) { getSystemService(NotificationManager::class.java).notify(8123, notification()) }
        scope.launch {
            delay(1000) // onStartCommand must first deliver the user connection intent.
            while (isActive) {
                if (WeaveloClient.backgrounded) {
                    if (!WeaveloClient.maintainBackgroundConnection()) WeaveloClient.pollTelemetry()
                }
                val stopped = withContext(Dispatchers.Main.immediate) {
                    // Serialize stop with onStartCommand: never consume a newer
                    // user start while its native open is still queued.
                    if (!WorkspaceClient.starting && !WorkspaceClient.retainingConnection &&
                        WeaveloClient.sessionState.value in listOf(SessionState.DISCONNECTED, SessionState.ERROR))
                        stopSelfResult(latestStartId) else false
                }
                if (stopped) break
                delay(if (WeaveloClient.backgrounded) 5000 else 1000)
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        latestStartId = startId
        when (intent?.action) {
            OPEN_WORKSPACE -> {
                val endpoint = intent.getStringExtra("endpoint")
                val device = intent.getStringExtra("device")
                val session = intent.getStringExtra("session")
                if (!endpoint.isNullOrBlank() && device != null && !session.isNullOrBlank()) {
                    WorkspaceClient.openFromService(applicationContext, scope, workspaceOwner,
                        WorkspaceSessionRequest(endpoint, device, session, intent.getStringExtra("incoming"),
                            intent.getBooleanExtra("open-main-display", false)), intent.getBooleanExtra("force-reconnect", false))
                }
            }
            CLOSE_WORKSPACE -> intent.getStringExtra("session")?.let { WorkspaceClient.disconnectSession(it) }
            "disconnect" -> {
                disconnectRequested = true
                WeaveloClient.disconnect()
                WorkspaceClient.requestedSession()?.let { WorkspaceClient.disconnectSession(it.sessionKey) }
                stopSelfResult(startId)
            }
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        languageObserver?.close()
        scope.cancel()
        WorkspaceClient.closeFromService(workspaceOwner, preserveForForeground = !disconnectRequested)
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun dump(fd: java.io.FileDescriptor, writer: java.io.PrintWriter, args: Array<out String>?) {
        val state = WorkspaceClient.state.value
        // Read-only scalar correlation for native host diagnostics. No endpoint,
        // ticket, session key, source title, clipboard, text or file payload.
        writer.println("Weavelo workspace diagnostics")
        writer.println(org.json.JSONObject().put("connection_id", state.connectionId)
            .put("generation", state.connectionGeneration).put("connected", state.connected)
            .put("incoming_ended", state.incomingEnded).put("retaining_connection", WorkspaceClient.retainingConnection)
            .put("panes", state.panes.size).put("accepted_panes", state.panes.count { it.accepted }))
    }
}
