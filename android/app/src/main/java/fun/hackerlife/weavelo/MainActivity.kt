package `fun`.hackerlife.weavelo

import `fun`.hackerlife.weavelo.ui.cameraNameText
import `fun`.hackerlife.weavelo.ui.appText
import `fun`.hackerlife.weavelo.ui.trFormat
import `fun`.hackerlife.weavelo.ui.tr
import `fun`.hackerlife.weavelo.ui.AppLanguageProvider
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
import androidx.compose.runtime.saveable.rememberSaveable
import `fun`.hackerlife.weavelo.ui.MobileConsoleScreen
import `fun`.hackerlife.weavelo.ui.MobileViewportScreen
import `fun`.hackerlife.weavelo.ui.QrScanScreen
import kotlinx.coroutines.delay
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

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
        DevicePresenceService.startIfEnabled(this)
        WorkspaceClient.resumeOnForeground(this)
        WorkspaceClient.backgrounded(false)
        `fun`.hackerlife.weavelo.managed.ManagedScreenBridge.resume(this)
    }

    override fun onStop() {
        WeaveloClient.setBackgrounded(true)
        WorkspaceClient.backgrounded(true)
        super.onStop()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        androidx.core.view.WindowCompat.getInsetsController(window, window.decorView).apply {
            isAppearanceLightStatusBars = true
            isAppearanceLightNavigationBars = true
        }
        requestNotificationPermissionOnce()
        WeaveloClient.initialize(applicationContext)

        setContent {
            AppLanguageProvider(this) {
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
            val restoredWorkspace = remember { WorkspaceClient.visibleSession() }
            var workspaceDeviceId by remember { mutableStateOf(restoredWorkspace?.deviceId ?: "") }
            var openMainDisplay by remember { mutableStateOf(restoredWorkspace?.openMainDisplay ?: false) }
            var scanningQr by remember { mutableStateOf(false) }
            var manualInvitationRequested by remember { mutableStateOf(false) }
            var workspaceEndpoint by remember { mutableStateOf(restoredWorkspace?.endpoint) }
            var incomingSession by remember { mutableStateOf(restoredWorkspace?.takeIf { it.incomingId != null }) }
            var receivedFiles by remember { mutableStateOf(false) }
            var receivedFileTargetPath by rememberSaveable { mutableStateOf<String?>(null) }
            var workspaceFilePath by rememberSaveable { mutableStateOf<String?>(null) }
            var pairingMessage by remember { mutableStateOf<String?>(null) }
            var pairingText by remember { mutableStateOf<AppText?>(null) }
            val publisherNotice by PublisherClient.notice.collectAsState()
            val outgoing by PublisherClient.outgoing.collectAsState()
            val camera by CameraPublisher.state.collectAsState()
            var cameraTarget by remember { mutableStateOf<String?>(null) }
            val incomingNotice by DirectedDelivery.notice.collectAsState()
            val receiverState by FileReceiverClient.state.collectAsState()
            val controlReady by RemoteControlService.connected.collectAsState()
            val streamPrefs = remember { getSharedPreferences("stream", MODE_PRIVATE) }
            var autoReceiveFiles by remember { mutableStateOf(streamPrefs.getBoolean("auto_receive_files", true)) }
            LaunchedEffect(WeaveloClient.nativeAvailable, devices.isNotEmpty(), autoReceiveFiles, joiningNetwork) {
                if (WeaveloClient.nativeAvailable && devices.isNotEmpty() && autoReceiveFiles && !joiningNetwork && !FileReceiverClient.ownsNetwork) {
                    FileReceiverClient.start(this@MainActivity)
                }
            }
            fun joinNetwork(raw: String) {
                if (joiningNetwork) return
                if (WorkspaceClient.active || PublisherClient.active || camera.enabled ||
                    sessionState in listOf(SessionState.CONNECTING, SessionState.STREAMING, SessionState.RECONNECTING)) {
                    pairingMessage = "error:Disconnect active sessions and stop file reception and screen sharing before changing networks."
                    pairingText = AppText("Disconnect active sessions and stop file reception and screen sharing before changing networks.")
                    return
                }
                joiningNetwork = true
                pairingMessage = "Joining device network..."
                pairingText = AppText("Joining device network...")
                scope.launch {
                    try {
                        if (FileReceiverClient.ownsNetwork) {
                            FileReceiverClient.stop(this@MainActivity)
                            var waited = 0
                            while (FileReceiverClient.ownsNetwork && waited < 30) {
                                delay(50)
                                waited++
                            }
                        }
                        pairingMessage = withContext(Dispatchers.IO) {
                            DirectedDelivery.changeNetwork { WeaveloClient.joinPairingPayload(raw) }
                        }
                        pairingText = pairingResultText(pairingMessage, WeaveloClient.networkName.value)
                    } catch (cancelled: kotlinx.coroutines.CancellationException) {
                        throw cancelled
                    } catch (error: Exception) {
                        pairingMessage = "error:${error.message ?: "Could not join network"}"
                        pairingText = appTextOf(error) ?: if (error.message == null) AppText("Could not join network") else null
                    } finally {
                        joiningNetwork = false
                    }
                }
            }
            var publishAudio by androidx.compose.runtime.saveable.rememberSaveable { mutableStateOf(false) }
            var publishControl by androidx.compose.runtime.saveable.rememberSaveable { mutableStateOf(false) }
            var pendingSendTarget by androidx.compose.runtime.saveable.rememberSaveable { mutableStateOf<String?>(null) }
            var audioChooserOpen by rememberSaveable { mutableStateOf(false) }
            var selectedAudioPackage by rememberSaveable { mutableStateOf<String?>(null) }
            var selectedAudioUid by rememberSaveable { mutableStateOf(-1) }
            var selectedAudioUpdated by rememberSaveable { mutableStateOf(-1L) }
            val projection = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
                val target = pendingSendTarget
                pendingSendTarget = null
                val audioTarget = selectedAudioPackage?.let { PlaybackAudioTarget(it, selectedAudioUid, selectedAudioUpdated) }
                if (result.resultCode == RESULT_OK && result.data != null) PublisherClient.start(this, result.resultCode, result.data!!, publishAudio, publishControl, target, audioTarget)
                else PublisherClient.status("Screen sharing was not started. Confirm Android screen capture to make this phone viewable.", false, AppText("Screen sharing was not started. Confirm Android screen capture to make this phone viewable."))
            }
            fun requestProjection(audio: Boolean, control: Boolean = false) {
                publishAudio = audio; publishControl = control
                if (!audio) selectedAudioPackage = null
                if (audio && android.os.Build.VERSION.SDK_INT < 29) {
                    pendingSendTarget = null
                    PublisherClient.status("Playback audio sharing requires Android 10 or later.", text = AppText("Playback audio sharing requires Android 10 or later."))
                    return
                }
                // Playback audio retains its real MediaProjection grant. A local
                // shell display only supplies video and the existing input path.
                if (!audio && `fun`.hackerlife.weavelo.managed.ManagedScreenBridge.attached() != null) {
                    `fun`.hackerlife.weavelo.managed.ManagedScreenBridge.enable(this, true)
                    PublisherClient.startManaged(this, control, pendingSendTarget)
                    pendingSendTarget = null
                    return
                }
                val manager = getSystemService(android.media.projection.MediaProjectionManager::class.java)
                // A separate user action requests full-display control; existing viewing keeps the OS source choice.
                val intent = if (control && android.os.Build.VERSION.SDK_INT >= 34)
                    manager.createScreenCaptureIntent(android.media.projection.MediaProjectionConfig.createConfigForDefaultDisplay())
                    else manager.createScreenCaptureIntent()
                projection.launch(intent)
            }
            val audioPermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
                if (granted) requestProjection(true,publishControl)
                else {
                    pendingSendTarget = null
                    PublisherClient.status("Audio permission was not granted; screen sharing was not started.", text = AppText("Audio permission was not granted; screen sharing was not started."))
                }
            }
            if (audioChooserOpen) PlaybackAudioDialog(showVideoOnly = pendingSendTarget != null,
                onDismiss = { audioChooserOpen = false; pendingSendTarget = null },
                onVideoOnly = { audioChooserOpen = false; requestProjection(false, publishControl) },
                onSelect = { target ->
                    audioChooserOpen = false
                    selectedAudioPackage = target?.packageName; selectedAudioUid = target?.uid ?: -1
                    selectedAudioUpdated = target?.updated ?: -1L
                    if (target != null && !PlaybackAudioSources.matches(this, target)) {
                        pendingSendTarget = null
                        PublisherClient.status("Selected audio app changed or is unavailable", text = AppText("Selected audio app changed or is unavailable"))
                    } else if (Build.VERSION.SDK_INT >= 29 && checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED)
                        audioPermission.launch(Manifest.permission.RECORD_AUDIO)
                    else requestProjection(true, publishControl)
                })
            val cameraPermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
                if (granted) CameraPublisher.start(this)
                else { cameraTarget = null; CameraPublisher.update(CameraSharingState(status = "Camera permission was not granted", statusText = AppText("Camera permission was not granted"))) }
            }
            fun requestCamera() {
                if (checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) CameraPublisher.start(this)
                else cameraPermission.launch(Manifest.permission.CAMERA)
            }
            if (cameraTarget != null && camera.enabled) androidx.compose.material3.AlertDialog(
                onDismissRequest = { cameraTarget = null }, title = { androidx.compose.material3.Text(tr("Send camera")) },
                text = { androidx.compose.foundation.layout.Column {
                    camera.sources.forEach { source -> androidx.compose.material3.TextButton(onClick = {
                        val target = cameraTarget ?: return@TextButton
                        cameraTarget = null
                        scope.launch(Dispatchers.IO) { runCatching { CameraPublisher.send(target, source) }
                            .onFailure { CameraPublisher.update(camera.copy(status = it.message ?: "Could not send camera", statusText = appTextOf(it) ?: if (it.message == null) AppText("Could not send camera") else null)) } }
                    }) { androidx.compose.material3.Text(appText(cameraNameText(source.facing))) } }
                } }, confirmButton = {}, dismissButton = { androidx.compose.material3.TextButton(onClick = { cameraTarget = null }) { androidx.compose.material3.Text(tr("Cancel")) } })

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
                                    DirectedDelivery.poll(workspaceEndpoint != null || scanningQr || receivedFileTargetPath != null ||
                                        WorkspaceClient.starting || WorkspaceClient.active ||
                                        sessionState in listOf(SessionState.CONNECTING, SessionState.STREAMING, SessionState.RECONNECTING))
                                }.onSuccess { DirectedDelivery.failed(null) }
                                    .onFailure { DirectedDelivery.failed(it.message ?: "Could not receive screens", appTextOf(it) ?: if (it.message == null) AppText("Could not receive screens") else null) }.getOrNull()
                            }
                            if (incoming != null) {
                                incomingSession = incoming
                                sendFileOnConnect = false
                                workspaceFilePath = null
                                openMainDisplay = false
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
                    val shownEndpoint = workspaceEndpoint!!
                    val shownDevice = workspaceDeviceId
                    val shownIncoming = incomingSession
                    `fun`.hackerlife.weavelo.ui.WorkspaceScreen(shownEndpoint, sendFileOnConnect, shownDevice, shownIncoming,
                        fileToSend = workspaceFilePath?.let(::File), onFileQueued = { workspaceFilePath = null },
                        openMainDisplay = openMainDisplay) {
                        if (workspaceEndpoint == shownEndpoint && workspaceDeviceId == shownDevice &&
                            incomingSession?.sessionKey == shownIncoming?.sessionKey) {
                            workspaceEndpoint = null; incomingSession = null; workspaceFilePath = null
                        }
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
                        pairingMessage = listOfNotNull(
                            WeaveloClient.nativeStartupError.takeIf { it.isNotBlank() }?.let { "error:" + trFormat("Native startup failed (%s). Existing data is retained.", it) },
                            pairingMessage?.let { message -> (if (message.startsWith("error:")) "error:" else "") + (pairingText?.let { appText(it) } ?: trFormat("Network join result: %s", message.removePrefix("error:"))) },
                            incomingNotice?.let { notice -> notice.text?.let { appText(it) } ?: trFormat("Details: %s", notice.message) },
                            publisherNotice.message.takeIf { it.isNotBlank() }?.let { publisherNotice.text?.let { text -> appText(text) } ?: trFormat("Details: %s", it) },
                            receiverState.message.takeIf { it.isNotBlank() }?.let { receiverState.messageText?.let { text -> appText(text) } ?: trFormat("Details: %s", it) }
                        ).distinct().joinToString("\n"),
                        sessionError = sessionError,
                        sessionState = sessionState,
                        onCancelConnect = {
                            scope.launch(Dispatchers.IO) { WeaveloClient.disconnect() }
                        },
                        onConnectAddress = { endpoint ->
                            incomingSession = null; sendFileOnConnect = false; workspaceFilePath = null
                            workspaceDeviceId = ""; openMainDisplay = true; workspaceEndpoint = endpoint
                        },
                        onConnectWorkspace = { deviceId, endpoint ->
                            openMainDisplay = deviceId.isNotBlank() && devices.firstOrNull { it.id == deviceId }?.canViewScreen == true
                            incomingSession = null; sendFileOnConnect = false; workspaceFilePath = null
                            workspaceDeviceId = deviceId; workspaceEndpoint = endpoint
                        },
                        onSendFiles = { deviceId, endpoint -> openMainDisplay = false; incomingSession = null; sendFileOnConnect = true; workspaceFilePath = null; workspaceDeviceId = deviceId; workspaceEndpoint = endpoint },
                        outgoing = outgoing,
                        onSendCamera = { target -> cameraTarget = target; if (!camera.enabled) requestCamera() },
                        onSendScreen = { deviceId ->
                            if (PublisherClient.active) scope.launch(Dispatchers.IO) {
                                runCatching { PublisherClient.sendScreen(deviceId) }.onSuccess {
                                    if (!PublisherClient.audioEnabled) PublisherClient.status("This sharing session has no audio. Stop it and start sharing with audio to send sound.", text = AppText("This sharing session has no audio. Stop it and start sharing with audio to send sound."))
                                }.onFailure { PublisherClient.status(it.message ?: "Could not send screen", text = appTextOf(it) ?: if (it.message == null) AppText("Could not send screen") else null) }
                            } else {
                                pendingSendTarget = deviceId; publishControl = false; audioChooserOpen = true
                            }
                        },
                        onStopSending = { id -> scope.launch(Dispatchers.IO) {
                            runCatching { PublisherClient.stopSending(id) }.onFailure { PublisherClient.status(it.message ?: "Could not stop sending", text = appTextOf(it) ?: if (it.message == null) AppText("Could not stop sending") else null) }
                        } },
                        publishing = PublisherClient.active,
                        receivingFiles = receiverState.active || receiverState.starting,
                        receiverStarting = receiverState.starting,
                        onStartReceiving = {
                            autoReceiveFiles = true
                            streamPrefs.edit().putBoolean("auto_receive_files", true).apply()
                            FileReceiverClient.start(this)
                        },
                        onStopReceiving = {
                            autoReceiveFiles = false
                            streamPrefs.edit().putBoolean("auto_receive_files", false).apply()
                            FileReceiverClient.stop(this)
                        },
                        controlReady = controlReady,
                        onControlSettings = { RemoteControlService.openSettings(this) },
                        onShareControlledScreen = {
                            pendingSendTarget = null
                            publishControl = true
                            audioChooserOpen = true
                        },
                        onShareScreen = { audio ->
                            pendingSendTarget = null
                            publishControl = RemoteControlService.connected.value
                            if (audio) audioChooserOpen = true else requestProjection(false,publishControl)
                        },
                        onStopSharing = { PublisherClient.stop(this) },
                        onShareCamera = { requestCamera() },
                        onBrowseFiles = { receivedFiles = true },
                        onJoinCode = ::joinNetwork,
                        showInvitationInput = manualInvitationRequested,
                        onInvitationInputShown = { manualInvitationRequested = false },
                        onScanQr = { scanningQr = true }
                    )
                }
            }
            if (receivedFiles) `fun`.hackerlife.weavelo.ui.ReceivedFilesDialog(onSendToDevice = { file ->
                receivedFiles = false
                receivedFileTargetPath = file.absolutePath
            }) { receivedFiles = false }
            if (receivedFileTargetPath != null) androidx.compose.material3.AlertDialog(
                onDismissRequest = { receivedFileTargetPath = null },
                title = { androidx.compose.material3.Text(tr("Send to device")) },
                text = { androidx.compose.foundation.layout.Column {
                    val targets = devices.filter { it.canSendFiles && it.canOpenWorkspace }
                    if (targets.isEmpty()) androidx.compose.material3.Text(tr("No devices available"))
                    targets.forEach { device -> androidx.compose.material3.TextButton(onClick = {
                        workspaceFilePath = receivedFileTargetPath
                        receivedFileTargetPath = null
                        incomingSession = null
                        sendFileOnConnect = false
                        openMainDisplay = false
                        workspaceDeviceId = device.id
                        workspaceEndpoint = device.endpoint
                    }) { androidx.compose.material3.Text(device.name) } }
                } }, confirmButton = {}, dismissButton = {
                    androidx.compose.material3.TextButton(onClick = { receivedFileTargetPath = null }) { androidx.compose.material3.Text(tr("Cancel")) }
                })
            }
            }
        }
    }
}
