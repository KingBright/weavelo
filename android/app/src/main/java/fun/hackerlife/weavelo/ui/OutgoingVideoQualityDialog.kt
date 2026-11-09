package `fun`.hackerlife.weavelo.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import `fun`.hackerlife.weavelo.OutgoingVideoQuality
import `fun`.hackerlife.weavelo.OutgoingVideoQualityStore
import `fun`.hackerlife.weavelo.OutgoingVideoRuntime
import `fun`.hackerlife.weavelo.PublisherClient
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

@Composable
internal fun OutgoingVideoQualityDialog(deliveryId: Long, onDismiss: () -> Unit) {
    val context = LocalContext.current.applicationContext
    val initial = remember { OutgoingVideoQualityStore.load(context) }
    var shortEdge by remember { mutableStateOf(initial.shortEdge.toString()) }
    var fps by remember { mutableStateOf(initial.fps.toString()) }
    var bitrate by remember { mutableStateOf(initial.bitrateKbps.toString()) }
    var error by remember { mutableStateOf<String?>(null) }
    var saving by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val encoder by OutgoingVideoRuntime.state.collectAsState()
    val invalidMessage = tr("Use 0 or 240–4320 pixels, 1–120 fps, and 100–100000 kbps.")
    val changedMessage = tr("Screen delivery changed; reopen settings")
    val updateFailedMessage = tr("Could not update screen quality")
    AlertDialog(onDismissRequest = { if (!saving) onDismiss() },
        title = { Text(tr("Outgoing screen quality")) },
        text = { Column(Modifier.fillMaxWidth().heightIn(max = 450.dp).verticalScroll(rememberScrollState())) {
            Text(tr("Changes apply to this screen delivery. Receiver, network and device limits may reduce quality."))
            if (deliveryId>0 && encoder.id==deliveryId) Text(trFormat("Configured encoder: %d × %d · %d fps · %d kbps",
                encoder.width,encoder.height,encoder.fps,encoder.bitrateKbps))
            else Text(tr("Encoder settings appear when a screen is sending."))
            OutlinedTextField(shortEdge, { shortEdge = it }, label = { Text(tr("Resolution short edge (0 = original)")) },
                singleLine = true, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number))
            OutlinedTextField(fps, { fps = it }, label = { Text(tr("Frames per second")) },
                singleLine = true, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number))
            OutlinedTextField(bitrate, { bitrate = it }, label = { Text(tr("Video bitrate (kbps)")) },
                singleLine = true, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number))
            error?.let { Text(it, color = androidx.compose.material3.MaterialTheme.colorScheme.error) }
        } },
        dismissButton = { TextButton(onClick = onDismiss, enabled = !saving) { Text(tr("Cancel")) } },
        confirmButton = { TextButton(enabled = !saving, onClick = {
            val edge = shortEdge.toIntOrNull(); val rate = fps.toIntOrNull(); val kbps = bitrate.toIntOrNull()
            val quality = if (edge != null && rate != null && kbps != null) OutgoingVideoQuality(edge, rate, kbps) else null
            if (quality == null || !quality.valid()) {
                error = invalidMessage
            } else {
                saving = true
                scope.launch {
                    val result = withContext(Dispatchers.IO) {
                        runCatching {
                            val current = PublisherClient.outgoing.value
                            check((if (current.active) current.id else 0L) == deliveryId) { "Screen delivery changed; reopen settings" }
                            OutgoingVideoQualityStore.save(context, deliveryId, quality)
                        }
                    }
                    saving = false
                    result.onSuccess { onDismiss() }.onFailure {
                        error = if (it.message == "Screen delivery changed; reopen settings") changedMessage else it.message ?: updateFailedMessage
                    }
                }
            }
        }) { Text(tr("Save")) } })
}
