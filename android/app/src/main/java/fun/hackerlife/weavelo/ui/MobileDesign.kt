package `fun`.hackerlife.weavelo.ui

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
    Devices("M3 4h18v13H3z M8 21h8 M12 17v4"),
    Network("M9 3h6v5H9z M3 16h6v5H3z M15 16h6v5h-6z M12 8v4 M6 16v-4h12v4"),
    Share("M3 12v7h18v-7 M12 16V3 M7 8l5-5 5 5"),
    Settings("M4 6h16 M4 12h16 M4 18h16 M8 3v6 M16 9v6 M10 15v6"),
    Scan("M3 8V3h5 M16 3h5v5 M21 16v5h-5 M8 21H3v-5 M8 8h2v2H8z M14 8h2v2h-2z M8 14h2v2H8z M14 14h3v3h-3z"),
    Invitation("M3 5h18v14H3z M3 6l9 7 9-7"),
    Open("M5 19L19 5 M8 5h11v11"),
    Files("M3 7V4h6l2 3h10v13H3z"),
    Sources("M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z"),
    Refresh("M20 7a9 9 0 1 0 1 9 M20 3v5h-5"),
    Close("M6 6l12 12 M18 6L6 18"),
    Back("M14 5l-7 7 7 7"),
    More("M5 12h.01 M12 12h.01 M19 12h.01"),
    Control("M5 3l14 9-6 1-3 6z M13 14l4 6"),
    Audio("M11 5L6 9H3v6h3l5 4z M15 8a6 6 0 0 1 0 8 M18 5a10 10 0 0 1 0 14"),
    Notifications("M6 9a6 6 0 0 1 12 0v6l2 3H4l2-3z M10 21h4"),
    Keyboard("M2 5h20v14H2z M6 9h.01 M10 9h.01 M14 9h.01 M18 9h.01 M6 13h.01 M10 13h.01 M14 13h.01 M18 13h.01 M7 16h10");

    val image: ImageVector = ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f)
        .addPath(PathParser().parsePathString(path).toNodes(), fill = null,
            stroke = SolidColor(Color.White), strokeLineWidth = 1.7f,
            strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round)
        .build()
}

@Composable
fun MobileAction(symbol: MobileSymbol, label: String, enabled: Boolean = true,
    modifier: Modifier = Modifier, onClick: () -> Unit) {
    IconButton(onClick = onClick, enabled = enabled, modifier = modifier) {
        Icon(symbol.image, contentDescription = label,
            tint = if (enabled) ColorTextPrimary else ColorTextSecondary.copy(alpha = .4f))
    }
}

private val mobileColorScheme = darkColorScheme(
        primary = ColorAccentEmerald, onPrimary = Color(0xFF10281F),
        secondary = ColorAccentCyan, onSecondary = Color(0xFF10252D),
        background = ColorBaseBg, onBackground = ColorTextPrimary,
        surface = ColorSurfaceCard, onSurface = ColorTextPrimary,
        surfaceVariant = Color(0xFF222A34), onSurfaceVariant = ColorTextSecondary,
        outline = Color(0xFF3B4552), outlineVariant = ColorBorderFine,
        error = Color(0xFFFFB4A9),
)
private val mobileShapes = Shapes(small = RoundedCornerShape(12.dp), medium = RoundedCornerShape(18.dp),
    large = RoundedCornerShape(24.dp))

@Composable
fun WeaveloTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = mobileColorScheme, shapes = mobileShapes, content = content)
}
