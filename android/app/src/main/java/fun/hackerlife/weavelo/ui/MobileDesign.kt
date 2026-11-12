package `fun`.hackerlife.weavelo.ui

import android.app.Activity
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathFillType
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.view.WindowCompat
import kotlin.math.min

/**
 * Unified Weavelo Brand Token System:
 * - Light: Prism Air (Cool mist alabaster + deep obsidian ink + authentic Weavelo prism teal #087A8A)
 * - Dark:  Obsidian Craft (Layered micro-crystal graphite + luminous prism cyan #38CFE0)
 */
@Immutable
data class WeaveloPalette(
    val isDark: Boolean,
    val canvas: Color,
    val surface: Color,
    val surfaceRaised: Color,
    val surfaceSunken: Color,
    val surfaceHover: Color,
    val surfaceGlass: Color,
    val border: Color,
    val borderMuted: Color,
    val borderStrong: Color,
    val textPrimary: Color,
    val textSecondary: Color,
    val textTertiary: Color,
    val brand: Color,
    val brandOn: Color,
    val brandSoft: Color,
    val brandBorder: Color,
    val prismCyan: Color,
    val prismCyanSoft: Color,
    val emeraldText: Color,
    val emeraldDot: Color,
    val emeraldSoft: Color,
    val warningText: Color,
    val warningSoft: Color,
    val warningBorder: Color,
    val danger: Color,
    val dangerOn: Color,
    val dangerText: Color,
    val dangerSoft: Color,
    val dangerBorder: Color,
)

val PrismAirPalette = WeaveloPalette(
    isDark = false,
    canvas = Color(0xFFF4F7FA),
    surface = Color(0xFFFFFFFF),
    surfaceRaised = Color(0xFFFFFFFF),
    surfaceSunken = Color(0xFFE9EFF5),
    surfaceHover = Color(0xFFDFE8F0),
    surfaceGlass = Color(0xF5FFFFFF),
    border = Color(0xFFD8E2EC),
    borderMuted = Color(0xFFE5EDF4),
    borderStrong = Color(0xFFBFD0DF),
    textPrimary = Color(0xFF0F172A),
    textSecondary = Color(0xFF475569),
    textTertiary = Color(0xFF64748B),
    brand = Color(0xFF087A8A),
    brandOn = Color(0xFFFFFFFF),
    brandSoft = Color(0xFFE3F3F5),
    brandBorder = Color(0xFF7CCAD4),
    prismCyan = Color(0xFF087A8A),
    prismCyanSoft = Color(0xFFE3F3F5),
    emeraldText = Color(0xFF057A55),
    emeraldDot = Color(0xFF10B981),
    emeraldSoft = Color(0xFFDCFCE7),
    warningText = Color(0xFF925006),
    warningSoft = Color(0xFFFEF3DC),
    warningBorder = Color(0xFFE5A93B),
    danger = Color(0xFFC83A37),
    dangerOn = Color(0xFFFFFFFF),
    dangerText = Color(0xFFB42318),
    dangerSoft = Color(0xFFFEECEB),
    dangerBorder = Color(0xFFF2A6A2),
)

val ObsidianCraftPalette = WeaveloPalette(
    isDark = true,
    canvas = Color(0xFF0F131A),
    surface = Color(0xFF161C26),
    surfaceRaised = Color(0xFF1D2533),
    surfaceSunken = Color(0xFF0A0D13),
    surfaceHover = Color(0xFF242E3F),
    surfaceGlass = Color(0xF0171D28),
    border = Color(0xFF253041),
    borderMuted = Color(0xFF1B2331),
    borderStrong = Color(0xFF344258),
    textPrimary = Color(0xFFF1F5F9),
    textSecondary = Color(0xFF9AA8BC),
    textTertiary = Color(0xFF6E7E94),
    brand = Color(0xFF38CFE0),
    brandOn = Color(0xFF04181D),
    brandSoft = Color(0xFF112B33),
    brandBorder = Color(0xFF1F6878),
    prismCyan = Color(0xFF38CFE0),
    prismCyanSoft = Color(0xFF112B33),
    emeraldText = Color(0xFF34D399),
    emeraldDot = Color(0xFF34D399),
    emeraldSoft = Color(0xFF112C24),
    warningText = Color(0xFFFBBF24),
    warningSoft = Color(0xFF2B2111),
    warningBorder = Color(0xFF785314),
    danger = Color(0xFFF87171),
    dangerOn = Color(0xFF1E0808),
    dangerText = Color(0xFFFCA5A5),
    dangerSoft = Color(0xFF2E1619),
    dangerBorder = Color(0xFF6B282C),
)

val LocalWeaveloPalette = staticCompositionLocalOf { PrismAirPalette }

object WeaveloTokens {
    val colors: WeaveloPalette
        @Composable get() = LocalWeaveloPalette.current
}

