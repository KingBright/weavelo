package `fun`.hackerlife.weavelo.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import `fun`.hackerlife.weavelo.AndroidGamepadController
import `fun`.hackerlife.weavelo.Ps5ConsoleSummary
import `fun`.hackerlife.weavelo.WorkspaceClient
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlin.math.floor

@Composable
internal fun Ps5PairingDialog(
    sessionKey: String,
    generation: Long,
    onDismiss: () -> Unit,
    onPaired: () -> Unit,
) {
    val p = WeaveloTokens.colors
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var consoles by remember { mutableStateOf<List<Ps5ConsoleSummary>>(emptyList()) }
    var psnInput by remember { mutableStateOf("") }
    var pinInput by remember { mutableStateOf("") }
    var hostIpInput by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    var notice by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    val msgPasteCredential = tr("Paste the PlayStation sign-in redirect URL, code, or AccountID")
    val msgEnterPin = tr("Enter the 8-digit PS5 Link Device PIN")
    val msgLinkedOk = tr("PS5 linked successfully")
    val msgPairFailed = tr("PS5 pairing failed")
    val msgLinkCopied = tr("Sign-in link copied")
    val qrMatrix = remember { runCatching { invitationQr(AndroidGamepadController.PSN_AUTHORIZE_URL) }.getOrNull() }

    fun refreshList() {
        if (busy) return
        busy = true
        error = null
        scope.launch {
            val res = withContext(Dispatchers.IO) { runCatching { WorkspaceClient.listConsoles() } }
            busy = false
            res.onSuccess { consoles = it }
                .onFailure { error = it.message ?: "Failed to scan LAN PS5 consoles" }
        }
    }

    LaunchedEffect(Unit) { refreshList() }

    PrecisionDialog(
        title = tr("Link PlayStation 5"),
        onDismissRequest = { if (!busy) onDismiss() },
        icon = MobileSymbol.Gamepad,
        maxWidth = 520.dp,
        dismissButton = {
            PrecisionButton(
                text = tr("Refresh"),
                leadingSymbol = MobileSymbol.Refresh,
                style = PrecisionButtonStyle.SUBTLE,
                compact = true,
                enabled = !busy,
                onClick = ::refreshList,
            )
        },
        confirmButton = {
            PrecisionButton(
                text = if (busy) tr("Working…") else tr("Link PS5"),
                style = PrecisionButtonStyle.PRIMARY,
                compact = true,
                enabled = !busy,
                onClick = {
                    val cleanAuth = psnInput.trim()
                    val cleanPin = pinInput.filter { it.isDigit() }
                    if (cleanAuth.isEmpty()) {
                        error = msgPasteCredential
                        return@PrecisionButton
                    }
                    if (cleanPin.length != 8) {
                        error = msgEnterPin
                        return@PrecisionButton
                    }
                    busy = true
                    error = null
                    notice = null
                    scope.launch {
                        val res = withContext(Dispatchers.IO) {
                            runCatching {
                                WorkspaceClient.pairConsole(
                                    authCredential = cleanAuth,
                                    pin = cleanPin,
                                    hostAddr = hostIpInput.trim().takeIf { it.isNotEmpty() },
                                )
                            }
                        }
                        busy = false
                        res.onSuccess { reply ->
                            if (reply.ok) {
                                pinInput = ""
                                notice = reply.console?.nickname?.takeIf { it.isNotBlank() }?.let {
                                    "$msgLinkedOk · $it"
                                } ?: msgLinkedOk
                                WorkspaceClient.refreshSources(sessionKey, generation)
                                refreshList()
                                onPaired()
                            } else {
                                error = reply.error ?: msgPairFailed
                            }
                        }.onFailure { error = it.message ?: msgPairFailed }
                    }
                },
            )
        },
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(max = 420.dp)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (qrMatrix != null) {
                    Box(
                        modifier = Modifier
                            .size(92.dp)
                            .clip(RoundedCornerShape(10.dp))
                            .background(Color.White)
                            .padding(4.dp),
                        contentAlignment = Alignment.Center,
                    ) {
                        Canvas(Modifier.fillMaxSize()) {
                            val cell = floor(size.minDimension / qrMatrix.width).coerceAtLeast(1f)
                            val origin = (size.minDimension - cell * qrMatrix.width) / 2f
                            for (y in 0 until qrMatrix.height) {
                                for (x in 0 until qrMatrix.width) {
                                    if (qrMatrix[x, y]) {
                                        drawRect(
                                            color = Color.Black,
                                            topLeft = Offset(origin + x * cell, origin + y * cell),
                                            size = Size(cell, cell),
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
                Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(
                        text = tr("1. Open PSN sign-in (or scan QR), sign in, and copy the redirect URL from the blank page."),
                        color = p.textPrimary,
                        fontSize = 12.sp,
                    )
                    Text(
                        text = tr("2. On PS5: Settings -> System -> Remote Play -> Link Device to get the 8-digit PIN."),
                        color = p.textSecondary,
                        fontSize = 11.5.sp,
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        PrecisionButton(
                            text = tr("Open sign-in"),
                            style = PrecisionButtonStyle.TONAL,
                            compact = true,
                            onClick = {
                                runCatching {
                                    context.startActivity(
                                        Intent(Intent.ACTION_VIEW, Uri.parse(AndroidGamepadController.PSN_AUTHORIZE_URL))
                                            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                                    )
                                }
                            },
                        )
                        PrecisionButton(
                            text = tr("Copy link"),
                            leadingSymbol = MobileSymbol.Copy,
                            style = PrecisionButtonStyle.SUBTLE,
                            compact = true,
                            onClick = {
                                context.getSystemService(ClipboardManager::class.java)
                                    ?.setPrimaryClip(ClipData.newPlainText("PSN OAuth", AndroidGamepadController.PSN_AUTHORIZE_URL))
                                notice = msgLinkCopied
                            },
                        )
                    }
                }
            }

            if (consoles.isNotEmpty()) {
                HorizontalDivider(color = p.borderMuted)
                Text(
                    text = tr("Discovered on host LAN"),
                    color = p.brand,
                    fontSize = 11.5.sp,
                    fontWeight = FontWeight.SemiBold,
                )
                consoles.forEach { item ->
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clip(RoundedCornerShape(10.dp))
                            .background(p.surfaceSunken)
                            .border(0.75.dp, p.borderMuted, RoundedCornerShape(10.dp))
                            .padding(horizontal = 10.dp, vertical = 7.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Column(Modifier.weight(1f)) {
                            Text(
                                text = item.nickname,
                                color = p.textPrimary,
                                fontSize = 12.5.sp,
                                fontWeight = FontWeight.Medium,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            Text(
                                text = "${item.hostAddr ?: item.hostId} · ${item.status}",
                                color = p.textSecondary,
                                fontSize = 11.sp,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                        if (!item.paired && item.hostAddr != null) {
                            PrecisionButton(
                                text = tr("Fill IP"),
                                style = PrecisionButtonStyle.SUBTLE,
                                compact = true,
                                onClick = { hostIpInput = item.hostAddr },
                            )
                        }
                        if (item.paired) {
                            PrecisionButton(
                                text = tr("Remove"),
                                style = PrecisionButtonStyle.SUBTLE,
                                compact = true,
                                enabled = !busy,
                                onClick = {
                                    busy = true
                                    scope.launch {
                                        val res = withContext(Dispatchers.IO) {
                                            runCatching { WorkspaceClient.removeConsole(item.consoleId) }
                                        }
                                        busy = false
                                        res.onSuccess {
                                            consoles = it
                                            WorkspaceClient.refreshSources(sessionKey, generation)
                                        }.onFailure { error = it.message ?: "Failed to remove PS5" }
                                    }
                                },
                            )
                        }
                    }
                }
            }

            PrecisionTextField(
                value = psnInput,
                onValueChange = { psnInput = it },
                label = tr("Redirect URL / OAuth code / AccountID"),
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                PrecisionTextField(
                    value = pinInput,
                    onValueChange = { pinInput = it },
                    label = tr("8-digit PIN"),
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                    modifier = Modifier.weight(1f),
                )
                PrecisionTextField(
                    value = hostIpInput,
                    onValueChange = { hostIpInput = it },
                    label = tr("PS5 IP (optional)"),
                    modifier = Modifier.weight(1f),
                )
            }
            notice?.let { Text(it, color = p.brand, fontSize = 12.sp) }
            error?.let { Text(it, color = p.danger, fontSize = 12.sp) }
        }
    }
}
