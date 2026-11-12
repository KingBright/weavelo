package `fun`.hackerlife.weavelo.ui

import android.os.SystemClock
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.PopupProperties
import `fun`.hackerlife.weavelo.SessionState
import `fun`.hackerlife.weavelo.TelemetrySnapshot
import java.util.Locale
import kotlinx.coroutines.delay

internal enum class NetworkTone { NEUTRAL, ONLINE, WARNING, ERROR }
internal data class NetworkIndicator(val label: String, val tone: NetworkTone)

internal fun freshNetworkTelemetry(session: SessionState, telemetry: TelemetrySnapshot, now: Long): Boolean =
    session == SessionState.STREAMING && telemetry.observedAtElapsedMs > 0 &&
        now - telemetry.observedAtElapsedMs in 0..5000

internal fun networkIndicator(session: SessionState, telemetry: TelemetrySnapshot, now: Long): NetworkIndicator {
    if (session == SessionState.CONNECTING) return NetworkIndicator("Connecting", NetworkTone.NEUTRAL)
    if (session == SessionState.RECONNECTING) return NetworkIndicator("Reconnecting", NetworkTone.WARNING)
    if (session != SessionState.STREAMING) return NetworkIndicator("Not connected", NetworkTone.ERROR)
    if (!freshNetworkTelemetry(session, telemetry, now)) return NetworkIndicator("Network unknown", NetworkTone.NEUTRAL)
    val rtt = telemetry.networkRttMs?.takeIf { it.isFinite() && it >= 0 }
    val decodeMs = telemetry.latencyMs.takeIf { it.isFinite() && it > 0f } ?: 0f
    val videoE2e = telemetry.captureReceiveEstimateMs?.takeIf { it.isFinite() && it >= 0 }?.let { it + decodeMs }
    val loss = telemetry.packetLossPercent.takeIf { it.isFinite() && it in 0f..100f }
    return when {
        (videoE2e != null && videoE2e > 180) || (rtt != null && rtt > 150) || (loss != null && loss >= 5) -> NetworkIndicator("Network poor", NetworkTone.ERROR)
        (videoE2e != null && videoE2e > 90) || (rtt != null && rtt > 80) || (loss != null && loss >= 1) -> NetworkIndicator("Network fair", NetworkTone.WARNING)
        videoE2e == null && (rtt == null || loss == null) -> NetworkIndicator("Network unknown", NetworkTone.NEUTRAL)
        else -> NetworkIndicator("Network good", NetworkTone.ONLINE)
    }
}

internal fun signalBarsLevel(session: SessionState, telemetry: TelemetrySnapshot, now: Long): Int {
    if (session != SessionState.STREAMING) return 0
    val status = networkIndicator(session, telemetry, now)
    return when (status.tone) {
        NetworkTone.ONLINE -> {
            val rtt = telemetry.networkRttMs?.takeIf { it.isFinite() && it >= 0 }
            val decodeMs = telemetry.latencyMs.takeIf { it.isFinite() && it > 0f } ?: 0f
            val videoE2e = telemetry.captureReceiveEstimateMs?.takeIf { it.isFinite() && it >= 0 }?.let { it + decodeMs }
            val loss = telemetry.packetLossPercent.takeIf { it.isFinite() && it in 0f..100f } ?: 0f
            if (loss < 0.3f && ((videoE2e != null && videoE2e <= 45f) || (videoE2e == null && rtt != null && rtt <= 35.0))) 4 else 3
        }
        NetworkTone.WARNING -> 2
        NetworkTone.ERROR -> 1
        NetworkTone.NEUTRAL -> 2
    }
}

/**
 * Minimalist signal-bars-only corner indicator (zero persistent text).
 * Tapping opens a floating popover with full live transport & video latency details.
 */
