package `fun`.hackerlife.weavelo.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import `fun`.hackerlife.weavelo.AppNetworkStatus
import `fun`.hackerlife.weavelo.DeviceArtwork
import `fun`.hackerlife.weavelo.DeviceBadgePreset
import `fun`.hackerlife.weavelo.DiscoveryDiagnostics
import `fun`.hackerlife.weavelo.HostDevice
import `fun`.hackerlife.weavelo.SessionState
import `fun`.hackerlife.weavelo.TelemetrySnapshot
import `fun`.hackerlife.weavelo.WeaveloClient
import `fun`.hackerlife.weavelo.publicConnectionEvidence

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
    val p = WeaveloTokens.colors
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
    if (pairingCodeOpen) {
        PrecisionDialog(
            title = tr("Join with invitation"),
            onDismissRequest = { pairingCode = ""; pairingCodeOpen = false },
            icon = MobileSymbol.Invitation,
            dismissButton = {
                PrecisionButton(
                    text = tr("Cancel"),
                    onClick = { pairingCode = ""; pairingCodeOpen = false },
                    style = PrecisionButtonStyle.SUBTLE,
                    compact = true
                )
            },
            confirmButton = {
                PrecisionButton(
                    text = tr("Join"),
                    enabled = nativeAvailable && !joiningNetwork && pairingCode.isNotBlank(),
                    style = PrecisionButtonStyle.PRIMARY,
                    compact = true,
                    onClick = {
                        onJoinCode(pairingCode.trim())
                        pairingCode = ""
                        pairingCodeOpen = false
                    }
                )
            }
        ) {
            PrecisionTextField(
                value = pairingCode,
                onValueChange = { pairingCode = it },
                label = tr("Invitation from an existing network member")
            )
        }
    }
    var activeTab by remember { mutableStateOf("Devices") }
    val update = rememberAppUpdate()
    var manualEndpoint by remember { mutableStateOf("") }
    Box(
        modifier = Modifier
            .fillMaxSize()
            .background(p.canvas)
            .safeDrawingPadding()
            .padding(horizontal = 14.dp, vertical = 8.dp)
    ) {
        val context = androidx.compose.ui.platform.LocalContext.current
        val cameraState by `fun`.hackerlife.weavelo.CameraPublisher.state.collectAsState()
        val workspaceState by `fun`.hackerlife.weavelo.WorkspaceClient.state.collectAsState()
        Column(modifier = Modifier.fillMaxSize()) {
            // Top Brand Bar
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(vertical = 4.dp),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically
            ) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(10.dp)
                ) {
                    WeaveloBrandEmblem(size = 30.dp)
                    Column(verticalArrangement = Arrangement.spacedBy(1.dp)) {
                        Text(
                            text = "Weavelo",
                            color = p.textPrimary,
                            fontSize = 20.sp,
                            fontWeight = FontWeight.Bold,
                            letterSpacing = (-0.3).sp
                        )
                        if (networkName.isNotBlank()) {
                            Text(
                                text = networkName,
                                color = p.textTertiary,
                                fontSize = 11.sp,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis
                            )
                        }
                    }
                }
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp)
                ) {
                    if (update.release != null && activeTab != "Settings") {
                        PrecisionButton(
                            text = tr("Update available"),
                            onClick = { activeTab = "Settings" },
                            style = PrecisionButtonStyle.TONAL,
                            compact = true
                        )
                    }
                    PrecisionButton(
                        text = tr("Pair devices"),
                        leadingSymbol = MobileSymbol.Network,
                        enabled = nativeAvailable,
                        style = if (activeTab == "Mesh P2P") PrecisionButtonStyle.PRIMARY else PrecisionButtonStyle.SUBTLE,
                        compact = true,
                        onClick = { activeTab = "Mesh P2P" }
                    )
                }
            }

            // Active Outgoing Sharing Banner
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
                Spacer(Modifier.height(8.dp))
                Row(
                    Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(14.dp))
                        .background(p.warningSoft)
                        .border(1.dp, p.warningBorder, RoundedCornerShape(14.dp))
                        .padding(horizontal = 12.dp, vertical = 10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(10.dp)
                ) {
                    Box(
                        modifier = Modifier
                            .size(34.dp)
                            .clip(RoundedCornerShape(10.dp))
                            .background(p.warningBorder.copy(alpha = 0.2f)),
                        contentAlignment = Alignment.Center
                    ) {
                        Icon(MobileSymbol.Share.image, contentDescription = null, tint = p.warningText, modifier = Modifier.size(18.dp))
                    }
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        Text(bannerTitle, color = p.warningText, fontSize = 13.sp, fontWeight = FontWeight.SemiBold)
                        Text(bannerSub, color = p.warningText.copy(alpha = 0.9f), fontSize = 11.5.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    }
                    PrecisionButton(
                        text = tr("Stop now"),
                        style = PrecisionButtonStyle.DANGER,
                        compact = true,
                        onClick = {
                            if (outgoing.active) onStopSending(outgoing.id)
                            if (publishing) onStopSharing()
                            if (cameraState.enabled) `fun`.hackerlife.weavelo.CameraPublisher.stop(context)
                        }
                    )
                }
            }

            // Quick Pairing Actions Card (shown on Network tab or when mesh is empty)
            if (activeTab == "Mesh P2P" || devices.isEmpty()) {
                Spacer(Modifier.height(10.dp))
                PrecisionCard(
                    cornerRadius = 16.dp,
                    contentPadding = PaddingValues(12.dp)
                ) {
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.spacedBy(8.dp)
                        ) {
                            QuickActionButton(
                                text = tr("Scan to join network"),
                                symbol = MobileSymbol.Scan,
                                modifier = Modifier.weight(1f),
                                enabled = nativeAvailable,
                                primary = true,
                                onClick = onScanQr
                            )
                            QuickActionButton(
                                text = tr("Enter invitation"),
                                symbol = MobileSymbol.Invitation,
                                modifier = Modifier.weight(1f),
                                enabled = nativeAvailable,
                                primary = false,
                                onClick = { pairingCodeOpen = true }
                            )
                        }
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            PrecisionButton(
                                text = tr("Show invitation QR"),
                                leadingSymbol = MobileSymbol.Network,
                                enabled = nativeAvailable && !joiningNetwork,
                                style = PrecisionButtonStyle.TONAL,
                                compact = true,
                                onClick = { invitationOpen = true }
                            )
                            if (networkName.isNotBlank()) {
                                PrecisionPillBadge(
                                    text = trFormat("Network: %s", networkName),
                                    tone = PillTone.NEUTRAL
                                )
                            }
                        }
                        if (!pairingMessage.isNullOrBlank()) {
                            val isErr = pairingMessage.startsWith("error:")
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clip(RoundedCornerShape(10.dp))
                                    .background(if (isErr) p.dangerSoft else p.emeraldSoft)
                                    .border(1.dp, if (isErr) p.dangerBorder else p.emeraldDot.copy(alpha = 0.4f), RoundedCornerShape(10.dp))
                                    .padding(horizontal = 10.dp, vertical = 7.dp),
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                Text(
                                    text = pairingMessage.removePrefix("error:"),
                                    color = if (isErr) p.dangerText else p.emeraldText,
                                    fontSize = 11.5.sp,
                                    lineHeight = 16.sp
                                )
                            }
                        }
                    }
                }
            }

            Spacer(modifier = Modifier.height(10.dp))

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
                        outgoing = outgoing,
                        onSendScreen = onSendScreen,
                        onSendCamera = onSendCamera,
                        onStopSending = onStopSending,
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

            // Bottom Floating Recessed Segmented Dock
            Surface(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(top = 8.dp)
                    .border(1.dp, p.borderStrong.copy(alpha = if (p.isDark) 0.9f else 0.6f), RoundedCornerShape(24.dp)),
                shape = RoundedCornerShape(24.dp),
                color = p.surfaceGlass,
                shadowElevation = 8.dp
            ) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 6.dp, vertical = 5.dp),
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    NavTab("Devices", active = activeTab == "Devices", modifier = Modifier.weight(1f)) { activeTab = "Devices" }
                    NavTab("Network", active = activeTab == "Mesh P2P", modifier = Modifier.weight(1f)) { activeTab = "Mesh P2P" }
                    NavTab("Sharing", active = activeTab == "Security", modifier = Modifier.weight(1f)) { activeTab = "Security" }
                    NavTab("Settings", active = activeTab == "Settings", modifier = Modifier.weight(1f)) { activeTab = "Settings" }
                }
            }
        }
    }
}

