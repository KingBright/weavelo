package `fun`.hackerlife.weavelo.ui
import androidx.compose.material3.TextButton
import androidx.compose.material3.AlertDialog

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import `fun`.hackerlife.weavelo.HostDevice
import `fun`.hackerlife.weavelo.DeviceArtwork
import `fun`.hackerlife.weavelo.DeviceBadgePreset
import `fun`.hackerlife.weavelo.TelemetrySnapshot
import `fun`.hackerlife.weavelo.DiscoveryDiagnostics
import `fun`.hackerlife.weavelo.AppNetworkStatus
import `fun`.hackerlife.weavelo.WeaveloClient
import `fun`.hackerlife.weavelo.publicConnectionEvidence
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import `fun`.hackerlife.weavelo.SessionState

// Air management palette
val ColorBaseBg = Color(0xFFF7F9FB)
val ColorSurfaceCard = Color(0xFFFFFFFF)
val ColorBorderFine = Color(0xFFE2E8F0)
val ColorAccentCyan = Color(0xFF2563EB)
val ColorAccentEmerald = Color(0xFF047857) // accessible green text
val ColorStatusGreen = Color(0xFF10B981)
val ColorTextPrimary = Color(0xFF172033)
val ColorTextSecondary = Color(0xFF526174)

@Composable
fun MobileConsoleScreen(
    telemetry: TelemetrySnapshot,
    devices: List<HostDevice> = emptyList(),
    discoveryDiagnostics: DiscoveryDiagnostics = DiscoveryDiagnostics(),
    appNetwork: AppNetworkStatus = AppNetworkStatus(),
    nativeAvailable: Boolean = false,
    networkName: String = "",
    joiningNetwork: Boolean = false,
    pairingMessage: String? = null,
    sessionError: String = "",
    sessionState: SessionState = SessionState.DISCONNECTED,
    onCancelConnect: () -> Unit = {},
    onConnectAddress: (String) -> Unit,
    onConnectWorkspace: (String, String) -> Unit = { _, _ -> },
    onSendFiles: (String, String) -> Unit = { _, _ -> },
    outgoing: `fun`.hackerlife.weavelo.OutgoingScreenState = `fun`.hackerlife.weavelo.OutgoingScreenState(),
    onSendScreen: (String) -> Unit = {},
    onSendCamera: (String) -> Unit = {},
    onStopSending: (Long) -> Unit = {},
    publishing: Boolean = false,
    receivingFiles: Boolean = false,
    receiverStarting: Boolean = false,
    onStartReceiving: () -> Unit = {},
    onStopReceiving: () -> Unit = {},
    controlReady: Boolean = false,
    onControlSettings: () -> Unit = {},
    onShareControlledScreen: () -> Unit = {},
    onShareScreen: (Boolean) -> Unit = {},
    onShareCamera: () -> Unit = {},
    onStopSharing: () -> Unit = {},
    onBrowseFiles: () -> Unit = {},
    onJoinCode: (String) -> Unit = {},
    showInvitationInput: Boolean = false,
    onInvitationInputShown: () -> Unit = {},
    onScanQr: () -> Unit
) {
    var pairingCode by remember { mutableStateOf("") }
    var qualityDelivery by remember { mutableStateOf<Long?>(null) }
    qualityDelivery?.let { id -> OutgoingVideoQualityDialog(id) { qualityDelivery = null } }
    val openQuality = { qualityDelivery = if (outgoing.active) outgoing.id else 0L }
    var pairingCodeOpen by remember { mutableStateOf(false) }
    var invitationOpen by remember { mutableStateOf(false) }
    if (invitationOpen) InvitationDialog { invitationOpen = false }
    LaunchedEffect(showInvitationInput) {
        if (showInvitationInput) {
            pairingCodeOpen = true
            onInvitationInputShown()
        }
    }
    var advancedConnection by remember { mutableStateOf(false) }
    if (pairingCodeOpen) AlertDialog(onDismissRequest = { pairingCode = ""; pairingCodeOpen = false }, title = { Text(tr("Join with invitation")) },
        text = { OutlinedTextField(pairingCode, { pairingCode = it }, label = { Text(tr("Invitation from an existing network member")) }) },
        dismissButton = { TextButton(onClick = { pairingCode = ""; pairingCodeOpen = false }) { Text(tr("Cancel")) } },
        confirmButton = { TextButton(onClick = { onJoinCode(pairingCode.trim()); pairingCode = ""; pairingCodeOpen = false }, enabled = nativeAvailable && !joiningNetwork && pairingCode.isNotBlank()) { Text(tr("Join")) } })
    var activeTab by remember { mutableStateOf("Devices") }
    val update = rememberAppUpdate()
    var manualEndpoint by remember { mutableStateOf("") }
    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(ColorBaseBg)
            .safeDrawingPadding()
            .padding(horizontal = 12.dp, vertical = 6.dp)
    ) {
        val context = androidx.compose.ui.platform.LocalContext.current
        val cameraState by `fun`.hackerlife.weavelo.CameraPublisher.state.collectAsState()
        val workspaceState by `fun`.hackerlife.weavelo.WorkspaceClient.state.collectAsState()
        Column(
            modifier = Modifier.fillMaxSize()
        ) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically) {
                Text("Weavelo", color = ColorTextPrimary, fontSize = 22.sp, fontWeight = FontWeight.SemiBold)
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    if (update.release != null && activeTab != "Settings") {
                        TextButton(onClick = { activeTab = "Settings" }) { Text(tr("Update available")) }
                    }
                    TextButton(onClick = { activeTab = "Mesh P2P" }, enabled = nativeAvailable) {
                        androidx.compose.material3.Icon(MobileSymbol.Network.image, contentDescription = null, modifier = Modifier.size(18.dp))
                        Spacer(Modifier.width(4.dp))
                        Text(tr("Pair devices"))
                    }
                }
            }
            if (publishing || outgoing.active || cameraState.enabled) {
                val peerNames = remember(outgoing.peers, outgoing.target, devices) {
                    val ids = buildSet {
                        addAll(outgoing.peers.filterValues { it > 0 }.keys)
                        if (outgoing.active && outgoing.target.isNotBlank()) add(outgoing.target)
                    }
                    ids.map { id -> devices.firstOrNull { it.id == id }?.name?.ifBlank { id } ?: id }
                }
                val bannerTitle = when {
                    (publishing || outgoing.active) && controlReady -> tr("Sharing screen & remote control")
                    publishing || outgoing.active -> tr("Sharing screen with paired devices")
                    else -> tr("Sharing camera with paired devices")
                }
                val bannerSub = if (peerNames.isNotEmpty()) {
                    trFormat("Active viewers: %s", peerNames.joinToString(", "))
                } else {
                    tr("Waiting for paired devices to view")
                }
                Spacer(Modifier.height(6.dp))
                Row(
                    Modifier.fillMaxWidth()
                        .clip(RoundedCornerShape(12.dp))
                        .background(Color(0xFFFEF3C7))
                        .border(1.dp, Color(0xFFD97706), RoundedCornerShape(12.dp))
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        Text(bannerTitle, color = Color(0xFF92400E), fontSize = 13.sp, fontWeight = FontWeight.SemiBold)
                        Text(bannerSub, color = Color(0xFF92400E), fontSize = 11.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    }
                    Button(
                        colors = ButtonDefaults.buttonColors(containerColor = Color(0xFFB42318), contentColor = Color.White),
                        contentPadding = PaddingValues(horizontal = 12.dp, vertical = 6.dp),
                        onClick = {
                            if (outgoing.active) onStopSending(outgoing.id)
                            if (publishing) onStopSharing()
                            if (cameraState.enabled) `fun`.hackerlife.weavelo.CameraPublisher.stop(context)
                        }
                    ) { Text(tr("Stop now"), fontSize = 12.sp) }
                }
            }
            Spacer(Modifier.height(8.dp))

            if (activeTab == "Mesh P2P" || devices.isEmpty()) {
            // 2. 快捷动作按钮栏
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(10.dp)
            ) {
                QuickActionButton(
                    text = tr("Scan to join network"),
                    modifier = Modifier.weight(1f),
                    enabled = nativeAvailable,
                    onClick = onScanQr
                )
                QuickActionButton(
                    text = tr("Enter invitation"),
                    modifier = Modifier.weight(1f),
                    enabled = nativeAvailable,
                    onClick = { pairingCodeOpen = true }
                )
            }
            TextButton(onClick = { invitationOpen = true }, enabled = nativeAvailable && !joiningNetwork) {
                Text(tr("Show invitation QR"), color = ColorAccentCyan)
            }
            if (networkName.isNotBlank()) {
                Text(trFormat("Network: %s", networkName), color = ColorTextSecondary, fontSize = 11.sp,
                    maxLines = 1, overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.padding(top = 8.dp))
            }

            if (!pairingMessage.isNullOrBlank()) {
                Text(
                    text = pairingMessage,
                    color = if (pairingMessage.startsWith("error:")) Color(0xFFB42318) else ColorAccentEmerald,
                    fontSize = 12.sp,
                    modifier = Modifier.padding(vertical = 8.dp)
                )
            }

            }
            Spacer(modifier = Modifier.height(12.dp))

            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .weight(1f)
            ) {
                when (activeTab) {
                    "Devices" -> DevicesTabContent(
                        devices = devices,
                        nativeAvailable = nativeAvailable,
                        sessionState = sessionState,
                        telemetry = telemetry,
                        discoveryDiagnostics = discoveryDiagnostics,
                        manualEndpoint = manualEndpoint,
                        onManualEndpointChange = { manualEndpoint = it },
                        advancedConnection = advancedConnection,
                        onToggleAdvanced = { advancedConnection = !advancedConnection },
                        onConnectAddress = onConnectAddress,
                        onConnectWorkspace = onConnectWorkspace,
                        onSendFiles = onSendFiles,
                        localStatus = when {
                            !nativeAvailable -> tr("Connection service unavailable")
                            joiningNetwork -> tr("Joining network…")
                            outgoing.unattributedSharing > 0 -> trFormat("Sharing · %d direct streams", outgoing.unattributedSharing)
                            publishing && !controlReady -> tr("Sharing ready · review control permission")
                            publishing -> tr("Sharing ready")
                            else -> tr("Sharing & permissions")
                        },
                        onLocalSettings = { activeTab = "Security" },
                        onLocalDetails = { activeTab = "Mesh P2P" },
                        outgoing = outgoing, onSendScreen = onSendScreen, onSendCamera = onSendCamera, onStopSending = onStopSending,
                        onOutgoingQuality = openQuality
                    )
                    "Mesh P2P" -> MeshP2pTabContent(
                        devices = devices,
                        networkName = networkName,
                        nativeAvailable = nativeAvailable,
                        joiningNetwork = joiningNetwork,
                        onConnectWorkspace = onConnectWorkspace
                    )
                    "Security" -> SecurityTabContent(
                        nativeAvailable = nativeAvailable,
                        publishing = publishing,
                        receivingFiles = receivingFiles,
                        receiverStarting = receiverStarting,
                        onStartReceiving = onStartReceiving,
                        onStopReceiving = onStopReceiving,
                        controlReady = controlReady,
                        onControlSettings = onControlSettings,
                        onShareControlledScreen = onShareControlledScreen,
                        onShareScreen = onShareScreen,
                        onShareCamera = onShareCamera,
                        onStopSharing = onStopSharing,
                        onBrowseFiles = onBrowseFiles,
                        onOutgoingQuality = openQuality
                    )
                    "Settings" -> SettingsTabContent(
                        update = update,
                        updateAllowed = !outgoing.active && !publishing && !workspaceState.transferring &&
                            sessionState == SessionState.DISCONNECTED,
                        onPrepareInstall = { if (receivingFiles) onStopReceiving() },
                        appNetwork = appNetwork,
                        nativeAvailable = nativeAvailable,
                        discoveryDiagnostics = discoveryDiagnostics,
                        sessionState = sessionState
                    )
                }
            }

            // 4. 底部悬浮磨砂导航栏
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(top = 10.dp)
                    .clip(RoundedCornerShape(9999.dp))
                    .background(ColorSurfaceCard)
                    .border(1.dp, ColorBorderFine, RoundedCornerShape(9999.dp))
                    .padding(vertical = 12.dp),
                horizontalArrangement = Arrangement.SpaceEvenly,
                verticalAlignment = Alignment.CenterVertically
            ) {
                NavTab("Devices", active = activeTab == "Devices") { activeTab = "Devices" }
                NavTab("Network", active = activeTab == "Mesh P2P") { activeTab = "Mesh P2P" }
                NavTab("Sharing", active = activeTab == "Security") { activeTab = "Security" }
                NavTab("Settings", active = activeTab == "Settings") { activeTab = "Settings" }
            }
        }
    }
}