@Composable
internal fun NetworkStatusChip(
    session: SessionState,
    telemetry: TelemetrySnapshot,
    route: String? = null,
    alpha: Float = 1f,
    modifier: Modifier = Modifier,
) {
    val p = WeaveloTokens.colors
    var now by remember { mutableStateOf(SystemClock.elapsedRealtime()) }
    LaunchedEffect(Unit) { while (true) { now = SystemClock.elapsedRealtime(); delay(1000) } }
    val sampleNow = maxOf(now, SystemClock.elapsedRealtime())
    val status = networkIndicator(session, telemetry, sampleNow)
    val bars = signalBarsLevel(session, telemetry, sampleNow)
    val fresh = freshNetworkTelemetry(session, telemetry, sampleNow)
    val rtt = telemetry.networkRttMs?.takeIf { fresh && it.isFinite() && it >= 0 }
    val decodeMs = telemetry.latencyMs.takeIf { fresh && it.isFinite() && it > 0f }
    val videoE2e = telemetry.captureReceiveEstimateMs?.takeIf { fresh && it.isFinite() && it >= 0 }
        ?.let { it + (decodeMs ?: 0f) }
    val rttLabel = rtt?.let { String.format(Locale.ROOT, "%.0f ms", it) }
    val fpsLabel = telemetry.fps.takeIf { fresh && it.isFinite() && it > 0f }
        ?.let { String.format(Locale.ROOT, "%.0f FPS", it) }
    val loss = telemetry.packetLossPercent.takeIf { fresh && it.isFinite() && it in 0f..100f }
    val routeLabel = when (route?.trim()?.uppercase(Locale.ROOT)) {
        null, "" -> null
        "LAN" -> tr("LAN")
        "P2P", "DIRECT" -> tr("P2P")
        "RELAY" -> tr("Relay")
        else -> route.trim()
    }

    val clampedAlpha = alpha.coerceIn(0.28f, 1f)
    val activeBarColor = when (status.tone) {
        NetworkTone.ONLINE -> p.emeraldDot
        NetworkTone.WARNING -> p.warningText
        NetworkTone.ERROR -> p.dangerText
        NetworkTone.NEUTRAL -> Color.White.copy(alpha = 0.78f)
    }
    val shape = RoundedCornerShape(11.dp)
    var pinned by remember { mutableStateOf(false) }

    Box(modifier) {
        Box(
            Modifier
                .size(34.dp)
                .clip(shape)
                .background(Color(0x940B1018).copy(alpha = 0.58f * clampedAlpha))
                .border(0.75.dp, Color.White.copy(alpha = 0.15f * clampedAlpha), shape)
                .clickable(role = Role.Button, onClickLabel = tr("Network details")) { pinned = !pinned },
            contentAlignment = Alignment.Center,
        ) {
            SignalBarsGlyph(
                bars = bars,
                activeColor = activeBarColor.copy(alpha = activeBarColor.alpha * clampedAlpha),
                inactiveColor = Color.White.copy(alpha = 0.24f * clampedAlpha),
                modifier = Modifier.size(width = 16.dp, height = 13.dp),
            )
        }
        DropdownMenu(
            expanded = pinned,
            onDismissRequest = { pinned = false },
            properties = PopupProperties(focusable = pinned),
            modifier = Modifier.widthIn(min = 196.dp, max = 248.dp),
        ) {
            Column(
                Modifier.padding(horizontal = 14.dp, vertical = 6.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                Row(
                    Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    Text(
                        tr("Network details"),
                        color = p.textPrimary,
                        fontSize = 12.sp,
                        fontWeight = FontWeight.SemiBold,
                    )
                    MobileAction(
                        MobileSymbol.Close,
                        tr("Close"),
                        tonalWell = true,
                        modifier = Modifier.size(26.dp),
                    ) { pinned = false }
                }
                HorizontalDivider(color = p.borderMuted)
                NetworkDetail("Connection", tr(status.label))
                if (routeLabel != null) NetworkDetail("Route", routeLabel)
                NetworkDetail("Video E2E latency", videoE2e?.let { String.format(Locale.ROOT, "%.0f ms", it) } ?: "—")
                NetworkDetail("Decode latency", decodeMs?.let { String.format(Locale.ROOT, "%.1f ms", it) } ?: "—")
                NetworkDetail("Network RTT", rttLabel ?: "—")
                NetworkDetail("Frame rate", fpsLabel ?: "—")
                NetworkDetail("Packet loss rate", loss?.let { String.format(Locale.ROOT, "%.1f%%", it) } ?: "—")
                NetworkDetail(
                    "Video bitrate",
                    if (fresh && telemetry.videoBitrateKbps >= 0) {
                        String.format(Locale.ROOT, "%.1f Mbps", telemetry.videoBitrateKbps / 1000.0)
                    } else {
                        "—"
                    },
                )
            }
        }
    }
}

@Composable
private fun SignalBarsGlyph(
    bars: Int,
    activeColor: Color,
    inactiveColor: Color,
    modifier: Modifier = Modifier,
) {
    val clamped = bars.coerceIn(0, 4)
    Canvas(modifier) {
        val barCount = 4
        val gap = 2.dp.toPx()
        val barWidth = ((size.width - gap * (barCount - 1)) / barCount).coerceAtLeast(1.5f)
        val radius = CornerRadius(barWidth * 0.48f, barWidth * 0.48f)
        val fractions = floatArrayOf(0.32f, 0.54f, 0.77f, 1.0f)
        for (i in 0 until barCount) {
            val h = (size.height * fractions[i]).coerceAtLeast(2.dp.toPx())
            val x = i * (barWidth + gap)
            val y = size.height - h
            drawRoundRect(
                color = if (i < clamped) activeColor else inactiveColor,
                topLeft = Offset(x, y),
                size = Size(barWidth, h),
                cornerRadius = radius,
            )
        }
    }
}

@Composable
private fun NetworkDetail(label: String, value: String) {
    val p = WeaveloTokens.colors
    Row(
        Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(tr(label), color = p.textSecondary, fontSize = 11.sp, maxLines = 1)
        Spacer(Modifier.weight(1f))
        Text(
            value,
            color = p.textPrimary,
            fontSize = 11.sp,
            fontFamily = FontFamily.Monospace,
            fontWeight = FontWeight.Medium,
            maxLines = 1,
        )
    }
}
