package `fun`.hackerlife.weavelo.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import `fun`.hackerlife.weavelo.DeviceArtwork
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

@Composable
fun DeviceBadgeSettings() {
    val p = WeaveloTokens.colors
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var selected by remember { mutableStateOf(DeviceArtwork.savedJson()) }
    var expanded by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var errorText by remember { mutableStateOf<`fun`.hackerlife.weavelo.AppText?>(null) }
    val save: (Any) -> Unit = { value ->
        scope.launch {
            busy = true
            runCatching { withContext(Dispatchers.IO) { DeviceArtwork.save(value) } }
                .onSuccess { selected = value; error = null; errorText = null }
                .onFailure { error = it.message; errorText = `fun`.hackerlife.weavelo.appTextOf(it) }
            busy = false
        }
    }
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri ->
        if (uri != null) scope.launch {
            busy = true
            runCatching {
                withContext(Dispatchers.IO) {
                    DeviceArtwork.importImage(context, uri).also { DeviceArtwork.save(it) }
                }
            }.onSuccess { selected = it; error = null; errorText = null }
                .onFailure { error = it.message; errorText = `fun`.hackerlife.weavelo.appTextOf(it) }
            busy = false
        }
    }
    val appearance = remember(selected) {
        val badge = if (selected is String) JSONObject().put("preset", "phone") else selected
        DeviceArtwork.decode(JSONObject().put("device", badge))
    }
    val preview = remember(appearance) { DeviceArtwork.badge(appearance.preset, appearance.custom).asImageBitmap() }
    val activeKey = remember(selected) {
        when (val s = selected) {
            is String -> s
            is JSONObject -> s.optString("preset", "custom")
            else -> "automatic"
        }
    }
    PrecisionCard(verticalSpacing = 8.dp) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Image(preview, tr("Device badge"), Modifier.size(40.dp))
            Column(Modifier.weight(1f).padding(horizontal = 12.dp)) {
                Text(
                    android.os.Build.MODEL,
                    color = p.textPrimary,
                    fontSize = 14.sp,
                    fontWeight = FontWeight.SemiBold,
                )
                Text(
                    tr("Device badge"),
                    color = p.textSecondary,
                    fontSize = 12.sp,
                )
            }
            Box {
                PrecisionButton(
                    text = tr("Change"),
                    enabled = !busy,
                    style = PrecisionButtonStyle.TONAL,
                    compact = true,
                    onClick = { expanded = true },
                )
                DropdownMenu(
                    expanded = expanded,
                    onDismissRequest = { expanded = false },
                ) {
                    listOf(
                        "automatic" to "Automatic",
                        "phone" to "Android phone",
                        "mac_book" to "MacBook",
                        "mac_studio" to "Mac Studio",
                        "windows" to "Windows",
                        "linux" to "Linux",
                    ).forEach { (key, label) ->
                        PrecisionMenuItem(
                            text = tr(label),
                            symbol = MobileSymbol.Devices,
                            checked = activeKey == key,
                            onClick = {
                                expanded = false
                                save(if (key == "automatic") key else JSONObject().put("preset", key))
                            },
                        )
                    }
                    HorizontalDivider(
                        modifier = Modifier.padding(vertical = 4.dp),
                        color = p.borderMuted,
                    )
                    PrecisionMenuItem(
                        text = tr("Choose image…"),
                        symbol = MobileSymbol.Folder,
                        onClick = {
                            expanded = false
                            picker.launch("image/*")
                        },
                    )
                }
            }
        }
        error?.let {
            Text(
                errorText?.let { text -> appText(text) } ?: trFormat("Details: %s", it),
                color = p.dangerText,
                fontSize = 12.sp,
            )
        }
    }
}