@Composable
fun QuickActionButton(text: String, modifier: Modifier = Modifier, enabled: Boolean = true, onClick: () -> Unit) {
    Box(
        modifier = modifier
            .clip(RoundedCornerShape(8.dp))
            .background(ColorSurfaceCard)
            .border(1.dp, ColorBorderFine, RoundedCornerShape(8.dp))
            .clickable(enabled = enabled, onClick = onClick)
            .padding(vertical = 12.dp),
        contentAlignment = Alignment.Center
    ) {
        Text(
            text = tr(text),
            color = ColorTextPrimary,
            fontSize = 12.sp,
            fontWeight = FontWeight.SemiBold
        )
    }
}

@Composable
fun HostCard(name: String, tag: String, subtext: String, enabled: Boolean = true,
    disabledHint: String? = null,
    deviceBadge: DeviceBadgePreset = DeviceBadgePreset.COMPUTER,
    canSendFiles: Boolean = false, onSendFiles: () -> Unit = {},
    canSendScreen: Boolean = false, sending: Boolean = false, onSendScreen: () -> Unit = {},
    canSendCamera: Boolean = false, onSendCamera: () -> Unit = {},
    details: String = "", hasActiveStream: Boolean = false, onEnd: (() -> Unit)? = null, onConnect: () -> Unit) {
    val context = androidx.compose.ui.platform.LocalContext.current
    var sendMenu by remember(name) { mutableStateOf(false) }
    var expanded by remember(name) { mutableStateOf(false) }
    val badge = remember(deviceBadge) { DeviceArtwork.badge(deviceBadge).asImageBitmap() }
    val activeStream = hasActiveStream || sending || onEnd != null || subtext.contains("Receiving") || subtext.contains("Sharing")
    Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp))
        .background(if (activeStream) Color(0xFFEFF6FF) else ColorSurfaceCard)
        .border(1.dp, if (activeStream) ColorAccentCyan.copy(alpha = .5f) else ColorBorderFine, RoundedCornerShape(12.dp))
        .clickable(enabled = enabled || disabledHint != null, onClick = {
            if (enabled) onConnect()
            else if (disabledHint != null) android.widget.Toast.makeText(context, disabledHint, android.widget.Toast.LENGTH_SHORT).show()
        }).padding(horizontal = 10.dp, vertical = 6.dp)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Image(badge, contentDescription = trFormat("Device type: %s", tr(deviceBadge.displayName)),
                modifier = Modifier.size(24.dp))
            Text(name, modifier = Modifier.weight(1f), color = ColorTextPrimary, fontSize = 15.sp,
                lineHeight = 20.sp, fontWeight = FontWeight.SemiBold, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(tag, color = ColorAccentEmerald, fontSize = 10.sp, lineHeight = 14.sp)
                Text(subtext, color = ColorTextSecondary, fontSize = 11.sp, lineHeight = 15.sp, maxLines = 2,
                    overflow = TextOverflow.Ellipsis)
            }
            MobileAction(MobileSymbol.View, trFormat("Open device %s", name), enabled, modifier = Modifier.size(48.dp), onClick = onConnect)
            Box {
                MobileAction(if (sending) MobileSymbol.Close else MobileSymbol.Share,
                    if (sending) trFormat("Stop sending to %s", name) else trFormat("Send to %s", name), canSendScreen || canSendCamera,
                    modifier = Modifier.size(48.dp), onClick = { if (sending) onSendScreen() else sendMenu = true })
                androidx.compose.material3.DropdownMenu(expanded = sendMenu && !sending,
                    onDismissRequest = { sendMenu = false }) {
                    androidx.compose.material3.DropdownMenuItem(text = { Text(tr("Screen or app")) },
                        enabled = canSendScreen, leadingIcon = {
                            androidx.compose.material3.Icon(MobileSymbol.Devices.image, contentDescription = null)
                        }, onClick = { sendMenu = false; onSendScreen() })
                    androidx.compose.material3.DropdownMenuItem(text = { Text(tr("Camera")) },
                        enabled = canSendCamera, leadingIcon = {
                            androidx.compose.material3.Icon(MobileSymbol.Camera.image, contentDescription = null)
                        }, onClick = { sendMenu = false; onSendCamera() })
                }
            }
            MobileAction(MobileSymbol.Files, trFormat("Send files to %s", name), canSendFiles, modifier = Modifier.size(48.dp), onClick = onSendFiles)
            MobileAction(MobileSymbol.More, trFormat("Device details %s", name), true, modifier = Modifier.size(48.dp), onClick = { expanded = !expanded })
        }
        if (expanded) {
            Text(details.ifBlank { subtext }, color = ColorTextSecondary, fontSize = 11.sp, lineHeight = 16.sp,
                modifier = Modifier.padding(vertical = 4.dp))
            onEnd?.let { end -> TextButton(onClick = end) { Text(tr("End streams")) } }
        }
    }
}

