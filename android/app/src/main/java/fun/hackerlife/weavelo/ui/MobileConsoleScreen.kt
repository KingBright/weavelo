package `fun`.hackerlife.weavelo.ui
import androidx.compose.material3.TextButton
import androidx.compose.material3.AlertDialog

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
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
import `fun`.hackerlife.weavelo.TelemetrySnapshot
import `fun`.hackerlife.weavelo.DiscoveryDiagnostics
import `fun`.hackerlife.weavelo.AppNetworkStatus
import `fun`.hackerlife.weavelo.WeaveloClient
import `fun`.hackerlife.weavelo.publicConnectionEvidence
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import `fun`.hackerlife.weavelo.SessionState

// Obsidian Stream 统一设计系统色彩定义
val ColorBaseBg = Color(0xFF101318)
val ColorSurfaceCard = Color(0xFF191E26)
val ColorBorderFine = Color(0x20FFFFFF)
val ColorAccentCyan = Color(0xFF91D4EA)
val ColorAccentEmerald = Color(0xFF93E6C1)
val ColorTextPrimary = Color(0xFFEAF0F6)
val ColorTextSecondary = Color(0xFF97A5B5)

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
    onConnectHost: (String, String) -> Unit,
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
    if (pairingCodeOpen) AlertDialog(onDismissRequest = { pairingCode = ""; pairingCodeOpen = false }, title = { Text("Join with invitation") },
        text = { OutlinedTextField(pairingCode, { pairingCode = it }, label = { Text("Invitation from an existing network member") }) },
        dismissButton = { TextButton(onClick = { pairingCode = ""; pairingCodeOpen = false }) { Text("Cancel") } },
        confirmButton = { TextButton(onClick = { onJoinCode(pairingCode.trim()); pairingCode = ""; pairingCodeOpen = false }, enabled = nativeAvailable && !joiningNetwork && pairingCode.isNotBlank()) { Text("Join") } })
    var activeTab by remember { mutableStateOf("Devices") }
    var manualEndpoint by remember { mutableStateOf("") }
    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(ColorBaseBg)
            .safeDrawingPadding()
            .padding(horizontal = 12.dp, vertical = 6.dp)
    ) {
        Column(
            modifier = Modifier.fillMaxSize()
        ) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically) {
                Text("Weavelo", color = ColorTextPrimary, fontSize = 22.sp, fontWeight = FontWeight.SemiBold)
                MobileAction(MobileSymbol.Network, "Pair devices", nativeAvailable,
                    modifier = Modifier.size(48.dp), onClick = { activeTab = "Mesh P2P" })
            }
            Spacer(Modifier.height(8.dp))

            if (activeTab == "Mesh P2P" || devices.isEmpty()) {
            // 2. 快捷动作按钮栏
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(10.dp)
            ) {
                QuickActionButton(
                    text = "Scan to join network",
                    modifier = Modifier.weight(1f),
                    enabled = nativeAvailable,
                    onClick = onScanQr
                )
                QuickActionButton(
                    text = "Enter invitation",
                    modifier = Modifier.weight(1f),
                    enabled = nativeAvailable,
                    onClick = { pairingCodeOpen = true }
                )
            }
            TextButton(onClick = { invitationOpen = true }, enabled = nativeAvailable && !joiningNetwork) {
                Text("Show invitation QR", color = ColorAccentCyan)
            }
            if (networkName.isNotBlank()) {
                Text("Network: $networkName", color = ColorTextSecondary, fontSize = 11.sp,
                    maxLines = 1, overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.padding(top = 8.dp))
            }

            if (!pairingMessage.isNullOrBlank()) {
                Text(
                    text = pairingMessage,
                    color = if (pairingMessage.startsWith("error:")) Color(0xFFFF8A80) else ColorAccentEmerald,
                    fontSize = 12.sp,
                    modifier = Modifier.padding(vertical = 8.dp)
                )
            }

            }
            Spacer(modifier = Modifier.height(20.dp))

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
                        onConnectHost = onConnectHost,
                        onConnectWorkspace = onConnectWorkspace,
                        onSendFiles = onSendFiles,
                        localStatus = when {
                            !nativeAvailable -> "Connection service unavailable"
                            joiningNetwork -> "Joining network…"
                            outgoing.unattributedSharing > 0 -> "Sharing · ${outgoing.unattributedSharing} direct streams"
                            publishing && !controlReady -> "Sharing ready · review control permission"
                            publishing -> "Sharing ready"
                            else -> "Sharing & permissions"
                        },
                        onLocalSettings = { activeTab = "Sharing" },
                        onLocalDetails = { activeTab = "Mesh P2P" },
                        outgoing = outgoing, onSendScreen = onSendScreen, onSendCamera = onSendCamera, onStopSending = onStopSending
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
                        onBrowseFiles = onBrowseFiles
                    )
                    "Settings" -> SettingsTabContent(
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
            text = text,
            color = ColorTextPrimary,
            fontSize = 12.sp,
            fontWeight = FontWeight.SemiBold
        )
    }
}