// Reactive semantic aliases kept compatible with all existing mobile screens
val ColorBaseBg: Color @Composable get() = LocalWeaveloPalette.current.canvas
val ColorSurfaceCard: Color @Composable get() = LocalWeaveloPalette.current.surface
val ColorBorderFine: Color @Composable get() = LocalWeaveloPalette.current.border
val ColorAccentCyan: Color @Composable get() = LocalWeaveloPalette.current.brand
val ColorAccentEmerald: Color @Composable get() = LocalWeaveloPalette.current.emeraldText
val ColorStatusGreen: Color @Composable get() = LocalWeaveloPalette.current.emeraldDot
val ColorTextPrimary: Color @Composable get() = LocalWeaveloPalette.current.textPrimary
val ColorTextSecondary: Color @Composable get() = LocalWeaveloPalette.current.textSecondary

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
    Close("M6.5 6.5l11 11 M17.5 6.5l-11 11"),
    Check("M5 12.5l4.5 4.5L19 7.5"),
    ChevronRight("M9 6l6 6-6 6"),
    Copy("M9 9h10v11H9z M5 15H4V4h11v1"),
    Download("M12 4v11 M8 11.5L12 15.5l4-4 M5 19h14"),
    Folder("M3 6h6l2 2h10v11H3z"),
    Clipboard("M9 4h6v3H9z M6 6h3v2h6V6h3v14H6z"),
    Grid("M4 4h7v7H4z M13 4h7v7h-7z M4 13h7v7H4z M13 13h7v7h-7z"),
    Disconnect("M15 4h4v16h-4 M10 17l5-5-5-5 M15 12H3"),
    Back("M14 5l-7 7 7 7"),
    Home("M20 12a8 8 0 1 1-16 0 8 8 0 0 1 16 0"),
    More("M5 11a1 1 0 1 0 0 2 1 1 0 1 0 0-2z M12 11a1 1 0 1 0 0 2 1 1 0 1 0 0-2z M19 11a1 1 0 1 0 0 2 1 1 0 1 0 0-2z"),
    Control("M5.5 3.5l13 7.5-5.8 1.7-2.7 5.8-4.5-15z M13.5 13.5l4.5 4.5"),
    Touch("M9 13V5a2 2 0 0 1 4 0v6 M13 9l6 3v6l-3 3h-5l-6-7 2-2 2 1"),
    Trackpad("M3 5h18v14H3z M3 14h18 M12 14v5 M8 9h8"),
    Pan("M12 3v18 M3 12h18 M9 6l3-3 3 3 M9 18l3 3 3-3 M6 9l-3 3 3 3 M18 9l3 3-3 3"),
    ExpandVertical("M12 3v18 M8.5 6.5L12 3l3.5 3.5 M8.5 17.5L12 21l3.5-3.5 M5 3h14 M5 21h14"),
    Audio("M11 5L6 9H3v6h3l5 4z M15 8a6 6 0 0 1 0 8 M18 5a10 10 0 0 1 0 14"),
    AudioMuted("M11 5L6 9H3v6h3l5 4z M16 9l6 6 M22 9l-6 6"),
    Notifications("M6 9a6 6 0 0 1 12 0v6l2 3H4l2-3z M10 21h4"),
    Rotate("M4 7a3 3 0 0 1 3-3h7a3 3 0 0 1 3 3v4a3 3 0 0 1-3 3H7a3 3 0 0 1-3-3z M17 10h2a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2h-8a2 2 0 0 1-2-2v-2"),
    FullscreenEnter("M4 9V5.5A1.5 1.5 0 0 1 5.5 4H9 M15 4h3.5A1.5 1.5 0 0 1 20 5.5V9 M20 15v3.5a1.5 1.5 0 0 1-1.5 1.5H15 M9 20H5.5A1.5 1.5 0 0 1 4 18.5V15"),
    FullscreenExit("M9 4v3.5A1.5 1.5 0 0 1 7.5 9H4 M15 4v3.5A1.5 1.5 0 0 0 16.5 9H20 M20 15h-3.5A1.5 1.5 0 0 0 15 16.5V20 M4 15h3.5A1.5 1.5 0 0 1 9 16.5V20"),
    Collapse("M6 15l6-6 6 6"),
    Keyboard("M2 5h20v14H2z M6 9h.01 M10 9h.01 M14 9h.01 M18 9h.01 M6 13h.01 M10 13h.01 M14 13h.01 M18 13h.01 M7 16h10");

    val image: ImageVector = ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f)
        .addPath(
            PathParser().parsePathString(path).toNodes(),
            fill = null,
            stroke = SolidColor(Color.White),
            strokeLineWidth = 1.75f,
            strokeLineCap = StrokeCap.Round,
            strokeLineJoin = StrokeJoin.Round
        )
        .build()
}

private val lightMaterialScheme = lightColorScheme(
    primary = PrismAirPalette.brand,
    onPrimary = PrismAirPalette.brandOn,
    primaryContainer = PrismAirPalette.brandSoft,
    onPrimaryContainer = PrismAirPalette.brand,
    secondary = PrismAirPalette.prismCyan,
    onSecondary = Color.White,
    secondaryContainer = PrismAirPalette.prismCyanSoft,
    onSecondaryContainer = PrismAirPalette.prismCyan,
    background = PrismAirPalette.canvas,
    onBackground = PrismAirPalette.textPrimary,
    surface = PrismAirPalette.surface,
    onSurface = PrismAirPalette.textPrimary,
    surfaceVariant = PrismAirPalette.surfaceSunken,
    onSurfaceVariant = PrismAirPalette.textSecondary,
    outline = PrismAirPalette.borderStrong,
    outlineVariant = PrismAirPalette.border,
    error = PrismAirPalette.danger,
    onError = PrismAirPalette.dangerOn,
)

