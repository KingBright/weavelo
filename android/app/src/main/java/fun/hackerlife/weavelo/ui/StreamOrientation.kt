package `fun`.hackerlife.weavelo.ui

import android.app.Activity
import android.content.ContextWrapper
import android.content.pm.ActivityInfo
import androidx.core.view.ViewCompat
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.ui.platform.LocalContext

/** The stream owns its orientation; returning to devices restores the host choice. */
@Composable
internal fun StreamOrientation(active: Boolean) {
    val context = LocalContext.current
    DisposableEffect(context, active) {
        val activity = generateSequence(context) { (it as? ContextWrapper)?.baseContext }
            .filterIsInstance<Activity>().firstOrNull()
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
            // Let phone and desktop streams follow the device in either orientation.
            activity?.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_FULL_SENSOR
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
