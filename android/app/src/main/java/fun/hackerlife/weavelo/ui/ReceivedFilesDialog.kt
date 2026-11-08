package `fun`.hackerlife.weavelo.ui

import android.content.Intent
import android.os.FileObserver
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

@Composable
fun ReceivedFilesDialog(onSendToDevice: ((File) -> Unit)? = null, onClose: () -> Unit) {
    val context = LocalContext.current
    val root = remember { File(context.filesDir, "received").apply { mkdirs() }.canonicalFile }
    var directory by remember { mutableStateOf(root) }
    var files by remember { mutableStateOf(emptyList<File>()) }
    var error by remember { mutableStateOf<String?>(null) }
    var errorText by remember { mutableStateOf<`fun`.hackerlife.weavelo.AppText?>(null) }
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
                source.inputStream().use { input -> checkNotNull(context.contentResolver.openOutputStream(uri, "w")).use { input.copyTo(it, 64 * 1024) } }
            }.onFailure { withContext(Dispatchers.Main) { error = it.message ?: "File could not be saved"; errorText = if (it.message == null) `fun`.hackerlife.weavelo.AppText("File could not be saved") else null } }
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
        }.onFailure { error = it.message ?: "No application can open this file"; errorText = if (it.message == null) `fun`.hackerlife.weavelo.AppText("No application can open this file") else null }
    }
    DisposableEffect(directory) {
        val observer = object : FileObserver(directory.path,
            FileObserver.CLOSE_WRITE or FileObserver.MOVED_TO or FileObserver.MOVED_FROM or
                FileObserver.CREATE or FileObserver.DELETE) {
            override fun onEvent(event: Int, path: String?) {
                if (path == null || !path.startsWith(".weavelo-")) scope.launch { refresh++ }
            }
        }
        observer.startWatching()
        onDispose { observer.stopWatching() }
    }
    LaunchedEffect(directory, refresh) {
        files = withContext(Dispatchers.IO) { directory.listFiles()?.filter { !it.name.startsWith(".weavelo-") && it.canonicalPath.startsWith(root.path + File.separator) }?.sortedWith(compareBy<File> { !it.isDirectory }.thenBy { it.name }) ?: emptyList() }
    }
    AlertDialog(onDismissRequest = onClose, title = { Text(tr("Received files")) }, text = {
        Column {
            if (directory != root) TextButton(onClick = { directory = directory.parentFile ?: root }) { Text(tr("Back to parent folder")) }
            error?.let { Text(errorText?.let { text -> appText(text) } ?: trFormat("Details: %s", it)) }
            LazyColumn(Modifier.heightIn(max = 420.dp)) {
                items(files, key = { it.absolutePath }) { file ->
                    Column {
                        TextButton(onClick = { if (file.isDirectory) directory = file else open(file, false) }) { Text(if (file.isDirectory) trFormat("Folder · %s", file.name) else file.name) }
                        if (file.isFile) Row {
                            TextButton(onClick = { exporting = file; save.launch(file.name) }) { Text(tr("Save a copy")) }
                            TextButton(onClick = { open(file, true) }) { Text(tr("Share")) }
                            onSendToDevice?.let { send ->
                                MobileAction(MobileSymbol.Send, tr("Send to device"), modifier = Modifier.size(44.dp)) { send(file) }
                            }
                        }
                    }
                }
            }
            if (files.isEmpty()) Text(tr("No completed files in this folder"))
        }
    }, confirmButton = { TextButton(onClick = onClose) { Text(tr("Close")) } })
}
