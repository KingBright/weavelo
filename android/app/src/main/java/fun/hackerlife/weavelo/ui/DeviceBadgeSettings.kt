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
import androidx.compose.ui.unit.dp
import `fun`.hackerlife.weavelo.DeviceArtwork
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

@Composable
fun DeviceBadgeSettings() {
    val context=LocalContext.current
    val scope=rememberCoroutineScope()
    var selected by remember { mutableStateOf(DeviceArtwork.savedJson()) }
    var expanded by remember { mutableStateOf(false) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val save:(Any)->Unit = { value ->
        scope.launch {
            busy=true
            runCatching { withContext(Dispatchers.IO) { DeviceArtwork.save(value) } }
                .onSuccess { selected=value;error=null }.onFailure { error=it.message }
            busy=false
        }
    }
    val picker=rememberLauncherForActivityResult(ActivityResultContracts.GetContent()) { uri ->
        if (uri != null) scope.launch {
            busy=true
            runCatching { withContext(Dispatchers.IO) {
                DeviceArtwork.importImage(context,uri).also {DeviceArtwork.save(it)}
            } }.onSuccess { selected=it;error=null }.onFailure { error=it.message }
            busy=false
        }
    }
    val appearance=remember(selected) {
        val badge=if(selected is String) JSONObject().put("preset","phone") else selected
        DeviceArtwork.decode(JSONObject().put("device",badge))
    }
    val preview=remember(appearance) {DeviceArtwork.badge(appearance.preset,appearance.custom).asImageBitmap()}
    Column(verticalArrangement=Arrangement.spacedBy(6.dp)) {
        Row(verticalAlignment=Alignment.CenterVertically,modifier=Modifier.fillMaxWidth()) {
            Image(preview,"Device badge",Modifier.size(40.dp))
            Column(Modifier.weight(1f).padding(horizontal=10.dp)) {
                Text(android.os.Build.MODEL,color=ColorTextPrimary)
                Text("Device badge",style=MaterialTheme.typography.labelMedium,color=ColorTextSecondary)
            }
            Box {
                TextButton(enabled=!busy,onClick={expanded=true}) {Text("Change")}
                DropdownMenu(expanded=expanded,onDismissRequest={expanded=false}) {
                    listOf("automatic" to "Automatic", "phone" to "Android phone", "mac_book" to "MacBook",
                        "mac_studio" to "Mac Studio", "windows" to "Windows", "linux" to "Linux").forEach { (key,label) ->
                        DropdownMenuItem(text={Text(label)},onClick={expanded=false;save(if(key=="automatic") key else JSONObject().put("preset",key))})
                    }
                    DropdownMenuItem(text={Text("Choose image…")},onClick={expanded=false;picker.launch("image/*")})
                }
            }
        }
        error?.let {Text(it,color=MaterialTheme.colorScheme.error)}
    }
}