private val darkMaterialScheme = darkColorScheme(
    primary = ObsidianCraftPalette.brand,
    onPrimary = ObsidianCraftPalette.brandOn,
    primaryContainer = ObsidianCraftPalette.brandSoft,
    onPrimaryContainer = ObsidianCraftPalette.brand,
    secondary = ObsidianCraftPalette.emeraldText,
    onSecondary = ObsidianCraftPalette.brandOn,
    secondaryContainer = ObsidianCraftPalette.emeraldSoft,
    onSecondaryContainer = ObsidianCraftPalette.emeraldText,
    background = ObsidianCraftPalette.canvas,
    onBackground = ObsidianCraftPalette.textPrimary,
    surface = ObsidianCraftPalette.surface,
    onSurface = ObsidianCraftPalette.textPrimary,
    surfaceVariant = ObsidianCraftPalette.surfaceSunken,
    onSurfaceVariant = ObsidianCraftPalette.textSecondary,
    outline = ObsidianCraftPalette.borderStrong,
    outlineVariant = ObsidianCraftPalette.border,
    error = ObsidianCraftPalette.danger,
    onError = ObsidianCraftPalette.dangerOn,
)

private val mobileShapes = Shapes(
    small = RoundedCornerShape(12.dp),
    medium = RoundedCornerShape(16.dp),
    large = RoundedCornerShape(24.dp)
)

@Composable
fun WeaveloTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    content: @Composable () -> Unit
) {
    val palette = if (darkTheme) ObsidianCraftPalette else PrismAirPalette
    val colorScheme = if (darkTheme) darkMaterialScheme else lightMaterialScheme
    val view = LocalView.current
    if (!view.isInEditMode) {
        SideEffect {
            (view.context as? Activity)?.window?.let { window ->
                WindowCompat.getInsetsController(window, view).apply {
                    isAppearanceLightStatusBars = !darkTheme
                    isAppearanceLightNavigationBars = !darkTheme
                }
            }
        }
    }
    CompositionLocalProvider(LocalWeaveloPalette provides palette) {
        MaterialTheme(
            colorScheme = colorScheme,
            shapes = mobileShapes,
            content = content
        )
    }
}

// Exact original approved Weavelo mark geometry from assets/brand/weavelo/app-square.svg (only color adapts to theme)
private val OriginalWeaveloMarkVector: ImageVector by lazy {
    ImageVector.Builder("WeaveloOriginalMark", 24.dp, 24.dp, 100f, 100f)
        .addPath(
            pathData = PathParser().parsePathString(
                "M84.04012,33.05061 L88.14134,37.15183 L89.13349,38.33297 L89.99793,39.72531 L90.71118,41.24674 L91.24974,42.81514 L91.59010,44.34840 L91.70878,45.76439 L91.59010,47.18038 L91.24974,48.71364 L90.71118,50.28204 L89.99793,51.80347 L89.13349,53.19581 L88.14134,54.37695 L73.37695,69.14134 L72.19581,70.13349 L70.80347,70.99793 L69.28204,71.71118 L67.71364,72.24974 L66.18038,72.59010 L64.76439,72.70878 L63.34840,72.59010 L61.81514,72.24974 L60.24674,71.71118 L58.72531,70.99793 L57.33297,70.13349 L56.15183,69.14134 L29.08378,42.07329 L38.10646,33.05061 L64.76439,59.70854 L78.70854,45.76439 L75.01744,42.07329 L84.04012,33.05061 M11.85866,54.37695 L10.86651,53.19581 L10.00207,51.80347 L9.28882,50.28204 L8.75026,48.71364 L8.40990,47.18038 L8.29122,45.76439 L8.40990,44.34840 L8.75026,42.81514 L9.28882,41.24674 L10.00207,39.72531 L10.86651,38.33297 L11.85866,37.15183 L19.17049,29.84000 L37.21585,29.84000 L21.29146,45.76439 L35.23561,59.70854 L38.92671,56.01744 L47.94939,65.04012 L43.84817,69.14134 L42.66703,70.13349 L41.27469,70.99793 L39.75326,71.71118 L38.18486,72.24974 L36.65160,72.59010 L35.23561,72.70878 L33.81962,72.59010 L32.28636,72.24974 L30.71796,71.71118 L29.19653,70.99793 L27.80419,70.13349 L26.62305,69.14134 L11.85866,54.37695 M83.14951,29.84000 L61.07329,51.91622 L52.05061,42.89354 L65.10415,29.84000"
            ).toNodes(),
            pathFillType = PathFillType.EvenOdd,
            fill = SolidColor(Color.White)
        )
        .build()
}

/** Original Weavelo brand icon design (exact app-square.svg / mark-dark.svg geometry), recolored to active theme. */
@Composable
fun WeaveloBrandEmblem(size: Dp = 28.dp, modifier: Modifier = Modifier) {
    val p = WeaveloTokens.colors
    val bg = if (p.isDark) p.brandSoft else p.brand
    val fg = if (p.isDark) p.brand else p.brandOn
    Box(
        modifier = modifier
            .size(size)
            .clip(RoundedCornerShape(size * 0.24f))
            .background(bg)
            .then(
                if (p.isDark) Modifier.border(1.dp, p.brandBorder, RoundedCornerShape(size * 0.24f))
                else Modifier
            ),
        contentAlignment = Alignment.Center
    ) {
        Icon(
            imageVector = OriginalWeaveloMarkVector,
            contentDescription = "Weavelo",
            tint = fg,
            modifier = Modifier.size(size)
        )
    }
}