@Composable
fun TelemetryLaneRow(name: String, value: String, color: Color) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically
    ) {
        Text(text = name, color = ColorTextPrimary, fontSize = 11.sp, fontWeight = FontWeight.Medium)
        Text(text = value, color = color, fontSize = 11.sp, fontFamily = FontFamily.Monospace, fontWeight = FontWeight.Bold)
    }
}

@Composable
fun NavTab(text: String, active: Boolean, onClick: () -> Unit = {}) {
    val symbol = when (text) {
        "Devices" -> MobileSymbol.Devices
        "Network" -> MobileSymbol.Network
        "Sharing" -> MobileSymbol.Permissions
        else -> MobileSymbol.Settings
    }
    Column(modifier = Modifier.width(72.dp).heightIn(min = 48.dp)
        .clip(RoundedCornerShape(18.dp))
        .background(if (active) ColorAccentEmerald.copy(alpha = .1f) else Color.Transparent)
        .clickable(onClick = onClick).padding(vertical = 6.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(4.dp)) {
        androidx.compose.material3.Icon(symbol.image, contentDescription = null,
            tint = if (active) ColorAccentEmerald else ColorTextSecondary, modifier = Modifier.size(20.dp))
        Text(tr(text), color = if (active) ColorTextPrimary else ColorTextSecondary, fontSize = 11.sp, lineHeight = 16.sp,
            fontWeight = if (active) FontWeight.SemiBold else FontWeight.Normal)
    }
}

@Composable
private fun DevicesTabContent(
    devices: List<HostDevice>,
    nativeAvailable: Boolean,
    sessionState: SessionState,
    telemetry: TelemetrySnapshot,
    discoveryDiagnostics: DiscoveryDiagnostics,
    manualEndpoint: String,
    onManualEndpointChange: (String) -> Unit,
    advancedConnection: Boolean,
    onToggleAdvanced: () -> Unit,
    onConnectAddress: (String) -> Unit,
    onConnectWorkspace: (String, String) -> Unit,
    onSendFiles: (String, String) -> Unit,
    localStatus: String,
    onLocalSettings: () -> Unit,
    onLocalDetails: () -> Unit,
    outgoing: `fun`.hackerlife.weavelo.OutgoingScreenState,
    onSendScreen: (String) -> Unit,
    onSendCamera: (String) -> Unit,
    onStopSending: (Long) -> Unit,
    onOutgoingQuality: () -> Unit
) {
    val workspace by `fun`.hackerlife.weavelo.WorkspaceClient.state.collectAsState()
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(bottom = 88.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        item {
            Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(ColorSurfaceCard)
                .padding(start = 10.dp, end = 2.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f).padding(vertical = 8.dp)) {
                    Text(android.os.Build.MODEL, color = ColorTextPrimary, fontSize = 13.sp, fontWeight = FontWeight.SemiBold)
                    Text(localStatus, color = ColorTextSecondary, fontSize = 11.sp, lineHeight = 15.sp)
                }
                MobileAction(MobileSymbol.Permissions, tr("Sharing & permissions"), true, modifier = Modifier.size(48.dp), onClick = onLocalSettings)
                MobileAction(MobileSymbol.More, tr("This device & pairing"), true, modifier = Modifier.size(48.dp), onClick = onLocalDetails)
            }
        }

        item { TextButton(onClick = onOutgoingQuality) { Text(tr("Outgoing screen quality")) } }

        if (devices.isEmpty()) {
            item {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(12.dp))
                        .background(ColorSurfaceCard)
                        .border(1.dp, ColorBorderFine, RoundedCornerShape(12.dp))
                        .padding(16.dp)
                ) {
                    Text(
                        tr(discoveryDiagnostics.emptyMessage),
                        color = ColorTextSecondary,
                        fontSize = 13.sp
                    )
                }
            }
        } else {
            items(devices.size) { index ->
                val device = devices[index]
                val receiving = workspace.deviceId == device.id
                val streams = if (receiving) workspace.panes.count { it.accepted && it.videoError == null } else 0
                val sharing = outgoing.peers[device.id] ?: 0
                val errorPane = if (receiving) workspace.panes.firstOrNull { it.videoError != null || it.inputError != null } else null
                val streamError = errorPane?.videoError ?: errorPane?.inputError
                val streamErrorText = if (errorPane?.videoError != null) errorPane.videoErrorText else errorPane?.inputErrorText
                val shownError = streamError?.let { streamErrorText?.let { text -> appText(text) } ?: trFormat("Details: %s", it) }
                val displayName = device.name.ifBlank { device.id }
                val state = buildList {
                    if (sharing > 0) add(trFormat("Sharing · %d", sharing))
                    if (streams > 0) add(trFormat("Receiving · %d", streams))
                    if (outgoing.active && outgoing.target == device.id && sharing == 0) add(tr("Connecting…"))
                    if (receiving && workspace.transferring) add(tr("Transferring files…"))
                    if (streamError != null) add(tr("Needs attention"))
                    if (device.online && !device.canOpenWorkspace) add(tr("Online · sharing off"))
                    else if (!device.hasService) add(tr("No shared sources"))
                }.joinToString(" · ")
                HostCard(
                    name = displayName,
                    deviceBadge = device.deviceBadge,
                    tag = device.scope,
                    subtext = state.ifBlank { tr("Available") },
                    disabledHint = if (device.online && !device.canOpenWorkspace) trFormat("Start screen sharing or file reception on %s first.", displayName) else null,
                    hasActiveStream = sharing > 0 || streams > 0,
                    details = listOfNotNull(shownError, if (outgoing.target == device.id) outgoing.status.takeIf { it.isNotBlank() }?.let { trFormat("Details: %s", it) } else null,
                        deviceCapabilities(device), device.endpoint, device.id).joinToString("\n"),
                    onEnd = if (receiving && workspace.panes.isNotEmpty()) ({ `fun`.hackerlife.weavelo.WorkspaceClient.disconnectSession(workspace.sessionKey) }) else null,
                    enabled = nativeAvailable && device.canOpenWorkspace && sessionState != SessionState.CONNECTING,
                    canSendFiles = nativeAvailable && device.canSendFiles,
                    canSendScreen = nativeAvailable && (outgoing.active && outgoing.target == device.id || !outgoing.active && device.canReceiveScreen),
                    sending = outgoing.active && outgoing.target == device.id,
                    onSendScreen = { if (outgoing.active && outgoing.target == device.id) onStopSending(outgoing.id) else onSendScreen(device.id) },
                    canSendCamera = nativeAvailable && !outgoing.active && device.canReceiveScreen,
                    onSendCamera = { onSendCamera(device.id) },
                    onSendFiles = { onSendFiles(device.id, device.endpoint) },
                    onConnect = { onConnectWorkspace(device.id, device.endpoint) }
                )

            }
        }

        item {
            TextButton(onClick = onToggleAdvanced) {
                Text(if (advancedConnection) tr("Hide manual connection") else tr("Advanced: connect by address"))
            }
            if (advancedConnection) {
                OutlinedTextField(
                    value = manualEndpoint,
                    onValueChange = onManualEndpointChange,
                    label = { Text(tr("Host IP:port")) },
                    singleLine = true,
                    colors = OutlinedTextFieldDefaults.colors(
                        focusedTextColor = ColorTextPrimary, unfocusedTextColor = ColorTextPrimary,
                        focusedLabelColor = ColorAccentCyan, unfocusedLabelColor = ColorTextSecondary,
                        focusedBorderColor = ColorAccentCyan, unfocusedBorderColor = ColorTextSecondary,
                        cursorColor = ColorAccentCyan
                    ),
                    modifier = Modifier.fillMaxWidth()
                )
                Spacer(modifier = Modifier.height(8.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(
                        colors = ButtonDefaults.buttonColors(
                            containerColor = ColorAccentCyan,
                            contentColor = Color.Black,
                            disabledContainerColor = ColorSurfaceCard,
                            disabledContentColor = ColorTextSecondary
                        ),
                        enabled = nativeAvailable && manualEndpoint.isNotBlank() && sessionState != SessionState.CONNECTING,
                        onClick = { onConnectAddress(manualEndpoint.trim()) }
                    ) { Text(tr("Connect to address")) }

                    TextButton(
                        onClick = { onConnectWorkspace("", manualEndpoint.trim()) },
                        enabled = nativeAvailable && manualEndpoint.isNotBlank()
                    ) { Text(tr("Open workspace")) }
                }
            }
        }

    }
}

