package `fun`.hackerlife.weavelo.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.ui.Alignment
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.unit.dp

/** Small native vectors, cached once and kept out of the video frame path. */
enum class MobileSymbol(path: String) {
    Devices("M3 4h18v12H3z M8 20h8 M12 16v4"),
    View("M2.5 3.5h19v13h-19z M8 20.5h8 M12 16.5v4 M12 6.5v6.5 M9.2 10.2L12 13l2.8-2.8"),
    Network("M9 3h6v5H9z M3 16h6v5H3z M15 16h6v5h-6z M12 8v4 M6 16v-4h12v4"),
    Share("M3 8V4h18v12h-6 M3 18.5h.01 M3 14.5a4 4 0 0 1 4 4 M3 10.5a8 8 0 0 1 8 8 M12.5 11.5L17 7 M13.5 7H17v3.5"),
    Permissions("M12 3l7 3v5.5c0 4.5-3 8.3-7 9.5-4-1.2-7-5-7-9.5V6l7-3z M9 12l2 2 4-4"),
    Send("M3 3l18 9-18 9 3-9-3-9z M6 12h15"),
    Camera("M3 7h4l2-3h6l2 3h4v13H3z M16 13a4 4 0 1 1-8 0 4 4 0 0 1 8 0"),
    CameraSwitch("M3 7h4l1.5-2.5h7L17 7h4v12H3z M9.5 12a2.5 2.5 0 0 1 4.5-1.2 M14.5 9v2.2h-2.2 M14.5 13a2.5 2.5 0 0 1-4.5 1.2 M9.5 16v-2.2h2.2"),
    Settings("M4 6h16 M4 12h16 M4 18h16 M8 3v6 M16 9v6 M10 15v6"),
    Scan("M3 8V3h5 M16 3h5v5 M21 16v5h-5 M8 21H3v-5 M8 8h2v2H8z M14 8h2v2h-2z M8 14h2v2H8z M14 14h3v3h-3z"),
    Invitation("M3 5h18v14H3z M3 6l9 7 9-7"),
    Open("M2.5 3.5h19v13h-19z M8 20.5h8 M12 16.5v4 M12 6.5v6.5 M9.2 10.2L12 13l2.8-2.8"),
    Files("M3 5h6l2 2.5h10v11.5H3z M8.5 12h7 M13.5 10l2 2-2 2 M15.5 15.5h-7 M10.5 13.5l-2 2 2 2"),
    Sources("M3 4h14v11H3z M3 8h14 M18 13v7 M14.5 16.5h7 M7 19h5"),
    Refresh("M20 7a9 9 0 1 0 1 9 M20 3v5h-5"),
    Close("M6 6l12 12 M18 6L6 18"),
    Back("M14 5l-7 7 7 7"),
    Home("M20 12a8 8 0 1 1-16 0 8 8 0 0 1 16 0"),
    More("M5 11a1 1 0 1 0 0 2 1 1 0 1 0 0-2z M12 11a1 1 0 1 0 0 2 1 1 0 1 0 0-2z M19 11a1 1 0 1 0 0 2 1 1 0 1 0 0-2z"),
    Control("M5.5 3.5l13 7.5-5.8 1.7-2.7 5.8-4.5-15z M13.5 13.5l4.5 4.5"),
    Touch("M9 13V5a2 2 0 0 1 4 0v6 M13 9l6 3v6l-3 3h-5l-6-7 2-2 2 1"),
    Trackpad("M3 5h18v14H3z M3 14h18 M12 14v5 M8 9h8"),
    Audio("M11 5L6 9H3v6h3l5 4z M15 8a6 6 0 0 1 0 8 M18 5a10 10 0 0 1 0 14"),
    Notifications("M6 9a6 6 0 0 1 12 0v6l2 3H4l2-3z M10 21h4"),
    Rotate("M4 7a3 3 0 0 1 3-3h7a3 3 0 0 1 3 3v4a3 3 0 0 1-3 3H7a3 3 0 0 1-3-3z M17 10h2a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2h-8a2 2 0 0 1-2-2v-2"),
    Collapse("M6 15l6-6 6 6"),
    Keyboard("M2 5h20v14H2z M6 9h.01 M10 9h.01 M14 9h.01 M18 9h.01 M6 13h.01 M10 13h.01 M14 13h.01 M18 13h.01 M7 16h10");

    val image: ImageVector = ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f)
        .addPath(PathParser().parsePathString(path).toNodes(), fill = null,
            stroke = SolidColor(Color.White), strokeLineWidth = 1.7f,
            strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        .build()
}

@Composable
fun MobileAction(symbol: MobileSymbol, label: String, enabled: Boolean = true,
    modifier: Modifier = Modifier, selected: Boolean = false, onClick: () -> Unit) {
    IconButton(onClick = onClick, enabled = enabled, modifier = modifier
        .background(if (selected) ColorAccentCyan.copy(alpha = .12f) else Color.Transparent, RoundedCornerShape(24.dp))) {
        Icon(symbol.image, contentDescription = label,
            tint = if (!enabled) ColorTextSecondary.copy(alpha = .4f) else if (selected) ColorAccentCyan else ColorTextPrimary)
    }
}

private val mobileColorScheme = lightColorScheme(
        primary = ColorAccentCyan, onPrimary = Color.White,
        secondary = ColorAccentEmerald, onSecondary = Color.White,
        background = ColorBaseBg, onBackground = ColorTextPrimary,
        surface = ColorSurfaceCard, onSurface = ColorTextPrimary,
        surfaceVariant = Color(0xFFF1F5F9), onSurfaceVariant = ColorTextSecondary,
        outline = Color(0xFF94A3B8), outlineVariant = ColorBorderFine,
        error = Color(0xFFB42318),
)
private val mobileShapes = Shapes(small = RoundedCornerShape(12.dp), medium = RoundedCornerShape(18.dp),
    large = RoundedCornerShape(24.dp))

@Composable
fun WeaveloTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = mobileColorScheme, shapes = mobileShapes, content = content)
}

/** The four persistent input actions live outside the measured video rectangle. */
@Composable
internal fun MobileInputControls(trackpad: Boolean, direct: Boolean, inputEnabled: Boolean,
    onTrackpad: () -> Unit, onDirect: () -> Unit, onKeyboard: () -> Unit, onReset: () -> Unit) {
    Box(Modifier.fillMaxWidth().windowInsetsPadding(WindowInsets.mandatorySystemGestures.only(WindowInsetsSides.Bottom))
        .padding(horizontal = 12.dp, vertical = 8.dp), contentAlignment = Alignment.Center) {
        Surface(shape = RoundedCornerShape(28.dp), color = ColorSurfaceCard.copy(alpha = .94f), shadowElevation = 6.dp) {
            Row(Modifier.padding(4.dp), verticalAlignment = Alignment.CenterVertically) {
                MobileAction(MobileSymbol.Trackpad, tr("Trackpad"), inputEnabled, Modifier.size(48.dp), selected = trackpad, onClick = onTrackpad)
                MobileAction(MobileSymbol.Touch, tr("Direct touch"), inputEnabled, Modifier.size(48.dp), selected = direct, onClick = onDirect)
                MobileAction(MobileSymbol.Keyboard, tr("Show keyboard"), inputEnabled, Modifier.size(48.dp), onClick = onKeyboard)
                MobileAction(MobileSymbol.Refresh, tr("Reset zoom"), modifier = Modifier.size(48.dp), onClick = onReset)
            }
        }
    }
}
