package `fun`.hackerlife.weavelo

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import `fun`.hackerlife.weavelo.ui.tr
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

@Composable
internal fun PlaybackAudioDialog(showVideoOnly: Boolean, onSelect: (PlaybackAudioTarget?) -> Unit,
    onVideoOnly: () -> Unit, onDismiss: () -> Unit) {
    val context = LocalContext.current
    var choices by remember { mutableStateOf<List<PlaybackAudioChoice>>(emptyList()) }
    LaunchedEffect(Unit) { choices = withContext(Dispatchers.IO) { PlaybackAudioSources.choices(context) } }
    val duplicateLabels = remember(choices) {
        choices.groupingBy { it.label.trim().lowercase() }.eachCount().filterValues { it > 1 }.keys
    }
    AlertDialog(onDismissRequest = onDismiss,
        title = { Text(tr("Choose audio to share")) },
        text = { Column {
            Text(tr("For app audio, select the same app in Android's screen picker. Some apps prevent capture. Phone audio stays on; computer-only playback is unavailable."))
            LazyColumn(Modifier.fillMaxWidth().heightIn(max = 360.dp)) {
                item { TextButton(onClick = { onSelect(null) }) { Text(tr("All eligible device audio")) } }
                items(choices, key = { it.target.packageName }) { choice ->
                    val showPackage = choice.label.trim().lowercase() in duplicateLabels
                    TextButton(onClick = { onSelect(choice.target) }) {
                        Text(if (showPackage) "${choice.label} (${choice.target.packageName})" else choice.label)
                    }
                }
            }
        } },
        confirmButton = { TextButton(onClick = onDismiss) { Text(tr("Cancel")) } },
        dismissButton = { if (showVideoOnly) TextButton(onClick = onVideoOnly) { Text(tr("Share without audio")) } })
}