/** Tactile Precision Craft card container with subtle hairline border and optional active focus state. */
@Composable
fun PrecisionCard(
    modifier: Modifier = Modifier,
    highlighted: Boolean = false,
    sunken: Boolean = false,
    cornerRadius: Dp = 16.dp,
    contentPadding: PaddingValues = PaddingValues(14.dp),
    onClick: (() -> Unit)? = null,
    enabled: Boolean = true,
    verticalSpacing: Dp = 0.dp,
    content: @Composable ColumnScope.() -> Unit
) {
    val p = WeaveloTokens.colors
    val shape = RoundedCornerShape(cornerRadius)
    val bg = when {
        highlighted -> p.brandSoft
        sunken -> p.surfaceSunken
        else -> p.surface
    }
    val borderColor = when {
        highlighted -> p.brandBorder
        sunken -> p.borderMuted
        else -> p.border
    }
    Column(
        modifier = modifier
            .fillMaxWidth()
            .clip(shape)
            .background(bg)
            .border(1.dp, borderColor, shape)
            .let { mod ->
                if (onClick != null) mod.clickable(enabled = enabled, onClick = onClick) else mod
            }
            .padding(contentPadding),
        verticalArrangement = Arrangement.spacedBy(verticalSpacing),
        content = content
    )
}

enum class PillTone { BRAND, CYAN, EMERALD, WARNING, DANGER, NEUTRAL }

@Composable
fun PrecisionPillBadge(
    text: String,
    tone: PillTone = PillTone.NEUTRAL,
    showDot: Boolean = false,
    monospace: Boolean = false,
    modifier: Modifier = Modifier
) {
    val p = WeaveloTokens.colors
    val (bg, fg, dot) = when (tone) {
        PillTone.BRAND -> Triple(p.brandSoft, p.brand, p.brand)
        PillTone.CYAN -> Triple(p.prismCyanSoft, p.prismCyan, p.prismCyan)
        PillTone.EMERALD -> Triple(p.emeraldSoft, p.emeraldText, p.emeraldDot)
        PillTone.WARNING -> Triple(p.warningSoft, p.warningText, p.warningText)
        PillTone.DANGER -> Triple(p.dangerSoft, p.dangerText, p.danger)
        PillTone.NEUTRAL -> Triple(p.surfaceSunken, p.textSecondary, p.textTertiary)
    }
    Row(
        modifier = modifier
            .clip(RoundedCornerShape(999.dp))
            .background(bg)
            .padding(horizontal = 8.dp, vertical = 3.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(5.dp)
    ) {
        if (showDot) {
            Box(Modifier.size(6.dp).background(dot, CircleShape))
        }
        Text(
            text = text,
            color = fg,
            fontSize = 10.5.sp,
            lineHeight = 14.sp,
            fontWeight = FontWeight.SemiBold,
            fontFamily = if (monospace) FontFamily.Monospace else FontFamily.Default,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis
        )
    }
}

enum class PrecisionButtonStyle { PRIMARY, TONAL, SUBTLE, DANGER, DANGER_TONAL }

@Composable
fun PrecisionButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    leadingSymbol: MobileSymbol? = null,
    style: PrecisionButtonStyle = PrecisionButtonStyle.PRIMARY,
    compact: Boolean = false
) {
    val p = WeaveloTokens.colors
    val shape = RoundedCornerShape(if (compact) 10.dp else 12.dp)
    val (bg, fg, borderCol) = when (style) {
        PrecisionButtonStyle.PRIMARY -> Triple(p.brand, p.brandOn, Color.Transparent)
        PrecisionButtonStyle.TONAL -> Triple(p.brandSoft, p.brand, p.brandBorder.copy(alpha = 0.55f))
        PrecisionButtonStyle.SUBTLE -> Triple(p.surfaceSunken, p.textPrimary, p.border)
        PrecisionButtonStyle.DANGER -> Triple(p.danger, p.dangerOn, Color.Transparent)
        PrecisionButtonStyle.DANGER_TONAL -> Triple(p.dangerSoft, p.dangerText, p.dangerBorder.copy(alpha = 0.65f))
    }
    val alpha = if (enabled) 1f else 0.44f
    Row(
        modifier = modifier
            .heightIn(min = if (compact) 34.dp else 40.dp)
            .clip(shape)
            .background(bg.copy(alpha = bg.alpha * alpha))
            .let { mod ->
                if (borderCol != Color.Transparent) mod.border(1.dp, borderCol.copy(alpha = borderCol.alpha * alpha), shape)
                else mod
            }
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .padding(horizontal = if (compact) 11.dp else 14.dp, vertical = if (compact) 6.dp else 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.Center
    ) {
        if (leadingSymbol != null) {
            Icon(
                leadingSymbol.image,
                contentDescription = null,
                tint = fg.copy(alpha = alpha),
                modifier = Modifier.size(if (compact) 15.dp else 17.dp)
            )
            Spacer(Modifier.width(6.dp))
        }
        Text(
            text = text,
            color = fg.copy(alpha = alpha),
            fontSize = if (compact) 12.sp else 13.sp,
            fontWeight = FontWeight.SemiBold,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis
        )
    }
}

/**
 * Unclipped soft-tonal icon action button: avoids Material IconButton's circular clip
 * (which previously sliced off rounded-square corners) and uses clean borderless tonal fills.
 */
@Composable
fun MobileAction(
    symbol: MobileSymbol,
    label: String,
    enabled: Boolean = true,
    modifier: Modifier = Modifier,
    selected: Boolean = false,
    tonalWell: Boolean = false,
    destructive: Boolean = false,
    onClick: () -> Unit
) {
    val p = WeaveloTokens.colors
    val shape = RoundedCornerShape(12.dp)
    val wellBg = when {
        destructive -> if (p.isDark) p.danger.copy(alpha = 0.18f) else p.dangerSoft
        selected -> if (p.isDark) p.brand.copy(alpha = 0.20f) else p.brandSoft
        tonalWell -> if (p.isDark) Color.White.copy(alpha = 0.07f) else p.surfaceSunken.copy(alpha = 0.90f)
        else -> Color.Transparent
    }
    val iconTint = when {
        !enabled -> p.textTertiary.copy(alpha = 0.36f)
        destructive -> p.dangerText
        selected -> p.brand
        else -> p.textPrimary
    }
    Box(
        modifier = modifier
            .defaultMinSize(minWidth = 38.dp, minHeight = 38.dp)
            .padding(1.5.dp)
            .clip(shape)
            .background(wellBg)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick),
        contentAlignment = Alignment.Center
    ) {
        Icon(
            symbol.image,
            contentDescription = label,
            tint = iconTint,
            modifier = Modifier.size(18.5.dp)
        )
    }
}

