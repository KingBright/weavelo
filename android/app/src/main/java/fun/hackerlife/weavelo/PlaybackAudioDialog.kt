package `fun`.hackerlife.weavelo

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.HorizontalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import `fun`.hackerlife.weavelo.ui.MobileSymbol
import `fun`.hackerlife.weavelo.ui.PrecisionButton
import `fun`.hackerlife.weavelo.ui.PrecisionButtonStyle
import `fun`.hackerlife.weavelo.ui.PrecisionDialog
import `fun`.hackerlife.weavelo.ui.PrecisionMenuItem
import `fun`.hackerlife.weavelo.ui.WeaveloTokens
import `fun`.hackerlife.weavelo.ui.tr
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

@Composable
internal fun PlaybackAudioDialog(
    showVideoOnly: Boolean,
    onSelect: (PlaybackAudioTarget?) -> Unit,
    onVideoOnly: () -> Unit,
    onDismiss: () -> Unit,
) {
    val p = WeaveloTokens.colors
    val context = LocalContext.current
    var choices by remember { mutableStateOf<List<PlaybackAudioChoice>>(emptyList()) }
    LaunchedEffect(Unit) { choices = withContext(Dispatchers.IO) { PlaybackAudioSources.choices(context) } }
    val duplicateLabels = remember(choices) {
        choices.groupingBy { it.label.trim().lowercase() }.eachCount().filterValues { it > 1 }.keys
    }
    PrecisionDialog(
        title = tr("Choose audio to share"),
        subtitle = tr("For app audio, select the same app in Android's screen picker. Some apps prevent capture. Phone audio stays on; computer-only playback is unavailable."),
        onDismissRequest = onDismiss,
        dismissButton = if (showVideoOnly) {
            {
                PrecisionButton(
                    text = tr("Share without audio"),
                    leadingSymbol = MobileSymbol.AudioMuted,
                    style = PrecisionButtonStyle.TONAL,
                    compact = true,
                    onClick = onVideoOnly,
                )
            }
        } else null,
        confirmButton = {
            PrecisionButton(
                text = tr("Cancel"),
                style = PrecisionButtonStyle.SUBTLE,
                compact = true,
                onClick = onDismiss,
            )
        },
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            LazyColumn(Modifier.fillMaxWidth().heightIn(max = 320.dp)) {
                item {
                    PrecisionMenuItem(
                        text = tr("All eligible device audio"),
                        symbol = MobileSymbol.Audio,
                        onClick = { onSelect(null) },
                    )
                    if (choices.isNotEmpty()) {
                        HorizontalDivider(color = p.borderMuted)
                    }
                }
                items(choices, key = { it.target.packageName }) { choice ->
                    val showPackage = choice.label.trim().lowercase() in duplicateLabels
                    PrecisionMenuItem(
                        text = choice.label,
                        subtitle = if (showPackage) choice.target.packageName else null,
                        symbol = MobileSymbol.Sources,
                        onClick = { onSelect(choice.target) },
                    )
                }
            }
        }
    }
}