@Composable
private fun MeshP2pTabContent(
    devices: List<HostDevice>,
    networkName: String,
    nativeAvailable: Boolean,
    joiningNetwork: Boolean,
    onConnectWorkspace: (String, String) -> Unit
) {
    val lanCount = devices.count { it.scope.equals("LAN", ignoreCase = true) }
    val relayCount = devices.count { it.scope.equals("Relay", ignoreCase = true) }
    val p2pCount = devices.count { it.scope.equals("P2P", ignoreCase = true) || it.scope.equals("Direct", ignoreCase = true) }
    val meshBadgeText = when {
        joiningNetwork -> tr("Joining...")
        !nativeAvailable -> tr("Offline")
        lanCount > 0 && relayCount > 0 -> trFormat("LAN · %d / Relay · %d", lanCount, relayCount)
        lanCount > 0 -> trFormat("LAN · %d", lanCount)
        p2pCount > 0 -> trFormat("P2P · %d", p2pCount)
        relayCount > 0 -> trFormat("Relay · %d", relayCount)
        else -> tr("Ready")
    }
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(bottom = 88.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        item {
            Text(
                text = tr("PRIVATE MESH TOPOLOGY"),
                color = ColorTextSecondary,
                fontSize = 11.sp,
                fontWeight = FontWeight.Bold,
                fontFamily = FontFamily.Monospace,
                letterSpacing = 1.sp
            )
        }

        item {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(12.dp))
                    .background(ColorSurfaceCard)
                    .border(1.dp, ColorBorderFine, RoundedCornerShape(12.dp))
                    .padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text(
                        text = networkName.ifBlank { tr("Personal Device Mesh") },
                        color = ColorTextPrimary,
                        fontSize = 15.sp,
                        fontWeight = FontWeight.Bold
                    )
                    Box(
                        modifier = Modifier
                            .clip(RoundedCornerShape(9999.dp))
                            .background(Color(0xFFEFF6FF))
                            .border(1.dp, ColorAccentCyan, RoundedCornerShape(9999.dp))
                            .padding(horizontal = 8.dp, vertical = 3.dp)
                    ) {
                        Text(
                            text = meshBadgeText,
                            color = ColorAccentCyan,
                            fontSize = 10.sp,
                            fontFamily = FontFamily.Monospace,
                            fontWeight = FontWeight.Bold
                        )
                    }
                }
                Text(
                    text = trFormat("%d linked peers in private overlay network", devices.size),
                    color = ColorTextSecondary,
                    fontSize = 12.sp
                )
                Text(
                    text = tr("Encrypted point-to-point tunnels with zero-config hole punching."),
                    color = ColorTextSecondary,
                    fontSize = 11.sp
                )
            }
        }

        item {
            Text(
                text = tr("LINKED PEERS"),
                color = ColorTextSecondary,
                fontSize = 11.sp,
                fontWeight = FontWeight.Bold,
                fontFamily = FontFamily.Monospace,
                letterSpacing = 1.sp
            )
        }

        if (devices.isEmpty()) {
            item {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(12.dp))
                        .background(ColorSurfaceCard)
                        .border(1.dp, ColorBorderFine, RoundedCornerShape(12.dp))
                        .padding(16.dp)
                ) {
                    Text(
                        tr("No peers linked yet. Scan a QR invitation or enter a join code from another device to expand your mesh."),
                        color = ColorTextSecondary,
                        fontSize = 13.sp
                    )
                }
            }
        } else {
            items(devices.size) { index ->
                val device = devices[index]
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(12.dp))
                        .background(ColorSurfaceCard)
                        .border(1.dp, ColorBorderFine, RoundedCornerShape(12.dp))
                        .padding(14.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp)
                ) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text(
                            text = device.name.ifBlank { device.id },
                            color = ColorTextPrimary,
                            fontSize = 14.sp,
                            fontWeight = FontWeight.Bold
                        )
                        Box(
                            modifier = Modifier
                                .clip(RoundedCornerShape(4.dp))
                                .background(Color(0xFFF1F5F9))
                                .padding(horizontal = 6.dp, vertical = 2.dp)
                        ) {
                            Text(
                                text = device.scope,
                                color = ColorAccentCyan,
                                fontSize = 9.sp,
                                fontFamily = FontFamily.Monospace,
                                fontWeight = FontWeight.Bold
                            )
                        }
                    }
                    Text(
                        text = trFormat("Endpoint: %s", device.endpoint.ifBlank { tr("Auto-negotiated UDP") }),
                        color = ColorTextSecondary,
                        fontSize = 11.sp,
                        fontFamily = FontFamily.Monospace
                    )
                    Text(
                        text = deviceCapabilities(device),
                        color = ColorTextSecondary,
                        fontSize = 11.sp
                    )
                    if (device.canOpenWorkspace) {
                        TextButton(
                            onClick = { onConnectWorkspace(device.id, device.endpoint) },
                            enabled = nativeAvailable && !joiningNetwork,
                            modifier = Modifier.align(Alignment.End)
                        ) { Text(tr("Inspect peer")) }
                    }
                }
            }
        }

        item {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(12.dp))
                    .background(ColorSurfaceCard)
                    .border(1.dp, ColorBorderFine, RoundedCornerShape(12.dp))
                    .padding(14.dp),
                verticalArrangement = Arrangement.spacedBy(4.dp)
            ) {
                Text(
                    text = tr("ROUTING & SECURITY POLICY"),
                    color = ColorTextPrimary,
                    fontSize = 12.sp,
                    fontWeight = FontWeight.Bold
                )
                Text(
                    text = tr("• Local Subnet: Direct UDP\n• WAN Cross-NAT: STUN UDP Hole Punching\n• Fallback: Authenticated end-to-end encrypted relay"),
                    color = ColorTextSecondary,
                    fontSize = 11.sp,
                    lineHeight = 16.sp
                )
            }
        }
    }
}

