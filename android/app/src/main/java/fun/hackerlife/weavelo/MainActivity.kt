package `fun`.hackerlife.weavelo

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.repeatOnLifecycle
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import `fun`.hackerlife.weavelo.ui.MobileConsoleScreen
import `fun`.hackerlife.weavelo.ui.MobileViewportScreen
import `fun`.hackerlife.weavelo.ui.QrScanScreen
import kotlinx.coroutines.delay
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class MainActivity : ComponentActivity() {
    private val notificationPermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { _ ->
        // Consent remains the user's choice; foreground-service startup does not
        // depend on granting notification-drawer visibility.
    }

    private fun requestNotificationPermissionOnce() {
        if (Build.VERSION.SDK_INT < 33 ||
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) return
        val preferences = getSharedPreferences("permissions", MODE_PRIVATE)
        if (preferences.getBoolean("notification_requested", false)) return
        preferences.edit().putBoolean("notification_requested", true).apply()
        notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
    }

    override fun onStart() {
        super.onStart()
        WeaveloClient.setBackgrounded(false)
        WorkspaceClient.resumeOnForeground(this)
        WorkspaceClient.backgrounded(false)
    }

    override fun onStop() {
        WeaveloClient.setBackgrounded(true)
        WorkspaceClient.backgrounded(true)
        super.onStop()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        requestNotificationPermissionOnce()
        WeaveloClient.initialize(applicationContext)

        setContent {
            `fun`.hackerlife.weavelo.ui.WeaveloTheme {
            val scope = rememberCoroutineScope()
            val sessionState by WeaveloClient.sessionState.collectAsState()
            val telemetry by WeaveloClient.telemetry.collectAsState()
            val sessionError by WeaveloClient.sessionError.collectAsState()
            val devices by WeaveloClient.devices.collectAsState()
            val discoveryDiagnostics by WeaveloClient.discoveryDiagnostics.collectAsState()
            val appNetwork by WeaveloClient.appNetwork.collectAsState()
            val networkName by WeaveloClient.networkName.collectAsState()
            var joiningNetwork by remember { mutableStateOf(false) }
            var sendFileOnConnect by remember { mutableStateOf(false) }
            var workspaceDeviceId by remember { mutableStateOf(WorkspaceClient.requestedSession()?.deviceId ?: "") }
            var scanningQr by remember { mutableStateOf(false) }
            var manualInvitationRequested by remember { mutableStateOf(false) }
            var workspaceEndpoint by remember { mutableStateOf(WorkspaceClient.requestedSession()?.endpoint) }
            var incomingSession by remember { mutableStateOf(WorkspaceClient.requestedSession()?.takeIf { it.incomingId != null }) }
            var receivedFiles by remember { mutableStateOf(false) }
            var pairingMessage by remember { mutableStateOf<String?>(null) }
            val publisherStatus by PublisherClient.status.collectAsState()
            val outgoing by PublisherClient.outgoing.collectAsState()
            val incomingError by DirectedDelivery.error.collectAsState()
            val receiverState by FileReceiverClient.state.collectAsState()
            val controlReady by RemoteControlService.connected.collectAsState()
            fun joinNetwork(raw: String) {
                if (joiningNetwork) return
                if (WorkspaceClient.active || PublisherClient.active || FileReceiverClient.ownsNetwork ||
                    sessionState in listOf(SessionState.CONNECTING, SessionState.STREAMING, SessionState.RECONNECTING)) {
                    pairingMessage = "error:Disconnect active sessions and stop file reception and screen sharing before changing networks."
                    return
                }
                joiningNetwork = true
                pairingMessage = "Joining device network..."
                scope.launch {
                    try {
                        pairingMessage = withContext(Dispatchers.IO) {
                            DirectedDelivery.pauseForNetworkChange()
                            WeaveloClient.joinPairingPayload(raw)
                        }
                    } catch (cancelled: kotlinx.coroutines.CancellationException) {
                        throw cancelled
                    } catch (error: Exception) {
                        pairingMessage = "error:${error.message ?: "Could not join network"}"
                    } finally {
                        joiningNetwork = false
                    }
                }
            }
            var publishAudio by remember { mutableStateOf(false) }
            var publishControl by remember { mutableStateOf(false) }
            var pendingSendTarget by androidx.compose.runtime.saveable.rememberSaveable { mutableStateOf<String?>(null) }
            val projection = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
                val target = pendingSendTarget
                pendingSendTarget = null
                if (result.resultCode == RESULT_OK && result.data != null) PublisherClient.start(this, result.resultCode, result.data!!, publishAudio, publishControl, target)
                else PublisherClient.status("Screen sharing was not started. Confirm Android screen capture to make this phone viewable.", false)
            }
            fun requestProjection(audio: Boolean, control: Boolean = false) {
                publishAudio = audio; publishControl = control
                val manager = getSystemService(android.media.projection.MediaProjectionManager::class.java)
                // A separate user action requests full-display control; existing viewing keeps the OS source choice.
                val intent = if (control && android.os.Build.VERSION.SDK_INT >= 34)
                    manager.createScreenCaptureIntent(android.media.projection.MediaProjectionConfig.createConfigForDefaultDisplay())
                    else manager.createScreenCaptureIntent()
                projection.launch(intent)
            }
            val audioPermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted -> requestProjection(granted,publishControl) }

            LaunchedEffect(Unit) {
                lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
                    val receiverOwner = Any()
                    try {
                        while (true) {
                            val incoming = withContext(Dispatchers.IO) {
                                WeaveloClient.refreshDevices()
                                WeaveloClient.pollTelemetry()
                                if (!WeaveloClient.nativeAvailable || joiningNetwork) null
                                else runCatching {
                                    PublisherClient.refreshOutgoing()
                                    if (devices.isEmpty()) return@runCatching null
                                    DirectedDelivery.listen(applicationContext, receiverOwner)
                                    DirectedDelivery.poll(workspaceEndpoint != null || scanningQr ||
                                        WorkspaceClient.starting || WorkspaceClient.active ||
                                        sessionState in listOf(SessionState.CONNECTING, SessionState.STREAMING, SessionState.RECONNECTING))
                                }.onSuccess { DirectedDelivery.failed(null) }
                                    .onFailure { DirectedDelivery.failed(it.message ?: "Could not receive screens") }.getOrNull()
                            }
                            if (incoming != null) {
                                incomingSession = incoming
                                sendFileOnConnect = false
                                workspaceDeviceId = incoming.deviceId
                                workspaceEndpoint = incoming.endpoint
                            }
                            delay(1000)
                        }
                    } finally {
                        withContext(kotlinx.coroutines.NonCancellable + Dispatchers.IO) { DirectedDelivery.leave(receiverOwner) }
                    }
                }
            }

            when {
                workspaceEndpoint != null -> {
                    `fun`.hackerlife.weavelo.ui.WorkspaceScreen(workspaceEndpoint!!, sendFileOnConnect, workspaceDeviceId, incomingSession) {
                        workspaceEndpoint = null; incomingSession = null
                    }
                }
                scanningQr -> {
                    QrScanScreen(
                        onResult = { raw ->
                            scanningQr = false
                            joinNetwork(raw)
                        },
                        onCancel = { scanningQr = false },
                        onManualInput = { scanningQr = false; manualInvitationRequested = true }
                    )
                }
                sessionState == SessionState.STREAMING || sessionState == SessionState.RECONNECTING -> {
                    MobileViewportScreen(
                        onDisconnect = {
                            scope.launch(Dispatchers.IO) { WeaveloClient.disconnect() }
                        }
                    )
                }
                else -> {
                    MobileConsoleScreen(
                        telemetry = telemetry,
                        devices = devices,
                        discoveryDiagnostics = discoveryDiagnostics,
                        appNetwork = appNetwork,
                        nativeAvailable = WeaveloClient.nativeAvailable && !joiningNetwork,
                        networkName = networkName,
                        joiningNetwork = joiningNetwork,
                        pairingMessage = listOfNotNull(WeaveloClient.nativeStartupError.takeIf { it.isNotBlank() }?.let { "error:Native startup failed ($it). Existing data is retained." }, pairingMessage, incomingError, publisherStatus.takeIf { it.isNotBlank() }, receiverState.message.takeIf { it.isNotBlank() }).distinct().joinToString("\n"),
                        sessionError = sessionError,
                        sessionState = sessionState,
                        onCancelConnect = {
                            scope.launch(Dispatchers.IO) { WeaveloClient.disconnect() }
                        },
                        onConnectHost = { deviceId, endpoint ->
                            scope.launch(Dispatchers.IO) { WeaveloClient.connect(deviceId, endpoint) }
                        },
                        onConnectWorkspace = { deviceId, endpoint -> incomingSession = null; sendFileOnConnect = false; workspaceDeviceId = deviceId; workspaceEndpoint = endpoint },
                        onSendFiles = { deviceId, endpoint -> incomingSession = null; sendFileOnConnect = true; workspaceDeviceId = deviceId; workspaceEndpoint = endpoint },
                        outgoing = outgoing,
                        onSendScreen = { deviceId ->
                            if (PublisherClient.active) scope.launch(Dispatchers.IO) {
                                runCatching { PublisherClient.sendScreen(deviceId) }.onFailure { PublisherClient.status(it.message ?: "Could not send screen") }
                            } else {
                                pendingSendTarget = deviceId
                                requestProjection(false)
                            }
                        },
                        onStopSending = { id -> scope.launch(Dispatchers.IO) {
                            runCatching { PublisherClient.stopSending(id) }.onFailure { PublisherClient.status(it.message ?: "Could not stop sending") }
                        } },
                        publishing = PublisherClient.active,
                        receivingFiles = receiverState.active || receiverState.starting,
                        receiverStarting = receiverState.starting,
                        onStartReceiving = { FileReceiverClient.start(this) },
                        onStopReceiving = { FileReceiverClient.stop(this) },
                        controlReady = controlReady,
                        onControlSettings = { RemoteControlService.openSettings(this) },
                        onShareControlledScreen = {
                            pendingSendTarget = null
                            publishControl = true
                            if (checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) != android.content.pm.PackageManager.PERMISSION_GRANTED) audioPermission.launch(android.Manifest.permission.RECORD_AUDIO)
                            else requestProjection(true,true)
                        },
                        onShareScreen = { audio ->
                            pendingSendTarget = null
                            publishControl = false
                            if (audio && checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) != android.content.pm.PackageManager.PERMISSION_GRANTED) audioPermission.launch(android.Manifest.permission.RECORD_AUDIO)
                            else requestProjection(audio)
                        },
                        onStopSharing = { PublisherClient.stop(this) },
                        onBrowseFiles = { receivedFiles = true },
                        onJoinCode = ::joinNetwork,
                        showInvitationInput = manualInvitationRequested,
                        onInvitationInputShown = { manualInvitationRequested = false },
                        onScanQr = { scanningQr = true }
                    )
                }
            }
            if (receivedFiles) `fun`.hackerlife.weavelo.ui.ReceivedFilesDialog { receivedFiles = false }
            }
        }
    }
}
