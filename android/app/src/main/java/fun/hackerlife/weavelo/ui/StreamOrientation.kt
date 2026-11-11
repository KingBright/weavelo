package `fun`.hackerlife.weavelo.ui

import android.app.Activity
import android.content.ContextWrapper
import android.content.pm.ActivityInfo
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.ime
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat

/** The stream owns its orientation and immersive bars; closing IME re-hides system bars. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
internal fun StreamOrientation(active: Boolean, landscapeOverride: Boolean? = null) {
    val context = LocalContext.current
    val activity = generateSequence(context) { (it as? ContextWrapper)?.baseContext }
        .filterIsInstance<Activity>().firstOrNull()
    val imeVisible = WindowInsets.ime.getBottom(LocalDensity.current) > 0
    LaunchedEffect(activity, active, landscapeOverride) {
        if (active) {
            activity?.requestedOrientation = when (landscapeOverride) {
                true -> ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
                false -> ActivityInfo.SCREEN_ORIENTATION_SENSOR_PORTRAIT
                null -> ActivityInfo.SCREEN_ORIENTATION_FULL_SENSOR
            }
        }
    }
    LaunchedEffect(activity, active, imeVisible) {
        if (active && !imeVisible) {
            val controller = activity?.window?.let { WindowCompat.getInsetsController(it, it.decorView) }
            controller?.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            controller?.hide(WindowInsetsCompat.Type.systemBars())
        }
    }
    DisposableEffect(activity, active) {
        val previous = activity?.requestedOrientation
        val controller = activity?.window?.let { WindowCompat.getInsetsController(it, it.decorView) }
        val insets = activity?.window?.decorView?.let(ViewCompat::getRootWindowInsets)
        val bars = WindowInsetsCompat.Type.systemBars()
        val visibleBars = listOf(WindowInsetsCompat.Type.statusBars(), WindowInsetsCompat.Type.navigationBars())
            .filter { insets?.isVisible(it) != false }.fold(0) { mask, type -> mask or type }
        val previousBehavior = controller?.systemBarsBehavior
        val previousLightStatusBars = controller?.isAppearanceLightStatusBars
        val previousLightNavigationBars = controller?.isAppearanceLightNavigationBars
        if (active) {
            controller?.isAppearanceLightStatusBars = false
            controller?.isAppearanceLightNavigationBars = false
            controller?.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            controller?.hide(bars)
        }
        onDispose {
            if (active) {
                if (previous != null) activity?.requestedOrientation = previous
                if (previousBehavior != null) controller?.systemBarsBehavior = previousBehavior
                if (previousLightStatusBars != null) controller?.isAppearanceLightStatusBars = previousLightStatusBars
                if (previousLightNavigationBars != null) controller?.isAppearanceLightNavigationBars = previousLightNavigationBars
                controller?.hide(bars)
                if (visibleBars != 0) controller?.show(visibleBars)
            }
        }
    }
}