@Composable
fun QuickActionButton(
    text: String,
    modifier: Modifier = Modifier,
    symbol: MobileSymbol = MobileSymbol.Scan,
    enabled: Boolean = true,
    primary: Boolean = false,
    onClick: () -> Unit
) {
    PrecisionButton(
        text = text,
        leadingSymbol = symbol,
        enabled = enabled,
        style = if (primary) PrecisionButtonStyle.PRIMARY else PrecisionButtonStyle.SUBTLE,
        modifier = modifier,
        onClick = onClick
    )
}

@Composable
fun HostCard(
    name: String,
    tag: String,
    subtext: String,
    enabled: Boolean = true,
    disabledHint: String? = null,
    deviceBadge: DeviceBadgePreset = DeviceBadgePreset.COMPUTER,
    canSendFiles: Boolean = false,
    onSendFiles: () -> Unit = {},
    canSendScreen: Boolean = false,
    sending: Boolean = false,
    onSendScreen: () -> Unit = {},
    canSendCamera: Boolean = false,
    onSendCamera: () -> Unit = {},
    details: String = "",
    hasActiveStream: Boolean = false,
    onEnd: (() -> Unit)? = null,
    onConnect: () -> Unit
) {
    val p = WeaveloTokens.colors
    val context = androidx.compose.ui.platform.LocalContext.current
    var sendMenu by remember(name) { mutableStateOf(false) }
    var expanded by remember(name) { mutableStateOf(false) }
    val badge = remember(deviceBadge) { DeviceArtwork.badge(deviceBadge).asImageBitmap() }
    val activeStream = hasActiveStream || sending || onEnd != null || subtext.contains("Receiving") || subtext.contains("Sharing")
    val tagTone = when (tag.uppercase(java.util.Locale.ROOT)) {
        "LAN" -> PillTone.EMERALD
        "P2P", "DIRECT" -> PillTone.CYAN
        "RELAY" -> PillTone.BRAND
        else -> PillTone.NEUTRAL
    }

    PrecisionCard(
        highlighted = activeStream,
        cornerRadius = 18.dp,
        contentPadding = PaddingValues(horizontal = 14.dp, vertical = 12.dp),
        enabled = enabled || disabledHint != null,
        onClick = {
            if (enabled) onConnect()
            else if (disabledHint != null) android.widget.Toast.makeText(context, disabledHint, android.widget.Toast.LENGTH_SHORT).show()
        }
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            // Tier 1: Sculpted Device Badge Well + Name & Status + Route Pill
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(11.dp)
            ) {
                Box(
                    modifier = Modifier
                        .size(42.dp)
                        .clip(RoundedCornerShape(12.dp))
                        .background(if (activeStream) p.surfaceRaised else p.surfaceSunken)
                        .border(1.dp, if (activeStream) p.brandBorder.copy(alpha = 0.6f) else p.borderMuted, RoundedCornerShape(12.dp)),
                    contentAlignment = Alignment.Center
                ) {
                    Image(
                        badge,
                        contentDescription = trFormat("Device type: %s", tr(deviceBadge.displayName)),
                        modifier = Modifier.size(26.dp)
                    )
                }
                Column(
                    modifier = Modifier.weight(1f),
                    verticalArrangement = Arrangement.spacedBy(3.dp)
                ) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        Text(
                            text = name,
                            modifier = Modifier.weight(1f, fill = false),
                            color = p.textPrimary,
                            fontSize = 15.5.sp,
                            lineHeight = 20.sp,
                            fontWeight = FontWeight.SemiBold,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis
                        )
                        if (tag.isNotBlank()) {
                            PrecisionPillBadge(
                                text = tag,
                                tone = tagTone,
                                showDot = enabled || activeStream
                            )
                        }
                    }
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(6.dp)
                    ) {
                        Box(
                            modifier = Modifier
                                .size(6.dp)
                                .background(
                                    when {
                                        activeStream -> p.brand
                                        enabled -> p.emeraldDot
                                        else -> p.textTertiary.copy(alpha = 0.5f)
                                    },
                                    CircleShape
                                )
                        )
                        Text(
                            text = subtext,
                            color = if (activeStream) p.brand else p.textSecondary,
                            fontSize = 12.sp,
                            lineHeight = 16.sp,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis
                        )
                    }
                }
            }

            // Tier 2: Primary Capsule CTA + Tonal Icon Wells
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp)
            ) {
                PrecisionButton(
                    text = tr("View"),
                    leadingSymbol = MobileSymbol.View,
                    enabled = enabled,
                    style = PrecisionButtonStyle.PRIMARY,
                    compact = true,
                    modifier = Modifier.weight(1f),
                    onClick = onConnect
                )
                Box {
                    MobileAction(
                        symbol = if (sending) MobileSymbol.Close else MobileSymbol.Share,
                        label = if (sending) trFormat("Stop sending to %s", name) else trFormat("Send to %s", name),
                        enabled = canSendScreen || canSendCamera,
                        tonalWell = true,
                        destructive = sending,
                        modifier = Modifier.size(42.dp),
                        onClick = { if (sending) onSendScreen() else sendMenu = true }
                    )
                    DropdownMenu(
                        expanded = sendMenu && !sending,
                        onDismissRequest = { sendMenu = false },
                        shape = RoundedCornerShape(16.dp),
                        containerColor = p.surfaceRaised,
                        border = BorderStroke(1.dp, p.borderStrong)
                    ) {
                        Column(
                            modifier = Modifier
                                .widthIn(min = 196.dp)
                                .padding(horizontal = 6.dp, vertical = 2.dp),
                            verticalArrangement = Arrangement.spacedBy(2.dp)
                        ) {
                            PrecisionMenuItem(
                                text = tr("Screen or app"),
                                symbol = MobileSymbol.Devices,
                                enabled = canSendScreen,
                                onClick = { sendMenu = false; onSendScreen() }
                            )
                            PrecisionMenuItem(
                                text = tr("Camera"),
                                symbol = MobileSymbol.Camera,
                                enabled = canSendCamera,
                                onClick = { sendMenu = false; onSendCamera() }
                            )
                        }
                    }
                }
                MobileAction(
                    symbol = MobileSymbol.Files,
                    label = trFormat("Send files to %s", name),
                    enabled = canSendFiles,
                    tonalWell = true,
                    modifier = Modifier.size(42.dp),
                    onClick = onSendFiles
                )
                MobileAction(
                    symbol = MobileSymbol.More,
                    label = trFormat("Device details %s", name),
                    enabled = true,
                    selected = expanded,
                    tonalWell = true,
                    modifier = Modifier.size(42.dp),
                    onClick = { expanded = !expanded }
                )
            }

            if (expanded) {
                HorizontalDivider(color = p.borderMuted)
                Text(
                    text = details.ifBlank { subtext },
                    color = p.textSecondary,
                    fontSize = 11.5.sp,
                    fontFamily = FontFamily.Monospace,
                    lineHeight = 16.sp
                )
                onEnd?.let { end ->
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                        PrecisionButton(
                            text = tr("End streams"),
                            leadingSymbol = MobileSymbol.Disconnect,
                            style = PrecisionButtonStyle.DANGER_TONAL,
                            compact = true,
                            onClick = end
                        )
                    }
                }
            }
        }
    }
}