@Composable
fun HostCard(name: String, tag: String, subtext: String, enabled: Boolean = true,
    canSendFiles: Boolean = false, onSendFiles: () -> Unit = {},
    canSendScreen: Boolean = false, sending: Boolean = false, onSendScreen: () -> Unit = {},
    canSendCamera: Boolean = false, onSendCamera: () -> Unit = {},
    details: String = "", onEnd: (() -> Unit)? = null, onConnect: () -> Unit) {
    var sendMenu by remember(name) { mutableStateOf(false) }
    var expanded by remember(name) { mutableStateOf(false) }
    Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp))
        .background(ColorSurfaceCard).border(1.dp, ColorBorderFine, RoundedCornerShape(12.dp))
        .clickable(enabled = enabled, onClick = onConnect).padding(horizontal = 10.dp, vertical = 6.dp)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            androidx.compose.material3.Icon(MobileSymbol.Devices.image, contentDescription = null,
                tint = ColorAccentEmerald, modifier = Modifier.size(20.dp))
            Text(name, modifier = Modifier.weight(1f), color = ColorTextPrimary, fontSize = 15.sp,
                lineHeight = 20.sp, fontWeight = FontWeight.SemiBold, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(tag, color = ColorAccentEmerald, fontSize = 10.sp, lineHeight = 14.sp)
                Text(subtext, color = ColorTextSecondary, fontSize = 11.sp, lineHeight = 15.sp, maxLines = 2,
                    overflow = TextOverflow.Ellipsis)
            }
            Box {
                MobileAction(if (sending) MobileSymbol.Close else MobileSymbol.Share,
                    if (sending) "Stop sending to $name" else "Send to $name", canSendScreen || canSendCamera,
                    modifier = Modifier.size(48.dp), onClick = { if (sending) onSendScreen() else sendMenu = true })
                androidx.compose.material3.DropdownMenu(expanded = sendMenu && !sending,
                    onDismissRequest = { sendMenu = false }) {
                    androidx.compose.material3.DropdownMenuItem(text = { Text("Screen or app") },
                        enabled = canSendScreen, leadingIcon = {
                            androidx.compose.material3.Icon(MobileSymbol.Devices.image, contentDescription = null)
                        }, onClick = { sendMenu = false; onSendScreen() })
                    androidx.compose.material3.DropdownMenuItem(text = { Text("Camera") },
                        enabled = canSendCamera, leadingIcon = {
                            androidx.compose.material3.Icon(MobileSymbol.Camera.image, contentDescription = null)
                        }, onClick = { sendMenu = false; onSendCamera() })
                }
            }
            MobileAction(MobileSymbol.Open, "Open device $name", enabled, modifier = Modifier.size(48.dp), onClick = onConnect)
            MobileAction(MobileSymbol.Files, "Send files to $name", canSendFiles, modifier = Modifier.size(48.dp), onClick = onSendFiles)
            MobileAction(MobileSymbol.More, "Device details $name", true, modifier = Modifier.size(48.dp), onClick = { expanded = !expanded })
        }
        if (expanded) {
            Text(details.ifBlank { subtext }, color = ColorTextSecondary, fontSize = 11.sp, lineHeight = 16.sp,
                modifier = Modifier.padding(vertical = 4.dp))
            onEnd?.let { end -> TextButton(onClick = end) { Text("End streams") } }
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
        "Sharing" -> MobileSymbol.Share
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
        Text(text, color = if (active) ColorTextPrimary else ColorTextSecondary, fontSize = 11.sp, lineHeight = 16.sp,
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
    onConnectHost: (String, String) -> Unit,
    onConnectWorkspace: (String, String) -> Unit,
    onSendFiles: (String, String) -> Unit,
    localStatus: String,
    onLocalSettings: () -> Unit,
    onLocalDetails: () -> Unit,
    outgoing: `fun`.hackerlife.weavelo.OutgoingScreenState,
    onSendScreen: (String) -> Unit,
    onSendCamera: (String) -> Unit,
    onStopSending: (Long) -> Unit
) {
    val workspace by `fun`.hackerlife.weavelo.WorkspaceClient.state.collectAsState()
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        verticalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        item {
            Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(ColorSurfaceCard)
                .padding(start = 10.dp, end = 2.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f).padding(vertical = 8.dp)) {
                    Text(android.os.Build.MODEL, color = ColorTextPrimary, fontSize = 13.sp, fontWeight = FontWeight.SemiBold)
                    Text(localStatus, color = ColorTextSecondary, fontSize = 11.sp, lineHeight = 15.sp)
                }
                MobileAction(MobileSymbol.Share, "Sharing & permissions", true, modifier = Modifier.size(48.dp), onClick = onLocalSettings)
                MobileAction(MobileSymbol.More, "This device & pairing", true, modifier = Modifier.size(48.dp), onClick = onLocalDetails)
            }
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
                        discoveryDiagnostics.emptyMessage,
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
                val streamError = if (receiving) workspace.panes.firstNotNullOfOrNull { it.videoError ?: it.inputError } else null
                val state = buildList {
                    if (sharing > 0) add("Sharing · $sharing")
                    if (streams > 0) add("Receiving · $streams")
                    if (outgoing.active && outgoing.target == device.id && sharing == 0) add("Connecting…")
                    if (receiving && workspace.transferring) add("Transferring files…")
                    if (streamError != null) add("Needs attention")
                    if (!device.hasService) add("No shared sources")
                }.joinToString(" · ")
                HostCard(
                    name = device.name.ifBlank { device.id },
                    tag = device.scope,
                    subtext = state.ifBlank { "Available" },
                    details = listOfNotNull(streamError, if (outgoing.target == device.id) outgoing.status.takeIf { it.isNotBlank() } else null,
                        device.capabilitySummary, device.endpoint, device.id).joinToString("\n"),
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
                Text(if (advancedConnection) "Hide manual connection" else "Advanced: connect by address")
            }
            if (advancedConnection) {
                OutlinedTextField(
                    value = manualEndpoint,
                    onValueChange = onManualEndpointChange,
                    label = { Text("Host IP:port") },
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
                        onClick = { onConnectHost(manualEndpoint.trim(), manualEndpoint.trim()) }
                    ) { Text("Connect to address") }

                    TextButton(
                        onClick = { onConnectWorkspace("", manualEndpoint.trim()) },
                        enabled = nativeAvailable && manualEndpoint.isNotBlank()
                    ) { Text("Open workspace") }
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
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        item {
            Text(
                text = "PRIVATE MESH TOPOLOGY",
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
                        text = networkName.ifBlank { "Personal Device Mesh" },
                        color = ColorTextPrimary,
                        fontSize = 15.sp,
                        fontWeight = FontWeight.Bold
                    )
                    Box(
                        modifier = Modifier
                            .clip(RoundedCornerShape(9999.dp))
                            .background(Color(0xFF132832))
                            .border(1.dp, ColorAccentCyan, RoundedCornerShape(9999.dp))
                            .padding(horizontal = 8.dp, vertical = 3.dp)
                    ) {
                        Text(
                            text = when {
                                joiningNetwork -> "Joining..."
                                nativeAvailable -> "Active · P2P"
                                else -> "Offline"
                            },
                            color = ColorAccentCyan,
                            fontSize = 10.sp,
                            fontFamily = FontFamily.Monospace,
                            fontWeight = FontWeight.Bold
                        )
                    }
                }
                Text(
                    text = "${devices.size} linked peers in private overlay network",
                    color = ColorTextSecondary,
                    fontSize = 12.sp
                )
                Text(
                    text = "Encrypted point-to-point tunnels with zero-config hole punching.",
                    color = ColorTextSecondary,
                    fontSize = 11.sp
                )
            }
        }

        item {
            Text(
                text = "LINKED PEERS",
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
                        "No peers linked yet. Scan a QR invitation or enter a join code from another device to expand your mesh.",
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
                                .background(Color(0xFF22262E))
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
                        text = "Endpoint: ${device.endpoint.ifBlank { "Auto-negotiated UDP" }}",
                        color = ColorTextSecondary,
                        fontSize = 11.sp,
                        fontFamily = FontFamily.Monospace
                    )
                    Text(
                        text = device.capabilitySummary,
                        color = ColorTextSecondary,
                        fontSize = 11.sp
                    )
                    if (device.canOpenWorkspace) {
                        TextButton(
                            onClick = { onConnectWorkspace(device.id, device.endpoint) },
                            enabled = nativeAvailable && !joiningNetwork,
                            modifier = Modifier.align(Alignment.End)
                        ) { Text("Inspect peer") }
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
                    text = "ROUTING & SECURITY POLICY",
                    color = ColorTextPrimary,
                    fontSize = 12.sp,
                    fontWeight = FontWeight.Bold
                )
                Text(
                    text = "• Local Subnet: Direct UDP\n• WAN Cross-NAT: STUN UDP Hole Punching\n• Fallback: Authenticated end-to-end encrypted relay",
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
    onBrowseFiles: () -> Unit
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
        title = { Text("Connection security") },
        text = { Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            TelemetryLaneRow("Transport cipher", "ChaCha20-Poly1305", ColorAccentCyan)
            TelemetryLaneRow("Network access", "Invitation required", ColorAccentEmerald)
            TelemetryLaneRow("Authentication", "Paired device group", ColorAccentCyan)
            TelemetryLaneRow("Storage", "App private", ColorTextSecondary)
        } }, confirmButton = { TextButton(onClick = { securityDetails = false }) { Text("Done") } })
    LazyColumn(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item {
            Text("Sharing & permissions", color = ColorTextPrimary, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
            Text("Share a screen, app or camera with your devices.",
                color = ColorTextSecondary, fontSize = 12.sp, modifier = Modifier.padding(top = 6.dp))
        }
        item {
            Column(Modifier.fillMaxWidth().background(ColorSurfaceCard, RoundedCornerShape(18.dp))
                .border(1.dp, ColorBorderFine, RoundedCornerShape(18.dp)).padding(horizontal = 14.dp, vertical = 6.dp)) {
                SharingPermissionRow(MobileSymbol.Share, "Screen or app", if (publishing) "Sharing" else "Not sharing",
                    if (publishing) "Stop" else "Share", nativeAvailable) {
                    if (publishing) onStopSharing() else onShareScreen(false)
                }
                SharingPermissionRow(MobileSymbol.Camera, "Camera", camera.status,
                    if (camera.enabled) "Stop" else "Share", nativeAvailable) {
                    if (camera.enabled) `fun`.hackerlife.weavelo.CameraPublisher.stop(context) else onShareCamera()
                }
                SharingPermissionRow(MobileSymbol.Control, "Remote control", if (controlReady) "Allowed" else "Needs permission",
                    if (controlReady) "Settings" else "Allow", nativeAvailable, onControlSettings)
                SharingPermissionRow(MobileSymbol.Audio, "Audio", if (audioAllowed) "Microphone allowed" else "Needs permission",
                    "Share with audio", nativeAvailable && !publishing) { onShareScreen(true) }
                SharingPermissionRow(MobileSymbol.Notifications, "Notifications", if (notificationsAllowed) "Allowed" else "Off",
                    "Settings", true) {
                    context.startActivity(android.content.Intent(android.provider.Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                        .putExtra(android.provider.Settings.EXTRA_APP_PACKAGE, context.packageName))
                }
                if (controlReady) TextButton(onClick = onShareControlledScreen, enabled = nativeAvailable && !publishing,
                    modifier = Modifier.fillMaxWidth()) { Text("Share with control and audio") }
            }
        }
        item {
            Column(Modifier.fillMaxWidth().background(ColorSurfaceCard, RoundedCornerShape(18.dp))
                .border(1.dp, ColorBorderFine, RoundedCornerShape(18.dp)).padding(horizontal = 14.dp, vertical = 6.dp)) {
                SharingPermissionRow(MobileSymbol.Files, "Receive files",
                    if (receiverStarting) "Starting…" else if (receivingFiles) "On" else "Off",
                    if (receivingFiles) "Stop" else "Start", nativeAvailable && !receiverStarting) {
                    if (receivingFiles) onStopReceiving() else onStartReceiving()
                }
                TextButton(onClick = onBrowseFiles, modifier = Modifier.fillMaxWidth()) { Text("Browse received files") }
            }
        }
        item { TextButton(onClick = { securityDetails = true }) { Text("Connection security") } }
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
            Text(title, color = ColorTextPrimary, fontSize = 14.sp, fontWeight = FontWeight.Medium)
            Text(state, color = ColorTextSecondary, fontSize = 11.sp)
        }
        TextButton(onClick = onClick, enabled = enabled) { Text(action, fontSize = 12.sp) }
    }
}