/**
 * Minimalist video-player-style orientation toggle button (zero text, pure 4-corner fullscreen icon).
 */
@Composable
internal fun StreamOrientationToggleButton(
    isLandscape: Boolean,
    alpha: Float = 1f,
    modifier: Modifier = Modifier,
    onClick: () -> Unit
) {
    val shape = RoundedCornerShape(11.dp)
    val clampedAlpha = alpha.coerceIn(0.28f, 1f)
    Box(
        modifier = modifier
            .size(34.dp)
            .clip(shape)
            .background(Color(0x940B1018).copy(alpha = 0.58f * clampedAlpha))
            .border(0.75.dp, Color.White.copy(alpha = 0.15f * clampedAlpha), shape)
            .clickable(role = Role.Button, onClick = onClick),
        contentAlignment = Alignment.Center
    ) {
        Icon(
            imageVector = (if (isLandscape) MobileSymbol.FullscreenExit else MobileSymbol.FullscreenEnter).image,
            contentDescription = if (isLandscape) tr("Portrait view") else tr("Landscape view"),
            tint = Color.White.copy(alpha = 0.90f * clampedAlpha),
            modifier = Modifier.size(17.dp)
        )
    }
}

/** Recessed segmented track for mode toggles, language pickers, and audio routing. */
@Composable
fun <T> PrecisionSegmentedRow(
    options: List<Pair<T, String>>,
    selected: T,
    onSelect: (T) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true
) {
    val p = WeaveloTokens.colors
    Row(
        modifier = modifier
            .clip(RoundedCornerShape(12.dp))
            .background(p.surfaceSunken)
            .border(1.dp, p.borderMuted, RoundedCornerShape(12.dp))
            .padding(3.dp),
        horizontalArrangement = Arrangement.spacedBy(3.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        options.forEach { (value, label) ->
            val isSelected = value == selected
            Box(
                modifier = Modifier
                    .weight(1f, fill = false)
                    .heightIn(min = 32.dp)
                    .clip(RoundedCornerShape(9.dp))
                    .background(if (isSelected) p.surfaceRaised else Color.Transparent)
                    .let { mod ->
                        if (isSelected) mod.border(1.dp, if (p.isDark) p.brandBorder else p.border, RoundedCornerShape(9.dp))
                        else mod
                    }
                    .clickable(enabled = enabled, role = Role.Tab) { onSelect(value) }
                    .padding(horizontal = 12.dp, vertical = 5.dp),
                contentAlignment = Alignment.Center
            ) {
                Text(
                    text = label,
                    color = when {
                        !enabled -> p.textTertiary.copy(alpha = 0.45f)
                        isSelected -> p.brand
                        else -> p.textSecondary
                    },
                    fontSize = 12.sp,
                    fontWeight = if (isSelected) FontWeight.SemiBold else FontWeight.Medium,
                    maxLines = 1
                )
            }
        }
    }
}

/** Sculpted tactile toggle switch row replacing raw Checkbox + unaligned Switch rows. */
@Composable
fun PrecisionSwitchRow(
    title: String,
    subtitle: String? = null,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    enabled: Boolean = true,
    leadingSymbol: MobileSymbol? = null,
    modifier: Modifier = Modifier
) {
    val p = WeaveloTokens.colors
    Row(
        modifier = modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .clickable(enabled = enabled, role = Role.Switch) { onCheckedChange(!checked) }
            .padding(vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        if (leadingSymbol != null) {
            Box(
                modifier = Modifier
                    .size(36.dp)
                    .clip(RoundedCornerShape(10.dp))
                    .background(if (checked) p.brandSoft else p.surfaceSunken)
                    .border(1.dp, if (checked) p.brandBorder.copy(alpha = 0.5f) else p.borderMuted, RoundedCornerShape(10.dp)),
                contentAlignment = Alignment.Center
            ) {
                Icon(
                    leadingSymbol.image,
                    contentDescription = null,
                    tint = if (checked) p.brand else p.textSecondary,
                    modifier = Modifier.size(18.dp)
                )
            }
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(
                text = title,
                color = if (enabled) p.textPrimary else p.textTertiary,
                fontSize = 13.5.sp,
                fontWeight = FontWeight.SemiBold,
                lineHeight = 18.sp
            )
            if (!subtitle.isNullOrBlank()) {
                Text(
                    text = subtitle,
                    color = p.textSecondary,
                    fontSize = 11.5.sp,
                    lineHeight = 16.sp
                )
            }
        }
        Switch(
            checked = checked,
            onCheckedChange = onCheckedChange,
            enabled = enabled,
            colors = SwitchDefaults.colors(
                checkedThumbColor = p.brandOn,
                checkedTrackColor = p.brand,
                checkedBorderColor = Color.Transparent,
                uncheckedThumbColor = p.textSecondary,
                uncheckedTrackColor = p.surfaceSunken,
                uncheckedBorderColor = p.borderStrong
            )
        )
    }
}

/** Recessed precision text field replacing boxy default OutlinedTextField. */
@Composable
fun PrecisionTextField(
    value: String,
    onValueChange: (String) -> Unit,
    label: String,
    modifier: Modifier = Modifier,
    singleLine: Boolean = true,
    isError: Boolean = false,
    monospace: Boolean = false,
    keyboardOptions: KeyboardOptions = KeyboardOptions.Default
) {
    val p = WeaveloTokens.colors
    OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        label = { Text(label, fontSize = 12.sp) },
        singleLine = singleLine,
        isError = isError,
        keyboardOptions = keyboardOptions,
        textStyle = androidx.compose.ui.text.TextStyle(
            fontFamily = if (monospace) FontFamily.Monospace else FontFamily.Default
        ),
        shape = RoundedCornerShape(12.dp),
        colors = OutlinedTextFieldDefaults.colors(
            focusedTextColor = p.textPrimary,
            unfocusedTextColor = p.textPrimary,
            focusedContainerColor = p.surface,
            unfocusedContainerColor = p.surfaceSunken.copy(alpha = 0.65f),
            focusedBorderColor = p.brand,
            unfocusedBorderColor = p.border,
            errorBorderColor = p.danger,
            focusedLabelColor = p.brand,
            unfocusedLabelColor = p.textSecondary,
            cursorColor = p.brand
        ),
        modifier = modifier.fillMaxWidth()
    )
}

/**
 * Precision Craft Modal Sheet/Dialog: replaces stock Material AlertDialog across the product
 * with a sculpted card, icon-well header, circular close button, and tactile footer actions.
 */
@Composable
fun PrecisionDialog(
    title: String,
    onDismissRequest: () -> Unit,
    icon: MobileSymbol? = null,
    subtitle: String? = null,
    maxWidth: Dp = 440.dp,
    dismissButton: (@Composable () -> Unit)? = null,
    confirmButton: (@Composable () -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit
) {
    val p = WeaveloTokens.colors
    Dialog(
        onDismissRequest = onDismissRequest,
        properties = DialogProperties(usePlatformDefaultWidth = false)
    ) {
        Surface(
            modifier = Modifier
                .imePadding()
                .navigationBarsPadding()
                .widthIn(max = maxWidth)
                .fillMaxWidth(0.92f)
                .border(1.dp, p.borderStrong.copy(alpha = if (p.isDark) 0.85f else 0.55f), RoundedCornerShape(22.dp)),
            shape = RoundedCornerShape(22.dp),
            color = p.surfaceRaised,
            shadowElevation = 14.dp
        ) {
            Column(
                modifier = Modifier.padding(horizontal = 18.dp, vertical = 16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp)
            ) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(10.dp)
                ) {
                    if (icon != null) {
                        Box(
                            modifier = Modifier
                                .size(34.dp)
                                .clip(RoundedCornerShape(10.dp))
                                .background(p.brandSoft)
                                .border(1.dp, p.brandBorder.copy(alpha = 0.55f), RoundedCornerShape(10.dp)),
                            contentAlignment = Alignment.Center
                        ) {
                            Icon(icon.image, contentDescription = null, tint = p.brand, modifier = Modifier.size(18.dp))
                        }
                    }
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        Text(
                            text = title,
                            color = p.textPrimary,
                            fontSize = 16.sp,
                            fontWeight = FontWeight.SemiBold,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis
                        )
                        if (!subtitle.isNullOrBlank()) {
                            Text(
                                text = subtitle,
                                color = p.textSecondary,
                                fontSize = 11.5.sp,
                                lineHeight = 16.sp
                            )
                        }
                    }
                    MobileAction(
                        symbol = MobileSymbol.Close,
                        label = tr("Close"),
                        tonalWell = true,
                        modifier = Modifier.size(38.dp),
                        onClick = onDismissRequest
                    )
                }
                HorizontalDivider(color = p.borderMuted, thickness = 1.dp)
                Column(
                    modifier = Modifier.fillMaxWidth(),
                    verticalArrangement = Arrangement.spacedBy(10.dp),
                    content = content
                )
                if (dismissButton != null || confirmButton != null) {
                    HorizontalDivider(color = p.borderMuted, thickness = 1.dp)
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        dismissButton?.invoke()
                        confirmButton?.invoke()
                    }
                }
            }
        }
    }
}