@Composable
fun TelemetryLaneRow(name: String, value: String, color: Color) {
    val p = WeaveloTokens.colors
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically
    ) {
        Text(text = name, color = p.textPrimary, fontSize = 12.sp, fontWeight = FontWeight.Medium)
        Text(text = value, color = color, fontSize = 11.5.sp, fontFamily = FontFamily.Monospace, fontWeight = FontWeight.Bold)
    }
}

@Composable
fun NavTab(text: String, active: Boolean, modifier: Modifier = Modifier, onClick: () -> Unit = {}) {
    val p = WeaveloTokens.colors
    val symbol = when (text) {
        "Devices" -> MobileSymbol.Devices
        "Network" -> MobileSymbol.Network
        "Sharing" -> MobileSymbol.Permissions
        else -> MobileSymbol.Settings
    }
    Column(
        modifier = modifier
            .heightIn(min = 50.dp)
            .clip(RoundedCornerShape(18.dp))
            .background(if (active) p.brandSoft else Color.Transparent)
            .let { mod ->
                if (active) mod.border(1.dp, p.brandBorder.copy(alpha = 0.55f), RoundedCornerShape(18.dp))
                else mod
            }
            .clickable(role = Role.Tab, onClick = onClick)
            .padding(vertical = 6.dp, horizontal = 4.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(3.dp)
    ) {
        Icon(
            symbol.image,
            contentDescription = null,
            tint = if (active) p.brand else p.textSecondary,
            modifier = Modifier.size(19.dp)
        )
        Text(
            tr(text),
            color = if (active) p.brand else p.textSecondary,
            fontSize = 11.sp,
            lineHeight = 14.sp,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            fontWeight = if (active) FontWeight.SemiBold else FontWeight.Medium
        )
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
    val p = WeaveloTokens.colors
    val workspace by `fun`.hackerlife.weavelo.WorkspaceClient.state.collectAsState()
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp)
    ) {
        // Local Device Card
        item {
            PrecisionCard(
                sunken = true,
                cornerRadius = 16.dp,
                contentPadding = PaddingValues(horizontal = 12.dp, vertical = 10.dp)
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(10.dp)
                ) {
                    Box(
                        modifier = Modifier
                            .size(38.dp)
                            .clip(RoundedCornerShape(11.dp))
                            .background(p.brandSoft)
                            .border(1.dp, p.brandBorder.copy(alpha = 0.5f), RoundedCornerShape(11.dp)),
                        contentAlignment = Alignment.Center
                    ) {
                        Icon(MobileSymbol.Touch.image, contentDescription = null, tint = p.brand, modifier = Modifier.size(19.dp))
                    }
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        Text(
                            android.os.Build.MODEL,
                            color = p.textPrimary,
                            fontSize = 13.5.sp,
                            fontWeight = FontWeight.SemiBold,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis
                        )
                        Text(
                            localStatus,
                            color = p.textSecondary,
                            fontSize = 11.5.sp,
                            lineHeight = 15.sp
                        )
                    }
                    MobileAction(
                        symbol = MobileSymbol.Settings,
                        label = tr("Outgoing screen quality"),
                        enabled = true,
                        tonalWell = true,
                        modifier = Modifier.size(40.dp),
                        onClick = onOutgoingQuality
                    )
                    MobileAction(
                        symbol = MobileSymbol.Permissions,
                        label = tr("Sharing & permissions"),
                        enabled = true,
                        tonalWell = true,
                        modifier = Modifier.size(40.dp),
                        onClick = onLocalSettings
                    )
                    MobileAction(
                        symbol = MobileSymbol.More,
                        label = tr("This device & pairing"),
                        enabled = true,
                        tonalWell = true,
                        modifier = Modifier.size(40.dp),
                        onClick = onLocalDetails
                    )
                }
            }
        }

        if (devices.isEmpty()) {
            item {
                PrecisionCard(cornerRadius = 16.dp) {
                    Text(
                        tr(discoveryDiagnostics.emptyMessage),
                        color = p.textSecondary,
                        fontSize = 13.sp,
                        lineHeight = 19.sp
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
                    details = listOfNotNull(
                        shownError,
                        if (outgoing.target == device.id) outgoing.status.takeIf { it.isNotBlank() }?.let { trFormat("Details: %s", it) } else null,
                        deviceCapabilities(device),
                        device.endpoint,
                        device.id
                    ).joinToString("\n"),
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
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    PrecisionButton(
                        text = if (advancedConnection) tr("Hide manual connection") else tr("Advanced: connect by address"),
                        style = PrecisionButtonStyle.SUBTLE,
                        compact = true,
                        onClick = onToggleAdvanced
                    )
                    PrecisionButton(
                        text = tr("Outgoing screen quality"),
                        leadingSymbol = MobileSymbol.Settings,
                        style = PrecisionButtonStyle.TONAL,
                        compact = true,
                        onClick = onOutgoingQuality
                    )
                }
                if (advancedConnection) {
                    PrecisionCard(cornerRadius = 16.dp) {
                        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                            PrecisionTextField(
                                value = manualEndpoint,
                                onValueChange = onManualEndpointChange,
                                label = tr("Host IP:port")
                            )
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                PrecisionButton(
                                    text = tr("Connect to address"),
                                    leadingSymbol = MobileSymbol.View,
                                    enabled = nativeAvailable && manualEndpoint.isNotBlank() && sessionState != SessionState.CONNECTING,
                                    style = PrecisionButtonStyle.PRIMARY,
                                    onClick = { onConnectAddress(manualEndpoint.trim()) }
                                )
                                PrecisionButton(
                                    text = tr("Open workspace"),
                                    enabled = nativeAvailable && manualEndpoint.isNotBlank(),
                                    style = PrecisionButtonStyle.TONAL,
                                    onClick = { onConnectWorkspace("", manualEndpoint.trim()) }
                                )
                            }
                        }
                    }
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
    val p = WeaveloTokens.colors
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
        contentPadding = PaddingValues(bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        item {
            Text(
                text = tr("PRIVATE MESH TOPOLOGY"),
                color = p.textTertiary,
                fontSize = 11.sp,
                fontWeight = FontWeight.Bold,
                fontFamily = FontFamily.Monospace,
                letterSpacing = 0.8.sp
            )
        }

        item {
            PrecisionCard(cornerRadius = 18.dp) {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text(
                            text = networkName.ifBlank { tr("Personal Device Mesh") },
                            color = p.textPrimary,
                            fontSize = 16.sp,
                            fontWeight = FontWeight.Bold,
                            modifier = Modifier.weight(1f, fill = false),
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis
                        )
                        PrecisionPillBadge(
                            text = meshBadgeText,
                            tone = if (nativeAvailable) PillTone.BRAND else PillTone.NEUTRAL,
                            showDot = nativeAvailable,
                            monospace = true
                        )
                    }
                    Text(
                        text = trFormat("%d linked peers in private overlay network", devices.size),
                        color = p.textPrimary,
                        fontSize = 12.5.sp,
                        fontWeight = FontWeight.Medium
                    )
                    Text(
                        text = tr("Encrypted point-to-point tunnels with zero-config hole punching."),
                        color = p.textSecondary,
                        fontSize = 11.5.sp,
                        lineHeight = 16.sp
                    )
                }
            }
        }

        item {
            Text(
                text = tr("LINKED PEERS"),
                color = p.textTertiary,
                fontSize = 11.sp,
                fontWeight = FontWeight.Bold,
                fontFamily = FontFamily.Monospace,
                letterSpacing = 0.8.sp
            )
        }

        if (devices.isEmpty()) {
            item {
                PrecisionCard(cornerRadius = 16.dp) {
                    Text(
                        tr("No peers linked yet. Scan a QR invitation or enter a join code from another device to expand your mesh."),
                        color = p.textSecondary,
                        fontSize = 13.sp,
                        lineHeight = 19.sp
                    )
                }
            }
        } else {
            items(devices.size) { index ->
                val device = devices[index]
                PrecisionCard(cornerRadius = 16.dp) {
                    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        Row(
                            modifier = Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically
                        ) {
                            Text(
                                text = device.name.ifBlank { device.id },
                                color = p.textPrimary,
                                fontSize = 14.5.sp,
                                fontWeight = FontWeight.SemiBold,
                                modifier = Modifier.weight(1f, fill = false),
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis
                            )
                            PrecisionPillBadge(
                                text = device.scope,
                                tone = PillTone.CYAN,
                                monospace = true
                            )
                        }
                        Text(
                            text = trFormat("Endpoint: %s", device.endpoint.ifBlank { tr("Auto-negotiated UDP") }),
                            color = p.textSecondary,
                            fontSize = 11.5.sp,
                            fontFamily = FontFamily.Monospace
                        )
                        Text(
                            text = deviceCapabilities(device),
                            color = p.textSecondary,
                            fontSize = 12.sp
                        )
                        if (device.canOpenWorkspace) {
                            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                                PrecisionButton(
                                    text = tr("Inspect peer"),
                                    leadingSymbol = MobileSymbol.View,
                                    enabled = nativeAvailable && !joiningNetwork,
                                    style = PrecisionButtonStyle.TONAL,
                                    compact = true,
                                    onClick = { onConnectWorkspace(device.id, device.endpoint) }
                                )
                            }
                        }
                    }
                }
            }
        }

        item {
            PrecisionCard(sunken = true, cornerRadius = 16.dp) {
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(
                        text = tr("ROUTING & SECURITY POLICY"),
                        color = p.textPrimary,
                        fontSize = 12.sp,
                        fontWeight = FontWeight.Bold
                    )
                    Text(
                        text = tr("• Local Subnet: Direct UDP\n• WAN Cross-NAT: STUN UDP Hole Punching\n• Fallback: Authenticated end-to-end encrypted relay"),
                        color = p.textSecondary,
                        fontSize = 11.5.sp,
                        lineHeight = 17.sp
                    )
                }
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
    val p = WeaveloTokens.colors
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
    if (securityDetails) {
        PrecisionDialog(
            title = tr("Connection security"),
            onDismissRequest = { securityDetails = false },
            icon = MobileSymbol.Permissions,
            confirmButton = {
                PrecisionButton(
                    text = tr("Done"),
                    onClick = { securityDetails = false },
                    style = PrecisionButtonStyle.PRIMARY,
                    compact = true
                )
            }
        ) {
            TelemetryLaneRow(tr("Transport cipher"), "ChaCha20-Poly1305", p.brand)
            TelemetryLaneRow(tr("Network access"), tr("Invitation required"), p.emeraldText)
            TelemetryLaneRow(tr("Authentication"), tr("Paired device group"), p.brand)
            TelemetryLaneRow(tr("Storage"), tr("App private"), p.textSecondary)
        }
    }
    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        item {
            Text(tr("Sharing & permissions"), color = p.textPrimary, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
            Text(
                tr("Share a screen, app or camera with your devices."),
                color = p.textSecondary,
                fontSize = 12.5.sp,
                modifier = Modifier.padding(top = 4.dp)
            )
        }
        item {
            PrecisionCard(cornerRadius = 18.dp, contentPadding = PaddingValues(horizontal = 14.dp, vertical = 10.dp)) {
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    SharingPermissionRow(
                        symbol = MobileSymbol.Share,
                        title = "Screen or app",
                        state = if (publishing) tr("Sharing") else tr("Not sharing"),
                        action = if (publishing) "Stop" else "Share",
                        active = publishing,
                        destructiveAction = publishing,
                        enabled = nativeAvailable
                    ) {
                        if (publishing) onStopSharing() else onShareScreen(false)
                    }
                    Row(Modifier.fillMaxWidth().padding(bottom = 4.dp), horizontalArrangement = Arrangement.End) {
                        PrecisionButton(
                            text = tr("Outgoing screen quality"),
                            leadingSymbol = MobileSymbol.Settings,
                            style = PrecisionButtonStyle.SUBTLE,
                            compact = true,
                            onClick = onOutgoingQuality
                        )
                    }
                    HorizontalDivider(color = p.borderMuted)
                    SharingPermissionRow(
                        symbol = MobileSymbol.Camera,
                        title = "Camera",
                        state = camera.statusText?.let { appText(it) } ?: trFormat("Details: %s", camera.status),
                        action = if (camera.enabled) "Stop" else "Share",
                        active = camera.enabled,
                        destructiveAction = camera.enabled,
                        enabled = nativeAvailable
                    ) {
                        if (camera.enabled) `fun`.hackerlife.weavelo.CameraPublisher.stop(context) else onShareCamera()
                    }
                    HorizontalDivider(color = p.borderMuted)
                    SharingPermissionRow(
                        symbol = MobileSymbol.Control,
                        title = "Remote control",
                        state = if (controlReady) tr("Allowed") else tr("Needs permission"),
                        action = if (controlReady) "Settings" else "Allow",
                        active = controlReady,
                        enabled = nativeAvailable,
                        onClick = onControlSettings
                    )
                    HorizontalDivider(color = p.borderMuted)
                    SharingPermissionRow(
                        symbol = MobileSymbol.Audio,
                        title = "Audio",
                        state = if (audioAllowed) tr("Microphone allowed") else tr("Needs permission"),
                        action = "Share with audio",
                        active = audioAllowed,
                        enabled = nativeAvailable && !publishing
                    ) { onShareScreen(true) }
                    HorizontalDivider(color = p.borderMuted)
                    SharingPermissionRow(
                        symbol = MobileSymbol.Notifications,
                        title = "Notifications",
                        state = if (notificationsAllowed) tr("Allowed") else tr("Off"),
                        action = "Settings",
                        active = notificationsAllowed,
                        enabled = true
                    ) {
                        context.startActivity(
                            android.content.Intent(android.provider.Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                                .putExtra(android.provider.Settings.EXTRA_APP_PACKAGE, context.packageName)
                        )
                    }
                    if (controlReady) {
                        HorizontalDivider(color = p.borderMuted)
                        Spacer(Modifier.height(4.dp))
                        PrecisionButton(
                            text = tr("Share with control and audio"),
                            leadingSymbol = MobileSymbol.Control,
                            enabled = nativeAvailable && !publishing,
                            style = PrecisionButtonStyle.PRIMARY,
                            modifier = Modifier.fillMaxWidth(),
                            onClick = onShareControlledScreen
                        )
                    }
                }
            }
        }
        item {
            PrecisionCard(cornerRadius = 18.dp, contentPadding = PaddingValues(horizontal = 14.dp, vertical = 10.dp)) {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    SharingPermissionRow(
                        symbol = MobileSymbol.Files,
                        title = "Receive files",
                        state = if (receiverStarting) tr("Starting…") else if (receivingFiles) tr("On") else tr("Off"),
                        action = if (receivingFiles) "Stop" else "Start",
                        active = receivingFiles,
                        destructiveAction = receivingFiles,
                        enabled = nativeAvailable && !receiverStarting
                    ) {
                        if (receivingFiles) onStopReceiving() else onStartReceiving()
                    }
                    PrecisionButton(
                        text = tr("Browse received files"),
                        leadingSymbol = MobileSymbol.Folder,
                        style = PrecisionButtonStyle.TONAL,
                        modifier = Modifier.fillMaxWidth(),
                        onClick = onBrowseFiles
                    )
                }
            }
        }
        item {
            PrecisionButton(
                text = tr("Connection security"),
                leadingSymbol = MobileSymbol.Permissions,
                style = PrecisionButtonStyle.SUBTLE,
                compact = true,
                onClick = { securityDetails = true }
            )
        }
    }
}

