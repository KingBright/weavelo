package `fun`.hackerlife.weavelo.ui

import android.content.Context
import android.content.Intent
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import `fun`.hackerlife.weavelo.DiagnosticsExporter
import `fun`.hackerlife.weavelo.DiagnosticsReportInput
import kotlinx.coroutines.delay

@Composable
fun DiagnosticsDialog(
    onDismiss: () -> Unit,
    reportInput: DiagnosticsReportInput? = null,
    recentLogs: List<String> = emptyList(),
    onCollectInput: (() -> DiagnosticsReportInput)? = null,
    onExportReport: ((String) -> Unit)? = null,
) {
    val p = WeaveloTokens.colors
    val context = LocalContext.current
    val clipboard = LocalClipboardManager.current

    var refreshNonce by remember { mutableIntStateOf(0) }
    var copiedFeedback by remember { mutableStateOf(false) }
    var actionMessage by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(copiedFeedback) {
        if (copiedFeedback) {
            delay(2000)
            copiedFeedback = false
        }
    }

    val effectiveInput = remember(reportInput, recentLogs, refreshNonce) {
        reportInput
            ?: onCollectInput?.invoke()
            ?: DiagnosticsExporter.collectLiveInput(
                context = context,
                recentLogs = recentLogs,
            )
    }

    val reportJson = remember(effectiveInput) {
        DiagnosticsExporter.buildRedactedJson(effectiveInput, prettyPrint = true)
    }

    val lineCount = remember(reportJson) {
        reportJson.count { it == '\n' } + 1
    }

    val verticalScroll = rememberScrollState()
    val horizontalScroll = rememberScrollState()
    val copiedMsg = tr("Redacted diagnostics JSON copied to clipboard.")
    val exportedMsg = tr("Diagnostics report exported.")
    val shareOpenedMsg = tr("Opened share sheet for diagnostics report.")
    val noShareTargetMsg = tr("No share target available on this device.")
    val shareSubject = tr("Weavelo Redacted Diagnostics Report")
    val shareChooserTitle = tr("Export Diagnostics")

    PrecisionDialog(
        title = tr("Diagnostics Export"),
        subtitle = tr("Redacted runtime, network, and telemetry snapshot. Secrets, tokens, PEM keys, and non-loopback IPs are scrubbed."),
        icon = MobileSymbol.Network,
        maxWidth = 560.dp,
        onDismissRequest = onDismiss,
        dismissButton = {
            PrecisionButton(
                text = tr("Refresh"),
                leadingSymbol = MobileSymbol.Refresh,
                style = PrecisionButtonStyle.SUBTLE,
                compact = true,
                onClick = {
                    actionMessage = null
                    refreshNonce += 1
                },
            )
        },
        confirmButton = {
            PrecisionButton(
                text = tr("Close"),
                style = PrecisionButtonStyle.TONAL,
                compact = true,
                onClick = onDismiss,
            )
        },
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                PrecisionPillBadge(
                    text = tr("Redacted"),
                    tone = PillTone.EMERALD,
                    showDot = true,
                )
                PrecisionPillBadge(
                    text = effectiveInput.routePreference.wire,
                    tone = PillTone.BRAND,
                    monospace = true,
                )
                PrecisionPillBadge(
                    text = trFormat("%d lines · %d B", lineCount, reportJson.length),
                    tone = PillTone.NEUTRAL,
                    monospace = true,
                )
            }

            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                PrecisionButton(
                    text = if (copiedFeedback) tr("Copied") else tr("Copy"),
                    leadingSymbol = if (copiedFeedback) MobileSymbol.Check else MobileSymbol.Copy,
                    style = PrecisionButtonStyle.PRIMARY,
                    compact = true,
                    modifier = Modifier.weight(1f),
                    onClick = {
                        clipboard.setText(AnnotatedString(reportJson))
                        copiedFeedback = true
                        actionMessage = copiedMsg
                    },
                )
                PrecisionButton(
                    text = tr("Share"),
                    leadingSymbol = MobileSymbol.Share,
                    style = PrecisionButtonStyle.TONAL,
                    compact = true,
                    modifier = Modifier.weight(1f),
                    onClick = {
                        if (onExportReport != null) {
                            onExportReport(reportJson)
                            actionMessage = exportedMsg
                        } else {
                            val shared = shareDiagnosticsJson(context, reportJson, shareSubject, shareChooserTitle)
                            actionMessage = if (shared) shareOpenedMsg else noShareTargetMsg
                        }
                    },
                )
            }

            actionMessage?.let { status ->
                Text(
                    text = status,
                    color = p.emeraldText,
                    fontSize = 11.5.sp,
                    fontWeight = FontWeight.Medium,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }

            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(min = 180.dp, max = 340.dp)
                    .clip(RoundedCornerShape(12.dp))
                    .background(p.surfaceSunken)
                    .border(BorderStroke(1.dp, p.borderMuted), RoundedCornerShape(12.dp))
                    .padding(12.dp),
            ) {
                SelectionContainer {
                    Text(
                        text = reportJson,
                        color = p.textPrimary,
                        fontSize = 11.sp,
                        lineHeight = 15.5.sp,
                        fontFamily = FontFamily.Monospace,
                        softWrap = false,
                        modifier = Modifier
                            .fillMaxWidth()
                            .verticalScroll(verticalScroll)
                            .horizontalScroll(horizontalScroll),
                    )
                }
            }

            Spacer(Modifier)
        }
    }
}

private fun shareDiagnosticsJson(
    context: Context,
    payload: String,
    subject: String,
    chooserTitle: String,
): Boolean = runCatching {
    val sendIntent = Intent(Intent.ACTION_SEND).apply {
        type = "text/plain"
        putExtra(Intent.EXTRA_SUBJECT, subject)
        putExtra(Intent.EXTRA_TEXT, payload)
    }
    val chooser = Intent.createChooser(sendIntent, chooserTitle).apply {
        addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }
    context.startActivity(chooser)
    true
}.getOrDefault(false)