@Composable
private fun SettingsTabContent(
    appNetwork: AppNetworkStatus,
    nativeAvailable: Boolean,
    discoveryDiagnostics: DiscoveryDiagnostics,
    sessionState: SessionState
) {
    var connectionDetails by remember { mutableStateOf<String?>(null) }
    connectionDetails?.let { evidence ->
        AlertDialog(onDismissRequest = { connectionDetails = null },
            title = { Text("Public connection diagnostics") },
            text = { Text(evidence, fontFamily = FontFamily.Monospace, fontSize = 10.sp,
                modifier = Modifier.verticalScroll(rememberScrollState())) },
            confirmButton = { TextButton(onClick = { connectionDetails = publicConnectionEvidence(appNetwork, discoveryDiagnostics) }) { Text("Refresh diagnostics") } },
            dismissButton = { TextButton(onClick = { connectionDetails = null }) { Text("Close diagnostics") } })
    }
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        item {
            Text(
                text = "RUNTIME & ENGINE",
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
                TelemetryLaneRow("Bridge Status", if (nativeAvailable) "Loaded (.so ready)" else "Missing native library", if (nativeAvailable) ColorAccentEmerald else Color(0xFFFF8A80))
                TelemetryLaneRow("Session State", sessionState.name, ColorAccentCyan)
                TelemetryLaneRow("Decoders", "Hardware MediaCodec (H.264/HEVC)", ColorTextPrimary)
                TelemetryLaneRow("Audio Engine", "AudioTrack + Opus Decoder", ColorTextPrimary)
            }
        }

        item {
            Column(
                modifier = Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp))
                    .background(ColorSurfaceCard).border(1.dp, ColorBorderFine, RoundedCornerShape(12.dp)).padding(14.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp)
            ) {
                val status = discoveryDiagnostics
                Text("CONNECTION STATUS", color = ColorTextSecondary, fontSize = 11.sp, fontWeight = FontWeight.Bold)
                TextButton(onClick = { connectionDetails = publicConnectionEvidence(appNetwork, status) }) { Text("Connection details") }
                TextButton(onClick = { WeaveloClient.checkRelayTransport() }) { Text("Check relay transport") }
                TelemetryLaneRow("Discovery", status.phase, ColorAccentCyan)
                TelemetryLaneRow("Runtime network", status.networkHash.take(12).ifEmpty { "Starting" }, ColorTextPrimary)
                TelemetryLaneRow("This device", status.deviceHash.take(12).ifEmpty { "Starting" }, ColorTextPrimary)
                TelemetryLaneRow("Configuration", "${status.reloadApplied}/${status.reloadRequested} · run ${status.generation}", ColorTextPrimary)
                TelemetryLaneRow("Routes / Devices", "${status.routes} / ${status.peers}", ColorTextPrimary)
                TelemetryLaneRow("Direct discovery", status.p2p, ColorTextPrimary)
                for ((name, relay) in listOf("Control relay" to status.control, "Discovery relay" to status.discovery)) {
                    TelemetryLaneRow(name, relay.state, ColorAccentCyan)
                    TelemetryLaneRow("Connection attempts", relay.attempts.toString(), ColorTextPrimary)
                    TelemetryLaneRow("Registration writes / Pongs", "${relay.registrationsSent} / ${relay.pongsReceived}", ColorTextPrimary)
                    if (relay.error.isNotEmpty()) TelemetryLaneRow("Relay reason", relay.error, ColorTextSecondary)
                }
                if (status.failure.isNotEmpty()) TelemetryLaneRow("Discovery reason", status.failure, ColorTextSecondary)
                Text("Registration writes are local sends. Pongs confirm a relay response; the device list confirms discovery.", color = ColorTextSecondary, fontSize = 11.sp)
            }
        }

        item {
            Text(
                text = "GESTURE SYSTEM",
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
                Text("Direct Touch (Screen-First)", color = ColorAccentCyan, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                Text("• Single Tap: Mouse Left Click\n• Touch & Drag: Mouse Drag Selection\n• Long Press (380ms): Mouse Right Click\n• Two-Finger Drag: Vertical / Horizontal Scroll\n• Two-Finger Tap: Mouse Right Click", color = ColorTextSecondary, fontSize = 11.sp, lineHeight = 16.sp)
                Spacer(modifier = Modifier.height(4.dp))
                Text("Virtual Trackpad", color = ColorAccentCyan, fontSize = 12.sp, fontWeight = FontWeight.Bold)
                Text("• Single Finger Glide: Relative Cursor Move\n• Single Tap: Left Click\n• Two-Finger Tap: Right Click\n• Two-Finger Glide: Wheel Scroll", color = ColorTextSecondary, fontSize = 11.sp, lineHeight = 16.sp)
            }
        }
    }
}