@Composable
private fun SharingPermissionRow(
    symbol: MobileSymbol,
    title: String,
    state: String,
    action: String,
    active: Boolean = false,
    destructiveAction: Boolean = false,
    enabled: Boolean,
    onClick: () -> Unit
) {
    val p = WeaveloTokens.colors
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 60.dp)
            .padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        Box(
            modifier = Modifier
                .size(38.dp)
                .clip(RoundedCornerShape(11.dp))
                .background(if (active) p.brandSoft else p.surfaceSunken)
                .border(1.dp, if (active) p.brandBorder.copy(alpha = 0.55f) else p.borderMuted, RoundedCornerShape(11.dp)),
            contentAlignment = Alignment.Center
        ) {
            Icon(
                symbol.image,
                contentDescription = null,
                tint = if (active) p.brand else p.textSecondary,
                modifier = Modifier.size(19.dp)
            )
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(tr(title), color = p.textPrimary, fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
            Text(state, color = if (active) p.emeraldText else p.textSecondary, fontSize = 11.5.sp)
        }
        PrecisionButton(
            text = tr(action),
            enabled = enabled,
            style = when {
                destructiveAction -> PrecisionButtonStyle.DANGER_TONAL
                action == "Share" || action == "Start" || action == "Allow" -> PrecisionButtonStyle.TONAL
                else -> PrecisionButtonStyle.SUBTLE
            },
            compact = true,
            onClick = onClick
        )
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
    val p = WeaveloTokens.colors
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
        PrecisionDialog(
            title = tr("Public connection diagnostics"),
            onDismissRequest = { connectionDetails = null },
            icon = MobileSymbol.Network,
            dismissButton = {
                PrecisionButton(
                    text = tr("Close diagnostics"),
                    onClick = { connectionDetails = null },
                    style = PrecisionButtonStyle.SUBTLE,
                    compact = true
                )
            },
            confirmButton = {
                PrecisionButton(
                    text = tr("Refresh diagnostics"),
                    leadingSymbol = MobileSymbol.Refresh,
                    onClick = { connectionDetails = publicConnectionEvidence(appNetwork, discoveryDiagnostics) },
                    style = PrecisionButtonStyle.TONAL,
                    compact = true
                )
            }
        ) {
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(max = 300.dp)
                    .clip(RoundedCornerShape(12.dp))
                    .background(p.surfaceSunken)
                    .border(1.dp, p.borderMuted, RoundedCornerShape(12.dp))
                    .padding(10.dp)
                    .verticalScroll(rememberScrollState())
            ) {
                Text(
                    evidence,
                    color = p.textPrimary,
                    fontFamily = FontFamily.Monospace,
                    fontSize = 10.5.sp,
                    lineHeight = 15.sp
                )
            }
        }
    }
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        item { LanguageSelector(modifier = Modifier.fillMaxWidth()) }
        item { AppUpdateCard(update, updateAllowed, onPrepareInstall = onPrepareInstall) }
        item {
            PrecisionCard(cornerRadius = 16.dp) {
                PrecisionSwitchRow(
                    title = tr("Stay online in background"),
                    subtitle = tr("Keep this phone reachable by paired devices when you leave Weavelo or lock the screen."),
                    checked = keepOnline,
                    enabled = nativeAvailable,
                    leadingSymbol = MobileSymbol.Network,
                    onCheckedChange = { `fun`.hackerlife.weavelo.DevicePresenceService.setEnabled(context, it) }
                )
            }
        }
        item { DeviceBadgeSettings() }
        item {
            Text(
                text = tr("GESTURE SYSTEM"),
                color = p.textTertiary,
                fontSize = 11.sp,
                fontWeight = FontWeight.Bold,
                fontFamily = FontFamily.Monospace,
                letterSpacing = 0.8.sp
            )
        }

        item {
            val naturalScroll by WeaveloClient.naturalScrollFlow.collectAsState()
            PrecisionCard(cornerRadius = 16.dp) {
                Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    PrecisionSwitchRow(
                        title = tr("Natural scrolling (Apple style)"),
                        subtitle = tr("Content moves in the same direction as your fingers"),
                        checked = naturalScroll,
                        enabled = nativeAvailable,
                        leadingSymbol = MobileSymbol.Touch,
                        onCheckedChange = { WeaveloClient.updateNaturalScroll(it) }
                    )
                    HorizontalDivider(color = p.borderMuted)
                    GestureGuideSection(
                        title = tr("Direct Touch (Screen-First)"),
                        body = tr("• Single Tap: Mouse Left Click\n• Touch & Drag: Mouse Drag Selection\n• Long Press (380ms): Mouse Right Click\n• Two-Finger Glide: Natural App Scroll (1x & Zoomed)\n• Two-Finger Tap: Mouse Right Click")
                    )
                    GestureGuideSection(
                        title = tr("Virtual Trackpad"),
                        body = tr("• Single Finger Glide: Relative Cursor Move\n• Single Tap: Left Click\n• Double-Tap & Hold Glide: Mouse Drag\n• Two-Finger Tap: Right Click\n• Two-Finger Glide: Natural App Scroll (1x & Zoomed)")
                    )
                    GestureGuideSection(
                        title = tr("Zoom & Canvas"),
                        body = tr("• Two-Finger Pinch: Zoom & Pan Viewport\n• Three-Finger Drag / Pan Canvas Button: Move Zoomed Canvas")
                    )
                }
            }
        }

        item {
            PrecisionButton(
                text = if (showAdvancedDiagnostics) tr("Hide advanced runtime & network diagnostics")
                    else tr("Show advanced runtime & network diagnostics"),
                leadingSymbol = MobileSymbol.Settings,
                style = PrecisionButtonStyle.SUBTLE,
                compact = true,
                onClick = { showAdvancedDiagnostics = !showAdvancedDiagnostics }
            )
        }

        if (showAdvancedDiagnostics) {
            item {
                Text(
                    text = tr("RUNTIME & ENGINE"),
                    color = p.textTertiary,
                    fontSize = 11.sp,
                    fontWeight = FontWeight.Bold,
                    fontFamily = FontFamily.Monospace,
                    letterSpacing = 0.8.sp
                )
            }

            item {
                PrecisionCard(cornerRadius = 16.dp) {
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        TelemetryLaneRow(
                            tr("Bridge Status"),
                            if (nativeAvailable) tr("Loaded (.so ready)") else tr("Missing native library"),
                            if (nativeAvailable) p.emeraldText else p.dangerText
                        )
                        TelemetryLaneRow(tr("Session State"), sessionLabel(sessionState), p.brand)
                        TelemetryLaneRow(tr("Decoders"), tr("Hardware MediaCodec (H.264/HEVC)"), p.textPrimary)
                        TelemetryLaneRow(tr("Audio Engine"), tr("AudioTrack + Opus Decoder"), p.textPrimary)
                    }
                }
            }

            item {
                PrecisionCard(cornerRadius = 16.dp) {
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        val status = discoveryDiagnostics
                        Text(tr("CONNECTION STATUS"), color = p.textTertiary, fontSize = 11.sp, fontWeight = FontWeight.Bold)
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            PrecisionButton(
                                text = tr("Connection details"),
                                style = PrecisionButtonStyle.TONAL,
                                compact = true,
                                onClick = { connectionDetails = publicConnectionEvidence(appNetwork, status) }
                            )
                            PrecisionButton(
                                text = tr("Check relay transport"),
                                leadingSymbol = MobileSymbol.Refresh,
                                style = PrecisionButtonStyle.SUBTLE,
                                compact = true,
                                onClick = { WeaveloClient.checkRelayTransport() }
                            )
                        }
                        TelemetryLaneRow(tr("Discovery"), status.phase, p.brand)
                        TelemetryLaneRow(tr("Runtime network"), status.networkHash.take(12).ifEmpty { tr("Starting") }, p.textPrimary)
                        TelemetryLaneRow(tr("This device"), status.deviceHash.take(12).ifEmpty { tr("Starting") }, p.textPrimary)
                        TelemetryLaneRow(tr("Configuration"), trFormat("%d/%d · run %d", status.reloadApplied, status.reloadRequested, status.generation), p.textPrimary)
                        TelemetryLaneRow(tr("Routes / Devices"), "${status.routes} / ${status.peers}", p.textPrimary)
                        TelemetryLaneRow(tr("Direct discovery"), status.p2p, p.textPrimary)
                        for ((name, relay) in listOf("Control relay" to status.control, "Discovery relay" to status.discovery)) {
                            TelemetryLaneRow(tr(name), relay.state, p.brand)
                            TelemetryLaneRow(tr("Connection attempts"), relay.attempts.toString(), p.textPrimary)
                            TelemetryLaneRow(tr("Registration writes / Pongs"), "${relay.registrationsSent} / ${relay.pongsReceived}", p.textPrimary)
                            if (relay.error.isNotEmpty()) TelemetryLaneRow(tr("Relay reason"), relay.error, p.textSecondary)
                        }
                        if (status.failure.isNotEmpty()) TelemetryLaneRow(tr("Discovery reason"), status.failure, p.textSecondary)
                        Text(
                            tr("Registration writes are local sends. Pongs confirm a relay response; the device list confirms discovery."),
                            color = p.textSecondary,
                            fontSize = 11.sp,
                            lineHeight = 16.sp
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun GestureGuideSection(title: String, body: String) {
    val p = WeaveloTokens.colors
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(11.dp))
            .background(p.surfaceSunken)
            .padding(horizontal = 11.dp, vertical = 9.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp)
    ) {
        Text(title, color = p.brand, fontSize = 12.sp, fontWeight = FontWeight.Bold)
        Text(body, color = p.textSecondary, fontSize = 11.5.sp, lineHeight = 17.sp)
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