@Composable
private fun SecurityTabContent(
    nativeAvailable: Boolean,
    publishing: Boolean,
    receivingFiles: Boolean,
    receiverStarting: Boolean,
    onStartReceiving: () -> Unit,
    onStopReceiving: () -> Unit,
    controlReady: Boolean,
    onControlSettings: () -> Unit,
    onShareControlledScreen: () -> Unit,
    onShareScreen: (Boolean) -> Unit,
    onShareCamera: () -> Unit,
    onStopSharing: () -> Unit,
    onBrowseFiles: () -> Unit,
    onOutgoingQuality: () -> Unit
) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val camera by `fun`.hackerlife.weavelo.CameraPublisher.state.collectAsState()
    val lifecycle = androidx.compose.ui.platform.LocalLifecycleOwner.current.lifecycle
    var permissionRevision by remember { mutableStateOf(0) }
    var securityDetails by remember { mutableStateOf(false) }
    androidx.compose.runtime.DisposableEffect(lifecycle) {
        val observer = androidx.lifecycle.LifecycleEventObserver { _, event ->
            if (event == androidx.lifecycle.Lifecycle.Event.ON_RESUME) permissionRevision++
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
    val audioAllowed = remember(permissionRevision) {
        context.checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) == android.content.pm.PackageManager.PERMISSION_GRANTED
    }
    val notificationsAllowed = remember(permissionRevision) {
        context.getSystemService(android.app.NotificationManager::class.java).areNotificationsEnabled()
    }
    if (securityDetails) AlertDialog(onDismissRequest = { securityDetails = false },
        title = { Text(tr("Connection security")) },
        text = { Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            TelemetryLaneRow(tr("Transport cipher"), "ChaCha20-Poly1305", ColorAccentCyan)
            TelemetryLaneRow(tr("Network access"), tr("Invitation required"), ColorAccentEmerald)
            TelemetryLaneRow(tr("Authentication"), tr("Paired device group"), ColorAccentCyan)
            TelemetryLaneRow(tr("Storage"), tr("App private"), ColorTextSecondary)
        } }, confirmButton = { TextButton(onClick = { securityDetails = false }) { Text(tr("Done")) } })
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 88.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item {
            Text(tr("Sharing & permissions"), color = ColorTextPrimary, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
            Text(tr("Share a screen, app or camera with your devices."),
                color = ColorTextSecondary, fontSize = 12.sp, modifier = Modifier.padding(top = 6.dp))
        }
        item {
            Column(Modifier.fillMaxWidth().background(ColorSurfaceCard, RoundedCornerShape(18.dp))
                .border(1.dp, ColorBorderFine, RoundedCornerShape(18.dp)).padding(horizontal = 14.dp, vertical = 6.dp)) {
                SharingPermissionRow(MobileSymbol.Share, "Screen or app", if (publishing) tr("Sharing") else tr("Not sharing"),
                    if (publishing) "Stop" else "Share", nativeAvailable) {
                    if (publishing) onStopSharing() else onShareScreen(false)
                }
                TextButton(onClick = onOutgoingQuality, modifier = Modifier.fillMaxWidth()) { Text(tr("Outgoing screen quality")) }
                SharingPermissionRow(MobileSymbol.Camera, "Camera", camera.statusText?.let { appText(it) } ?: trFormat("Details: %s", camera.status),
                    if (camera.enabled) "Stop" else "Share", nativeAvailable) {
                    if (camera.enabled) `fun`.hackerlife.weavelo.CameraPublisher.stop(context) else onShareCamera()
                }
                SharingPermissionRow(MobileSymbol.Control, "Remote control", if (controlReady) tr("Allowed") else tr("Needs permission"),
                    if (controlReady) "Settings" else "Allow", nativeAvailable, onControlSettings)
                SharingPermissionRow(MobileSymbol.Audio, "Audio", if (audioAllowed) tr("Microphone allowed") else tr("Needs permission"),
                    "Share with audio", nativeAvailable && !publishing) { onShareScreen(true) }
                SharingPermissionRow(MobileSymbol.Notifications, "Notifications", if (notificationsAllowed) tr("Allowed") else tr("Off"),
                    "Settings", true) {
                    context.startActivity(android.content.Intent(android.provider.Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                        .putExtra(android.provider.Settings.EXTRA_APP_PACKAGE, context.packageName))
                }
                if (controlReady) TextButton(onClick = onShareControlledScreen, enabled = nativeAvailable && !publishing,
                    modifier = Modifier.fillMaxWidth()) { Text(tr("Share with control and audio")) }
            }
        }
        item {
            Column(Modifier.fillMaxWidth().background(ColorSurfaceCard, RoundedCornerShape(18.dp))
                .border(1.dp, ColorBorderFine, RoundedCornerShape(18.dp)).padding(horizontal = 14.dp, vertical = 6.dp)) {
                SharingPermissionRow(MobileSymbol.Files, "Receive files",
                    if (receiverStarting) tr("Starting…") else if (receivingFiles) tr("On") else tr("Off"),
                    if (receivingFiles) "Stop" else "Start", nativeAvailable && !receiverStarting) {
                    if (receivingFiles) onStopReceiving() else onStartReceiving()
                }
                TextButton(onClick = onBrowseFiles, modifier = Modifier.fillMaxWidth()) { Text(tr("Browse received files")) }
            }
        }
        item { TextButton(onClick = { securityDetails = true }) { Text(tr("Connection security")) } }
    }
}