/** Structured popover menu item with icon, active badge/check, and optional destructive styling. */
@Composable
fun PrecisionMenuItem(
    text: String,
    symbol: MobileSymbol,
    onClick: () -> Unit,
    enabled: Boolean = true,
    checked: Boolean = false,
    subtitle: String? = null,
    destructive: Boolean = false
) {
    val p = WeaveloTokens.colors
    val fg = when {
        !enabled -> p.textTertiary.copy(alpha = 0.42f)
        destructive -> p.dangerText
        checked -> p.brand
        else -> p.textPrimary
    }
    val iconTint = when {
        !enabled -> p.textTertiary.copy(alpha = 0.38f)
        destructive -> p.dangerText
        checked -> p.brand
        else -> p.textSecondary
    }
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .heightIn(min = 42.dp)
            .clip(RoundedCornerShape(10.dp))
            .background(if (checked) p.brandSoft.copy(alpha = 0.7f) else Color.Transparent)
            .clickable(enabled = enabled, role = Role.Button, onClick = onClick)
            .padding(horizontal = 10.dp, vertical = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp)
    ) {
        Icon(symbol.image, contentDescription = null, tint = iconTint, modifier = Modifier.size(18.dp))
        Column(Modifier.weight(1f)) {
            Text(
                text = text,
                color = fg,
                fontSize = 13.sp,
                fontWeight = if (checked || destructive) FontWeight.SemiBold else FontWeight.Medium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
            if (!subtitle.isNullOrBlank()) {
                Text(subtitle, color = p.textSecondary, fontSize = 11.sp, maxLines = 1,
                    overflow = TextOverflow.Ellipsis)
            }
        }
        if (checked) {
            Icon(MobileSymbol.Check.image, contentDescription = null, tint = p.brand, modifier = Modifier.size(16.dp))
        }
    }
}

