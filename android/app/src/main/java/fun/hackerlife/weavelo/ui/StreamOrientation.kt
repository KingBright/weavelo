package `fun`.hackerlife.weavelo.ui

import android.app.Activity
import android.content.ContextWrapper
import android.content.pm.ActivityInfo
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
        if (active) activity?.requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
        onDispose {
            if (active && previous != null) activity?.requestedOrientation = previous
        }
    }
}