@Composable
private fun SharingPermissionRow(symbol: MobileSymbol, title: String, state: String, action: String,
    enabled: Boolean, onClick: () -> Unit) {
    Row(Modifier.fillMaxWidth().heightIn(min = 64.dp), verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        androidx.compose.material3.Icon(symbol.image, contentDescription = null, tint = ColorAccentCyan,
            modifier = Modifier.size(22.dp))
        Column(Modifier.weight(1f)) {
            Text(tr(title), color = ColorTextPrimary, fontSize = 14.sp, fontWeight = FontWeight.Medium)
            Text(state, color = ColorTextSecondary, fontSize = 11.sp)
        }
        TextButton(onClick = onClick, enabled = enabled) { Text(tr(action), fontSize = 12.sp) }
    }
}

@Composable
private fun SettingsTabContent(
    update: AppUpdateUi,
    updateAllowed: Boolean,
    onPrepareInstall: () -> Unit = {},
    appNetwork: AppNetworkStatus,
    nativeAvailable: Boolean,
    discoveryDiagnostics: DiscoveryDiagnostics,
    sessionState: SessionState
) {
    val context = androidx.compose.ui.platform.LocalContext.current
    val backgroundPrefs = remember(context) { context.getSharedPreferences("stream", android.content.Context.MODE_PRIVATE) }
    var keepOnline by remember(backgroundPrefs) { mutableStateOf(`fun`.hackerlife.weavelo.DevicePresenceService.enabled(context)) }
    var showAdvancedDiagnostics by remember { mutableStateOf(false) }
    androidx.compose.runtime.DisposableEffect(backgroundPrefs) {
        val listener = android.content.SharedPreferences.OnSharedPreferenceChangeListener { _, key ->
            if (key == `fun`.hackerlife.weavelo.DevicePresenceService.PREFERENCE) {
                keepOnline = `fun`.hackerlife.weavelo.DevicePresenceService.enabled(context)
            }
        }
        backgroundPrefs.registerOnSharedPreferenceChangeListener(listener)
        onDispose { backgroundPrefs.unregisterOnSharedPreferenceChangeListener(listener) }
    }
    var connectionDetails by remember { mutableStateOf<String?>(null) }
    connectionDetails?.let { evidence ->
        AlertDialog(onDismissRequest = { connectionDetails = null },
            title = { Text(tr("Public connection diagnostics")) },
            text = { Text(evidence, fontFamily = FontFamily.Monospace, fontSize = 10.sp,
                modifier = Modifier.verticalScroll(rememberScrollState())) },
            confirmButton = { TextButton(onClick = { connectionDetails = publicConnectionEvidence(appNetwork, discoveryDiagnostics) }) { Text(tr("Refresh diagnostics")) } },
            dismissButton = { TextButton(onClick = { connectionDetails = null }) { Text(tr("Close diagnostics")) } })
    }
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(bottom = 88.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        item { LanguageSelector() }
        item { AppUpdateCard(update, updateAllowed, onPrepareInstall = onPrepareInstall) }
        item {
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                Column(Modifier.weight(1f)) {
                    Text(tr("Stay online in background"), color = ColorTextPrimary)
                    Text(tr("Keep this phone reachable by paired devices when you leave Weavelo or lock the screen."),
                        color = ColorTextSecondary, fontSize = 12.sp)
                }
                androidx.compose.material3.Switch(checked = keepOnline, enabled = nativeAvailable,
                    onCheckedChange = { `fun`.hackerlife.weavelo.DevicePresenceService.setEnabled(context, it) })
            }
        }
        item { DeviceBadgeSettings() }
        item {
            Text(
                text = tr("GESTURE SYSTEM"),
                color = ColorTextSecondary,
                fontSize = 11.sp,
                fontWeight = FontWeight.Bold,
                fontFamily = FontFamily.Monospace,
                letterSpacing = 1.sp
            )
        }

        item {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(12.dp))
                    .background(ColorSurfaceCard)
                    .border(1.dp, ColorBorderFine, RoundedCornerShape(12.dp))
                    .padding(14.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp)
            ) {
                Text(tr("Direct Touch (Screen-First)"), color = ColorAccentCyan, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                Text(tr("• Single Tap: Mouse Left Click\n• Touch & Drag: Mouse Drag Selection\n• Long Press (380ms): Mouse Right Click\n• Two-Finger Drag: Vertical / Horizontal Scroll\n• Two-Finger Tap: Mouse Right Click"), color = ColorTextSecondary, fontSize = 11.sp, lineHeight = 16.sp)
                Spacer(modifier = Modifier.height(4.dp))
                Text(tr("Virtual Trackpad"), color = ColorAccentCyan, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                Text(tr("• Single Finger Glide: Relative Cursor Move\n• Single Tap: Left Click\n• Two-Finger Tap: Right Click\n• Two-Finger Glide: Wheel Scroll"), color = ColorTextSecondary, fontSize = 11.sp, lineHeight = 16.sp)
            }
        }

        item {
            TextButton(onClick = { showAdvancedDiagnostics = !showAdvancedDiagnostics }) {
                Text(
                    if (showAdvancedDiagnostics) tr("Hide advanced runtime & network diagnostics")
                    else tr("Show advanced runtime & network diagnostics")
                )
            }
        }

        if (showAdvancedDiagnostics) {
            item {
                Text(
                    text = tr("RUNTIME & ENGINE"),
                    color = ColorTextSecondary,
                    fontSize = 11.sp,
                    fontWeight = FontWeight.Bold,
                    fontFamily = FontFamily.Monospace,
                    letterSpacing = 1.sp
                )
            }

            item {
                Column(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(12.dp))
                        .background(ColorSurfaceCard)
                        .border(1.dp, ColorBorderFine, RoundedCornerShape(12.dp))
                        .padding(14.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp)
                ) {
                    TelemetryLaneRow(tr("Bridge Status"), if (nativeAvailable) tr("Loaded (.so ready)") else tr("Missing native library"), if (nativeAvailable) ColorAccentEmerald else Color(0xFFB42318))
                    TelemetryLaneRow(tr("Session State"), sessionLabel(sessionState), ColorAccentCyan)
                    TelemetryLaneRow(tr("Decoders"), tr("Hardware MediaCodec (H.264/HEVC)"), ColorTextPrimary)
                    TelemetryLaneRow(tr("Audio Engine"), tr("AudioTrack + Opus Decoder"), ColorTextPrimary)
                }
            }

            item {
                Column(
                    modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp))
                        .background(ColorSurfaceCard).border(1.dp, ColorBorderFine, RoundedCornerShape(12.dp)).padding(14.dp),
                    verticalArrangement = Arrangement.spacedBy(6.dp)
                ) {
                    val status = discoveryDiagnostics
                    Text(tr("CONNECTION STATUS"), color = ColorTextSecondary, fontSize = 11.sp, fontWeight = FontWeight.Bold)
                    TextButton(onClick = { connectionDetails = publicConnectionEvidence(appNetwork, status) }) { Text(tr("Connection details")) }
                    TextButton(onClick = { WeaveloClient.checkRelayTransport() }) { Text(tr("Check relay transport")) }
                    TelemetryLaneRow(tr("Discovery"), status.phase, ColorAccentCyan)
                    TelemetryLaneRow(tr("Runtime network"), status.networkHash.take(12).ifEmpty { tr("Starting") }, ColorTextPrimary)
                    TelemetryLaneRow(tr("This device"), status.deviceHash.take(12).ifEmpty { tr("Starting") }, ColorTextPrimary)
                    TelemetryLaneRow(tr("Configuration"), trFormat("%d/%d · run %d", status.reloadApplied, status.reloadRequested, status.generation), ColorTextPrimary)
                    TelemetryLaneRow(tr("Routes / Devices"), "${status.routes} / ${status.peers}", ColorTextPrimary)
                    TelemetryLaneRow(tr("Direct discovery"), status.p2p, ColorTextPrimary)
                    for ((name, relay) in listOf("Control relay" to status.control, "Discovery relay" to status.discovery)) {
                        TelemetryLaneRow(tr(name), relay.state, ColorAccentCyan)
                        TelemetryLaneRow(tr("Connection attempts"), relay.attempts.toString(), ColorTextPrimary)
                        TelemetryLaneRow(tr("Registration writes / Pongs"), "${relay.registrationsSent} / ${relay.pongsReceived}", ColorTextPrimary)
                        if (relay.error.isNotEmpty()) TelemetryLaneRow(tr("Relay reason"), relay.error, ColorTextSecondary)
                    }
                    if (status.failure.isNotEmpty()) TelemetryLaneRow(tr("Discovery reason"), status.failure, ColorTextSecondary)
                    Text(tr("Registration writes are local sends. Pongs confirm a relay response; the device list confirms discovery."), color = ColorTextSecondary, fontSize = 11.sp)
                }
            }
        }
    }
}

@Composable
private fun deviceCapabilities(device: HostDevice): String = buildList {
    if (device.canStream) add(tr("Screen sharing"))
    if (device.fileTransfer) add(tr("Files"))
    if (device.canView) add(tr("Can view other devices"))
    if (!device.hasService) add(tr("No incoming service active"))
}.joinToString(" · ").ifBlank { tr("No supported action advertised") }

@Composable
private fun sessionLabel(state: SessionState): String = when (state) {
    SessionState.DISCONNECTED -> tr("Idle · Ready")
    SessionState.CONNECTING -> tr("Connecting…")
    SessionState.STREAMING -> tr("Streaming")
    SessionState.RECONNECTING -> tr("Reconnecting…")
    SessionState.ERROR -> tr("Error")
}
