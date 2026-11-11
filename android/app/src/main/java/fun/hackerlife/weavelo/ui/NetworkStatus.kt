package `fun`.hackerlife.weavelo.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.PopupProperties
import `fun`.hackerlife.weavelo.SessionState
import `fun`.hackerlife.weavelo.TelemetrySnapshot
import android.os.SystemClock
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
    val loss = telemetry.packetLossPercent.takeIf { it.isFinite() && it in 0f..100f }
    return when {
        (rtt != null && rtt > 150) || (loss != null && loss >= 5) -> NetworkIndicator("Network poor", NetworkTone.ERROR)
        (rtt != null && rtt > 80) || (loss != null && loss >= 1) -> NetworkIndicator("Network fair", NetworkTone.WARNING)
        rtt == null || loss == null -> NetworkIndicator("Network unknown", NetworkTone.NEUTRAL)
        else -> NetworkIndicator("Network good", NetworkTone.ONLINE)
    }
}

/** Uses existing observations. Opening never starts a probe or changes a session. */
@Composable
internal fun NetworkStatusChip(
    session: SessionState, telemetry: TelemetrySnapshot,
    route: String? = null,
    modifier: Modifier = Modifier
) {
    var now by remember { mutableStateOf(SystemClock.elapsedRealtime()) }
    LaunchedEffect(Unit) { while (true) { now = SystemClock.elapsedRealtime(); delay(1000) } }
    val sampleNow = maxOf(now, SystemClock.elapsedRealtime())
    val status = networkIndicator(session, telemetry, sampleNow)
    val fresh = freshNetworkTelemetry(session, telemetry, sampleNow)
    val rtt = telemetry.networkRttMs?.takeIf { fresh && it.isFinite() && it >= 0 }
    val latency = rtt?.let { String.format(Locale.ROOT, "%.0f ms", it) }
    val loss = telemetry.packetLossPercent.takeIf { fresh && it.isFinite() && it in 0f..100f }
    val routeLabel = when (route?.trim()?.uppercase(Locale.ROOT)) {
        null, "" -> null
        "LAN" -> tr("LAN")
        "P2P", "DIRECT" -> tr("P2P")
        "RELAY" -> tr("Relay")
        else -> route.trim()
    }
    val chipText = when {
        session != SessionState.STREAMING -> tr(status.label)
        routeLabel != null && latency != null -> "$routeLabel · $latency"
        latency != null -> latency
        routeLabel != null -> "$routeLabel · ${tr(status.label)}"
        else -> tr(status.label)
    }

    val color = when (status.tone) {
        NetworkTone.ONLINE -> ColorAccentEmerald
        NetworkTone.WARNING -> Color(0xFF9A6700)
        NetworkTone.ERROR -> Color(0xFFB42318)
        NetworkTone.NEUTRAL -> ColorTextSecondary
    }
    var pinned by remember { mutableStateOf(false) }
    Box(modifier) {
        Row(Modifier.heightIn(min = 34.dp).clip(RoundedCornerShape(50))
            .background(ColorSurfaceCard).border(1.dp, ColorBorderFine, RoundedCornerShape(50))
            .clickable(role = Role.Button, onClickLabel = tr("Network details")) { pinned = !pinned }
            .padding(horizontal = 10.dp, vertical = 5.dp),
            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Box(Modifier.size(6.dp).background(color, CircleShape))
            Text(chipText, color = color, fontSize = 11.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        DropdownMenu(expanded = pinned, onDismissRequest = { pinned = false },
            properties = PopupProperties(focusable = pinned),
            modifier = Modifier.widthIn(min = 156.dp, max = 216.dp)) {
            Column(Modifier.padding(horizontal = 12.dp, vertical = 4.dp), verticalArrangement = Arrangement.spacedBy(5.dp)) {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween) {
                    Text(tr("Network details"), fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
                    MobileAction(MobileSymbol.Close, tr("Close"), modifier = Modifier.size(24.dp)) { pinned = false }
                }
                NetworkDetail("Connection", tr(status.label))
                if (routeLabel != null) NetworkDetail("Route", routeLabel)
                NetworkDetail("Network RTT", latency ?: "—")
                NetworkDetail("Packet loss rate", loss?.let { String.format(Locale.ROOT, "%.1f%%", it) } ?: "—")
                NetworkDetail("Video bitrate", if (fresh && telemetry.videoBitrateKbps >= 0)
                    String.format(Locale.ROOT, "%.1f Mbps", telemetry.videoBitrateKbps / 1000.0) else "—")
            }
        }
    }
}

@Composable
private fun NetworkDetail(label: String, value: String) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Text(tr(label), color = ColorTextSecondary, fontSize = 11.sp, maxLines = 1)
        Spacer(Modifier.weight(1f))
        Text(value, color = ColorTextPrimary, fontSize = 11.sp, fontWeight = FontWeight.Medium, maxLines = 1)
    }
}
