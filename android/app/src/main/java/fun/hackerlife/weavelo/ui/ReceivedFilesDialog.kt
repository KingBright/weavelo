package `fun`.hackerlife.weavelo.ui

import android.content.ContentValues
import android.content.Intent
import android.os.Build
import android.os.Environment
import android.os.FileObserver
import android.provider.MediaStore
import android.webkit.MimeTypeMap
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.FileProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

@Composable
fun ReceivedFilesDialog(onSendToDevice: ((File) -> Unit)? = null, onClose: () -> Unit) {
    val p = WeaveloTokens.colors
    val context = LocalContext.current
    val root = remember { File(context.filesDir, "received").apply { mkdirs() }.canonicalFile }
    var directory by remember { mutableStateOf(root) }
    var files by remember { mutableStateOf(emptyList<File>()) }
    var error by remember { mutableStateOf<String?>(null) }
    var errorText by remember { mutableStateOf<`fun`.hackerlife.weavelo.AppText?>(null) }
    var savedNotice by remember { mutableStateOf<String?>(null) }
    val shareLabel = tr("Share received file")
    val openLabel = tr("Open received file")
    var refresh by remember { mutableIntStateOf(0) }
    var exporting by remember { mutableStateOf<File?>(null) }
    val scope = rememberCoroutineScope()
    val save = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("application/octet-stream")) { uri ->
        val source = exporting
        exporting = null
        if (uri != null && source != null) scope.launch(Dispatchers.IO) {
            runCatching {
                source.inputStream().use { input ->
                    checkNotNull(context.contentResolver.openOutputStream(uri, "w")).use { input.copyTo(it, 64 * 1024) }
                }
            }.onSuccess {
                withContext(Dispatchers.Main) { error = null; errorText = null; savedNotice = source.name }
            }.onFailure {
                withContext(Dispatchers.Main) {
                    error = it.message ?: "File could not be saved"
                    errorText = if (it.message == null) `fun`.hackerlife.weavelo.AppText("File could not be saved") else null
                }
            }
        }
    }
    fun saveToSystemDownloads(file: File) {
        if (Build.VERSION.SDK_INT < 29) {
            exporting = file
            save.launch(file.name)
            return
        }
        scope.launch(Dispatchers.IO) {
            runCatching {
                val ext = file.extension.lowercase()
                val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext) ?: "application/octet-stream"
                val values = ContentValues().apply {
                    put(MediaStore.Downloads.DISPLAY_NAME, file.name)
                    put(MediaStore.Downloads.MIME_TYPE, mime)
                    put(MediaStore.Downloads.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS + "/Weavelo")
                    put(MediaStore.Downloads.IS_PENDING, 1)
                }
                val resolver = context.contentResolver
                val uri = checkNotNull(resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)) { "File could not be saved" }
                try {
                    file.inputStream().use { input ->
                        checkNotNull(resolver.openOutputStream(uri, "w")).use { output -> input.copyTo(output, 64 * 1024) }
                    }
                    resolver.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
                } catch (failure: Throwable) {
                    resolver.delete(uri, null, null)
                    throw failure
                }
            }.onSuccess {
                withContext(Dispatchers.Main) { error = null; errorText = null; savedNotice = file.name }
            }.onFailure {
                withContext(Dispatchers.Main) {
                    error = it.message ?: "File could not be saved"
                    errorText = if (it.message == null) `fun`.hackerlife.weavelo.AppText("File could not be saved") else null
                }
            }
        }
    }
    fun open(file: File, share: Boolean) {
        runCatching {
            val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
            val type = context.contentResolver.getType(uri) ?: "application/octet-stream"
            val intent = if (share) Intent(Intent.ACTION_SEND).setType(type).putExtra(Intent.EXTRA_STREAM, uri)
            else Intent(Intent.ACTION_VIEW).setDataAndType(uri, type)
            intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            context.startActivity(Intent.createChooser(intent, if (share) shareLabel else openLabel))
        }.onFailure {
            error = it.message ?: "No application can open this file"
            errorText = if (it.message == null) `fun`.hackerlife.weavelo.AppText("No application can open this file") else null
        }
    }
    DisposableEffect(directory) {
        val observer = object : FileObserver(
            directory.path,
            FileObserver.CLOSE_WRITE or FileObserver.MOVED_TO or FileObserver.MOVED_FROM or
                FileObserver.CREATE or FileObserver.DELETE,
        ) {
            override fun onEvent(event: Int, path: String?) {
                if (path == null || !path.startsWith(".weavelo-")) scope.launch { refresh++ }
            }
        }
        observer.startWatching()
        onDispose { observer.stopWatching() }
    }
    LaunchedEffect(directory, refresh) {
        files = withContext(Dispatchers.IO) {
            directory.listFiles()
                ?.filter { !it.name.startsWith(".weavelo-") && it.canonicalPath.startsWith(root.path + File.separator) }
                ?.sortedWith(compareBy<File> { !it.isDirectory }.thenBy { it.name })
                ?: emptyList()
        }
    }

    PrecisionDialog(
        title = tr("Received files"),
        onDismissRequest = onClose,
        confirmButton = {
            PrecisionButton(
                text = tr("Close"),
                onClick = onClose,
                style = PrecisionButtonStyle.TONAL,
                compact = true,
            )
        },
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (directory != root) {
                PrecisionButton(
                    text = tr("Back to parent folder"),
                    leadingSymbol = MobileSymbol.Back,
                    style = PrecisionButtonStyle.TONAL,
                    compact = true,
                    onClick = { directory = directory.parentFile ?: root },
                )
            }
            savedNotice?.let {
                Text(
                    trFormat("Saved to Downloads: %s", it),
                    color = p.emeraldText,
                    fontSize = 12.sp,
                    fontWeight = FontWeight.Medium,
                )
            }
            error?.let {
                Text(
                    errorText?.let { text -> appText(text) } ?: trFormat("Details: %s", it),
                    color = p.dangerText,
                    fontSize = 12.sp,
                )
            }
            if (files.isEmpty()) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(12.dp))
                        .background(p.surfaceSunken)
                        .border(BorderStroke(1.dp, p.borderMuted), RoundedCornerShape(12.dp))
                        .padding(16.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(tr("No completed files in this folder"), color = p.textSecondary, fontSize = 13.sp)
                }
            } else {
                LazyColumn(
                    Modifier.heightIn(max = 380.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    items(files, key = { it.absolutePath }) { file ->
                        Column(
                            modifier = Modifier
                                .fillMaxWidth()
                                .clip(RoundedCornerShape(12.dp))
                                .background(p.surfaceSunken)
                                .border(BorderStroke(1.dp, p.borderMuted), RoundedCornerShape(12.dp))
                                .padding(10.dp),
                            verticalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Row(
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clip(RoundedCornerShape(8.dp))
                                    .clickable { if (file.isDirectory) directory = file else open(file, false) }
                                    .padding(vertical = 2.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(10.dp),
                            ) {
                                Icon(
                                    imageVector = (if (file.isDirectory) MobileSymbol.Folder else MobileSymbol.Files).image,
                                    contentDescription = null,
                                    tint = p.brand,
                                    modifier = Modifier.size(18.dp),
                                )
                                Text(
                                    text = if (file.isDirectory) trFormat("Folder · %s", file.name) else file.name,
                                    color = p.textPrimary,
                                    fontSize = 13.sp,
                                    fontWeight = FontWeight.Medium,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                    modifier = Modifier.weight(1f),
                                )
                                Icon(
                                    imageVector = MobileSymbol.ChevronRight.image,
                                    contentDescription = null,
                                    tint = p.textTertiary,
                                    modifier = Modifier.size(15.dp),
                                )
                            }
                            if (file.isFile) {
                                Row(
                                    verticalAlignment = Alignment.CenterVertically,
                                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                                ) {
                                    PrecisionButton(
                                        text = tr("Save to Downloads"),
                                        leadingSymbol = MobileSymbol.Download,
                                        style = PrecisionButtonStyle.PRIMARY,
                                        compact = true,
                                        onClick = { saveToSystemDownloads(file) },
                                    )
                                    PrecisionButton(
                                        text = tr("Save a copy"),
                                        style = PrecisionButtonStyle.TONAL,
                                        compact = true,
                                        onClick = { exporting = file; save.launch(file.name) },
                                    )
                                    MobileAction(
                                        symbol = MobileSymbol.Share,
                                        label = tr("Share"),
                                        tonalWell = true,
                                        modifier = Modifier.size(36.dp),
                                    ) { open(file, true) }
                                    onSendToDevice?.let { send ->
                                        MobileAction(
                                            symbol = MobileSymbol.Send,
                                            label = tr("Send to device"),
                                            tonalWell = true,
                                            modifier = Modifier.size(36.dp),
                                        ) { send(file) }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