/** Persistent input & canvas actions live outside the measured video rectangle. */
@Composable
internal fun MobileInputControls(
    trackpad: Boolean,
    direct: Boolean,
    panView: Boolean = false,
    inputEnabled: Boolean,
    onTrackpad: () -> Unit,
    onDirect: () -> Unit,
    onPanView: (() -> Unit)? = null,
    onKeyboard: () -> Unit,
    onReset: () -> Unit
) {
    val p = WeaveloTokens.colors
    Box(
        Modifier
            .fillMaxWidth()
            .windowInsetsPadding(WindowInsets.mandatorySystemGestures.only(WindowInsetsSides.Bottom))
            .padding(horizontal = 12.dp, vertical = 8.dp),
        contentAlignment = Alignment.Center
    ) {
        Surface(
            shape = RoundedCornerShape(24.dp),
            color = p.surfaceGlass,
            shadowElevation = 8.dp,
            modifier = Modifier.border(1.dp, p.border, RoundedCornerShape(24.dp))
        ) {
            Row(Modifier.padding(horizontal = 6.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                MobileAction(MobileSymbol.Trackpad, tr("Trackpad"), inputEnabled, Modifier.size(46.dp), selected = trackpad, onClick = onTrackpad)
                MobileAction(MobileSymbol.Touch, tr("Direct touch"), inputEnabled, Modifier.size(46.dp), selected = direct, onClick = onDirect)
                if (onPanView != null) {
                    MobileAction(MobileSymbol.Pan, tr("Pan canvas"), true, Modifier.size(46.dp), selected = panView, onClick = onPanView)
                }
                MobileAction(MobileSymbol.Keyboard, tr("Show keyboard"), inputEnabled, Modifier.size(46.dp), onClick = onKeyboard)
                MobileAction(MobileSymbol.Refresh, tr("Reset zoom"), modifier = Modifier.size(46.dp), onClick = onReset)
            }
        }
    }
}

/** Compact floating pill shown only when zoomed (> 1.05x) for 1-tap canvas pan or reset. */
@Composable
internal fun ZoomedCanvasChip(
    scale: Float,
    panActive: Boolean,
    canTogglePan: Boolean,
    modifier: Modifier = Modifier,
    onTogglePan: () -> Unit,
    onResetZoom: () -> Unit
) {
    val p = WeaveloTokens.colors
    Surface(
        modifier = modifier.border(1.dp, p.border, RoundedCornerShape(20.dp)),
        shape = RoundedCornerShape(20.dp),
        color = p.surfaceGlass,
        shadowElevation = 6.dp
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 5.dp, vertical = 3.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(3.dp)
        ) {
            if (canTogglePan) {
                Row(
                    modifier = Modifier
                        .heightIn(min = 30.dp)
                        .clip(RoundedCornerShape(15.dp))
                        .background(if (panActive) p.brandSoft else Color.Transparent)
                        .let { mod ->
                            if (panActive) mod.border(1.dp, p.brandBorder.copy(alpha = 0.6f), RoundedCornerShape(15.dp))
                            else mod
                        }
                        .clickable(role = Role.Button, onClick = onTogglePan)
                        .padding(horizontal = 9.dp, vertical = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(4.dp)
                ) {
                    Icon(
                        MobileSymbol.Pan.image,
                        contentDescription = null,
                        tint = if (panActive) p.brand else p.textPrimary,
                        modifier = Modifier.size(14.dp)
                    )
                    Text(
                        tr("Pan canvas"),
                        color = if (panActive) p.brand else p.textPrimary,
                        fontSize = 11.5.sp,
                        fontWeight = FontWeight.SemiBold
                    )
                }
            }
            Row(
                modifier = Modifier
                    .heightIn(min = 30.dp)
                    .clip(RoundedCornerShape(15.dp))
                    .background(p.surfaceSunken)
                    .clickable(role = Role.Button, onClick = onResetZoom)
                    .padding(horizontal = 9.dp, vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp)
            ) {
                Text(
                    String.format(java.util.Locale.ROOT, "%.1f×", scale),
                    color = p.brand,
                    fontSize = 11.5.sp,
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.Bold
                )
                Icon(
                    MobileSymbol.Refresh.image,
                    contentDescription = tr("Reset zoom"),
                    tint = p.textSecondary,
                    modifier = Modifier.size(13.dp)
                )
            }
        }
    }
}

/**
 * Signature Weavelo Aero-Prism remote cursor:
 * Compact, nimble split-tail dual-facet pointer with pearlescent contrast halo,
 * obsidian-slate left wing, luminous Weavelo cyan-cobalt right bevel, and tactile press pulse.
 */
@Composable
internal fun WeaveloPointerOverlay(
    normX: Float,
    normY: Float,
    pressing: Boolean,
    transform: ViewportTransform,
    baseWidth: Float,
    baseHeight: Float,
    modifier: Modifier = Modifier
) {
    val outerPath = remember { Path() }
    val leftFacetPath = remember { Path() }
    val rightFacetPath = remember { Path() }
    Canvas(modifier.fillMaxSize()) {
        val bw = baseWidth.coerceAtLeast(1f)
        val bh = baseHeight.coerceAtLeast(1f)
        val (tipX, tipY) = transform.normalizedToScreen(normX, normY, bw, bh, size.width, size.height)
        val minViewportDim = min(size.width, size.height)
        val len = min(14.5.dp.toPx(), minViewportDim * 0.052f).coerceIn(11.dp.toPx(), 14.5.dp.toPx())

        val leftWingX = 0.04f * len
        val leftWingY = 0.98f * len
        val notchX = 0.33f * len
        val notchY = 0.72f * len
        val rightWingX = 0.74f * len
        val rightWingY = 0.65f * len

        outerPath.reset()
        outerPath.moveTo(0f, 0f)
        outerPath.lineTo(leftWingX, leftWingY)
        outerPath.lineTo(notchX, notchY)
        outerPath.lineTo(rightWingX, rightWingY)
        outerPath.close()

        leftFacetPath.reset()
        leftFacetPath.moveTo(0f, 0f)
        leftFacetPath.lineTo(leftWingX, leftWingY)
        leftFacetPath.lineTo(notchX, notchY)
        leftFacetPath.close()

        rightFacetPath.reset()
        rightFacetPath.moveTo(0f, 0f)
        rightFacetPath.lineTo(notchX, notchY)
        rightFacetPath.lineTo(rightWingX, rightWingY)
        rightFacetPath.close()

        if (pressing) {
            drawCircle(
                color = Color(0x4238CFE0),
                radius = 5.2.dp.toPx(),
                center = Offset(tipX, tipY)
            )
            drawCircle(
                color = Color(0xB838CFE0),
                radius = 3.6.dp.toPx(),
                center = Offset(tipX, tipY),
                style = Stroke(width = 1.dp.toPx())
            )
        }

        translate(tipX + 0.85.dp.toPx(), tipY + 1.35.dp.toPx()) {
            drawPath(outerPath, Color(0x52020617))
        }

        translate(tipX, tipY) {
            drawPath(
                path = outerPath,
                color = Color(0xFFF8FAFC),
                style = Stroke(
                    width = 1.6.dp.toPx(),
                    cap = StrokeCap.Round,
                    join = StrokeJoin.Round
                )
            )
            drawPath(
                path = leftFacetPath,
                brush = Brush.linearGradient(
                    colors = listOf(Color(0xFF0F172A), Color(0xFF18343B)),
                    start = Offset.Zero,
                    end = Offset(notchX, leftWingY)
                )
            )
            drawPath(
                path = rightFacetPath,
                brush = Brush.linearGradient(
                    colors = listOf(Color(0xFF38CFE0), Color(0xFF087A8A)),
                    start = Offset.Zero,
                    end = Offset(rightWingX, notchY)
                )
            )
            drawLine(
                color = Color(0xB8D9F7FA),
                start = Offset(0.02f * len, 0.04f * len),
                end = Offset(notchX * 0.96f, notchY * 0.96f),
                strokeWidth = 0.75.dp.toPx(),
                cap = StrokeCap.Round
            )
            drawCircle(
                color = Color.White,
                radius = 0.85.dp.toPx(),
                center = Offset(0.03f * len, 0.05f * len)
            )
        }
    }
}
