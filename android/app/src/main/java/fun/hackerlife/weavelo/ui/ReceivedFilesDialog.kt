package `fun`.hackerlife.weavelo.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.content.ContentValues
import android.content.Intent
import android.graphics.BitmapFactory
import android.os.Build
import android.os.Environment
import android.os.FileObserver
import android.provider.MediaStore
import android.webkit.MimeTypeMap
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.FileProvider
import `fun`.hackerlife.weavelo.ActiveTransferItem
import `fun`.hackerlife.weavelo.ClipboardHistoryEntry
import `fun`.hackerlife.weavelo.WorkspaceClient
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

internal fun formatHumanBytes(bytes: Long): String {
    if (bytes < 1024L) return "$bytes B"
    val kb = bytes / 1024.0
    if (kb < 1024.0) return String.format(Locale.ROOT, "%.1f KB", kb)
    val mb = kb / 1024.0
    if (mb < 1024.0) return String.format(Locale.ROOT, "%.1f MB", mb)
    val gb = mb / 1024.0
    return String.format(Locale.ROOT, "%.2f GB", gb)
}

internal fun formatShortTimestamp(epochMs: Long): String {
    if (epochMs <= 0L) return ""
    return runCatching {
        SimpleDateFormat("MM-dd HH:mm", Locale.getDefault()).format(Date(epochMs))
    }.getOrDefault("")
}

@Composable
fun ReceivedFilesDialog(
    onSendToDevice: ((File) -> Unit)? = null,
    sessionKey: String? = null,
    initialTab: String = "received",
    onPickSendFiles: (() -> Unit)? = null,
    onPickShareFiles: (() -> Unit)? = null,
    onClose: () -> Unit,
) {
    val p = WeaveloTokens.colors
    val context = LocalContext.current
    val workspaceState by WorkspaceClient.state.collectAsState()
    val inSession = sessionKey != null && workspaceState.sessionKey == sessionKey
    var activeTab by remember(initialTab, inSession) {
        mutableStateOf(if (inSession) initialTab else "received")
    }

    val root = remember { File(context.filesDir, "received").apply { mkdirs() }.canonicalFile }
    var directory by remember { mutableStateOf(root) }
    var files by remember { mutableStateOf(emptyList<File>()) }
    var searchQuery by remember { mutableStateOf("") }
    var sortMode by remember { mutableStateOf("newest") }
    var confirmDeletePath by remember { mutableStateOf<String?>(null) }
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

    fun deleteLocalFile(file: File) {
        scope.launch(Dispatchers.IO) {
            runCatching {
                if (file.canonicalPath.startsWith(root.path + File.separator)) {
                    file.deleteRecursively()
                }
            }.onSuccess {
                withContext(Dispatchers.Main) {
                    confirmDeletePath = null
                    refresh++
                }
            }
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

    LaunchedEffect(directory, refresh, workspaceState.activeTransfers.size) {
        files = withContext(Dispatchers.IO) {
            directory.listFiles()
                ?.filter { !it.name.startsWith(".weavelo-") && it.canonicalPath.startsWith(root.path + File.separator) }
                ?: emptyList()
        }
    }

    val filteredFiles = remember(files, searchQuery, sortMode) {
        val q = searchQuery.trim()
        files.filter { q.isEmpty() || it.name.contains(q, ignoreCase = true) }
            .sortedWith(
                compareBy<File> { !it.isDirectory }.let { base ->
                    when (sortMode) {
                        "size" -> base.thenByDescending { if (it.isFile) it.length() else 0L }.thenBy { it.name }
                        "name" -> base.thenBy { it.name.lowercase(Locale.getDefault()) }
                        else -> base.thenByDescending { it.lastModified() }.thenBy { it.name }
                    }
                }
            )
    }

    val totalReceivedBytes = remember(files) { files.filter { it.isFile }.sumOf { it.length() } }
    val runningTransfersCount = workspaceState.activeTransfers.count { it.isRunning }

    PrecisionDialog(
        title = if (inSession) tr("File transfer & manager") else tr("Received files"),
        icon = MobileSymbol.Files,
        maxWidth = 540.dp,
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
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            if (inSession) {
                PrecisionSegmentedRow(
                    options = listOf(
                        "transfers" to if (runningTransfersCount > 0) "${tr("Transfers")} ($runningTransfersCount)" else tr("Transfers"),
                        "received" to "${tr("Received")} (${files.count { it.isFile }})",
                        "remote" to tr("Remote shared"),
                    ),
                    selected = activeTab,
                    modifier = Modifier.fillMaxWidth(),
                    onSelect = { tab ->
                        activeTab = tab
                        if (tab == "remote" && workspaceState.connected && workspaceState.files && !workspaceState.sharedBusy) {
                            WorkspaceClient.listSharedFiles()
                        }
                    }
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

            when (activeTab) {
                "transfers" -> {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        onPickSendFiles?.let { pick ->
                            PrecisionButton(
                                text = tr("Send files"),
                                leadingSymbol = MobileSymbol.Send,
                                enabled = workspaceState.connected && workspaceState.files,
                                style = PrecisionButtonStyle.PRIMARY,
                                compact = true,
                                onClick = pick
                            )
                        }
                        onPickShareFiles?.let { sharePick ->
                            PrecisionButton(
                                text = tr("Publish files"),
                                leadingSymbol = MobileSymbol.Share,
                                enabled = workspaceState.connected,
                                style = PrecisionButtonStyle.TONAL,
                                compact = true,
                                onClick = sharePick
                            )
                        }
                        Spacer(Modifier.weight(1f))
                        if (workspaceState.activeTransfers.any { !it.isRunning }) {
                            PrecisionButton(
                                text = tr("Clear finished"),
                                style = PrecisionButtonStyle.SUBTLE,
                                compact = true,
                                onClick = { WorkspaceClient.clearFinishedTransfers() }
                            )
                        }
                    }

                    if (workspaceState.activeTransfers.isEmpty()) {
                        Box(
                            modifier = Modifier
                                .fillMaxWidth()
                                .clip(RoundedCornerShape(12.dp))
                                .background(p.surfaceSunken)
                                .border(BorderStroke(1.dp, p.borderMuted), RoundedCornerShape(12.dp))
                                .padding(16.dp),
                            contentAlignment = Alignment.Center,
                        ) {
                            Text(
                                tr("No active transfers. Select one or multiple files to send."),
                                color = p.textSecondary,
                                fontSize = 12.5.sp
                            )
                        }
                    } else {
                        LazyColumn(
                            Modifier.heightIn(max = 340.dp),
                            verticalArrangement = Arrangement.spacedBy(8.dp)
                        ) {
                            items(workspaceState.activeTransfers, key = { "${it.incoming}-${it.id}" }) { item ->
                                TransferQueueRow(item = item)
                            }
                        }
                    }
                }

                "remote" -> {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            PrecisionButton(
                                text = tr("Refresh"),
                                leadingSymbol = MobileSymbol.Refresh,
                                enabled = workspaceState.connected && !workspaceState.sharedBusy,
                                style = PrecisionButtonStyle.TONAL,
                                compact = true,
                                onClick = { WorkspaceClient.listSharedFiles() }
                            )
                            workspaceState.sharedNext?.let { next ->
                                PrecisionButton(
                                    text = tr("Next page"),
                                    enabled = workspaceState.connected && !workspaceState.sharedBusy,
                                    style = PrecisionButtonStyle.SUBTLE,
                                    compact = true,
                                    onClick = { WorkspaceClient.listSharedFiles(next) }
                                )
                            }
                        }
                        PrecisionButton(
                            text = tr("Clear my shared list"),
                            style = PrecisionButtonStyle.SUBTLE,
                            compact = true,
                            enabled = workspaceState.connected,
                            onClick = { WorkspaceClient.clearSharedFiles() }
                        )
                    }

                    if (workspaceState.remoteFiles.isEmpty()) {
                        Box(
                            modifier = Modifier
                                .fillMaxWidth()
                                .clip(RoundedCornerShape(12.dp))
                                .background(p.surfaceSunken)
                                .border(BorderStroke(1.dp, p.borderMuted), RoundedCornerShape(12.dp))
                                .padding(16.dp),
                            contentAlignment = Alignment.Center
                        ) {
                            Text(
                                if (workspaceState.sharedBusy) tr("Requesting shared files...")
                                else tr("No files explicitly shared by this device"),
                                color = p.textSecondary,
                                fontSize = 12.5.sp
                            )
                        }
                    } else {
                        LazyColumn(
                            Modifier.heightIn(max = 340.dp),
                            verticalArrangement = Arrangement.spacedBy(8.dp)
                        ) {
                            items(workspaceState.remoteFiles, key = { it.id }) { remoteFile ->
                                Row(
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .clip(RoundedCornerShape(12.dp))
                                        .background(p.surfaceSunken)
                                        .border(BorderStroke(1.dp, p.borderMuted), RoundedCornerShape(12.dp))
                                        .padding(horizontal = 12.dp, vertical = 10.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                    horizontalArrangement = Arrangement.spacedBy(10.dp)
                                ) {
                                    Icon(
                                        imageVector = MobileSymbol.Files.image,
                                        contentDescription = null,
                                        tint = p.brand,
                                        modifier = Modifier.size(18.dp)
                                    )
                                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                                        Text(
                                            text = remoteFile.name,
                                            color = p.textPrimary,
                                            fontSize = 13.sp,
                                            fontWeight = FontWeight.Medium,
                                            maxLines = 1,
                                            overflow = TextOverflow.Ellipsis
                                        )
                                        Text(
                                            text = formatHumanBytes(remoteFile.sizeBytes),
                                            color = p.textTertiary,
                                            fontSize = 11.sp,
                                            fontFamily = FontFamily.Monospace
                                        )
                                    }
                                    PrecisionButton(
                                        text = tr("Pull"),
                                        leadingSymbol = MobileSymbol.Download,
                                        enabled = workspaceState.connected && !workspaceState.sharedBusy && sessionKey != null,
                                        style = PrecisionButtonStyle.PRIMARY,
                                        compact = true,
                                        onClick = {
                                            if (sessionKey != null) {
                                                WorkspaceClient.fetchSharedFile(remoteFile.id, sessionKey)
                                                activeTab = "transfers"
                                            }
                                        }
                                    )
                                }
                            }
                        }
                    }
                }

                else -> {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        if (directory != root) {
                            PrecisionButton(
                                text = tr("Back to parent folder"),
                                leadingSymbol = MobileSymbol.Back,
                                style = PrecisionButtonStyle.TONAL,
                                compact = true,
                                onClick = { directory = directory.parentFile ?: root },
                            )
                        } else {
                            Text(
                                text = trFormat("%d files · %s", files.count { it.isFile }, formatHumanBytes(totalReceivedBytes)),
                                color = p.textSecondary,
                                fontSize = 11.5.sp,
                                fontFamily = FontFamily.Monospace
                            )
                        }
                        PrecisionSegmentedRow(
                            options = listOf(
                                "newest" to tr("Newest"),
                                "size" to tr("Size"),
                                "name" to tr("Name"),
                            ),
                            selected = sortMode,
                            onSelect = { sortMode = it }
                        )
                    }

                    if (files.size > 2 || searchQuery.isNotBlank()) {
                        PrecisionTextField(
                            value = searchQuery,
                            onValueChange = { searchQuery = it },
                            label = tr("Search received files")
                        )
                    }

                    if (filteredFiles.isEmpty()) {
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
                            Modifier.heightIn(max = 340.dp),
                            verticalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            items(filteredFiles, key = { it.absolutePath }) { file ->
                                val confirmingDelete = confirmDeletePath == file.absolutePath
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
                                        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                                            Text(
                                                text = if (file.isDirectory) trFormat("Folder · %s", file.name) else file.name,
                                                color = p.textPrimary,
                                                fontSize = 13.sp,
                                                fontWeight = FontWeight.Medium,
                                                maxLines = 1,
                                                overflow = TextOverflow.Ellipsis,
                                            )
                                            if (file.isFile) {
                                                Text(
                                                    text = "${formatHumanBytes(file.length())} · ${formatShortTimestamp(file.lastModified())}",
                                                    color = p.textTertiary,
                                                    fontSize = 11.sp,
                                                    fontFamily = FontFamily.Monospace
                                                )
                                            }
                                        }
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
                                                modifier = Modifier.size(34.dp),
                                            ) { open(file, true) }
                                            onSendToDevice?.let { send ->
                                                MobileAction(
                                                    symbol = MobileSymbol.Send,
                                                    label = tr("Send to device"),
                                                    tonalWell = true,
                                                    modifier = Modifier.size(34.dp),
                                                ) { send(file) }
                                            }
                                            Spacer(Modifier.weight(1f))
                                            if (confirmingDelete) {
                                                PrecisionButton(
                                                    text = tr("Confirm delete"),
                                                    style = PrecisionButtonStyle.DANGER,
                                                    compact = true,
                                                    onClick = { deleteLocalFile(file) }
                                                )
                                            } else {
                                                MobileAction(
                                                    symbol = MobileSymbol.Close,
                                                    label = tr("Delete"),
                                                    destructive = true,
                                                    modifier = Modifier.size(34.dp),
                                                ) { confirmDeletePath = file.absolutePath }
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
    }
}

@Composable
private fun TransferQueueRow(item: ActiveTransferItem) {
    val p = WeaveloTokens.colors
    val statusBadge = when (item.state) {
        "completed" -> tr("Completed") to PillTone.EMERALD
        "cancelled" -> tr("Cancelled") to PillTone.NEUTRAL
        else -> (if (item.incoming) tr("Receiving") else tr("Sending")) to PillTone.BRAND
    }
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .background(p.surfaceSunken)
            .border(BorderStroke(1.dp, p.borderMuted), RoundedCornerShape(12.dp))
            .padding(horizontal = 12.dp, vertical = 10.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp)
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            PrecisionPillBadge(text = statusBadge.first, tone = statusBadge.second)
            Text(
                text = item.name,
                color = p.textPrimary,
                fontSize = 13.sp,
                fontWeight = FontWeight.Medium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f)
            )
            if (item.isRunning) {
                PrecisionButton(
                    text = tr("Cancel"),
                    style = PrecisionButtonStyle.DANGER_TONAL,
                    compact = true,
                    onClick = {
                        if (item.groupId != null) WorkspaceClient.cancelTransferGroup(item.groupId)
                        else WorkspaceClient.cancelTransfer(item.id)
                    }
                )
            }
        }
        if (item.totalBytes > 0L || item.isRunning) {
            LinearProgressIndicator(
                progress = { item.progressFraction },
                modifier = Modifier.fillMaxWidth().height(5.dp).clip(RoundedCornerShape(4.dp)),
                color = if (item.state == "completed") p.emeraldDot else p.brand,
                trackColor = p.borderMuted
            )
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween
            ) {
                Text(
                    text = if (item.totalBytes > 0L) "${formatHumanBytes(item.doneBytes)} / ${formatHumanBytes(item.totalBytes)}"
                    else formatHumanBytes(item.doneBytes),
                    color = p.textTertiary,
                    fontSize = 11.sp,
                    fontFamily = FontFamily.Monospace
                )
                Text(
                    text = "${(item.progressFraction * 100).toInt()}%",
                    color = p.textSecondary,
                    fontSize = 11.sp,
                    fontFamily = FontFamily.Monospace,
                    fontWeight = FontWeight.SemiBold
                )
            }
        }
    }
}

@Composable
fun ClipboardCenterDialog(
    sessionKey: String,
    onSendPhoneClipboard: () -> Unit,
    onTypeIntoRemote: ((String) -> Unit)? = null,
    onClose: () -> Unit,
) {
    val p = WeaveloTokens.colors
    val context = LocalContext.current
    val state by WorkspaceClient.state.collectAsState()
    val connected = state.sessionKey == sessionKey && state.connected && state.clipboardAvailable
    var draftText by remember { mutableStateOf("") }
    var feedback by remember { mutableStateOf<String?>(null) }

    fun copyEntryToPhone(entry: ClipboardHistoryEntry) {
        val clipboard = context.getSystemService(ClipboardManager::class.java)
        runCatching {
            when {
                !entry.text.isNullOrEmpty() -> {
                    clipboard.setPrimaryClip(ClipData.newPlainText("Weavelo", entry.text))
                    feedback = tr("Copied to phone clipboard")
                }
                !entry.imagePath.isNullOrEmpty() -> {
                    val file = File(entry.imagePath)
                    if (file.exists()) {
                        val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
                        clipboard.setPrimaryClip(ClipData.newUri(context.contentResolver, "Weavelo image", uri))
                        feedback = tr("Copied image to phone clipboard")
                    }
                }
            }
        }
    }

    PrecisionDialog(
        title = tr("Clipboard manager"),
        icon = MobileSymbol.Clipboard,
        maxWidth = 540.dp,
        onDismissRequest = onClose,
        confirmButton = {
            PrecisionButton(
                text = tr("Close"),
                onClick = onClose,
                style = PrecisionButtonStyle.TONAL,
                compact = true
            )
        }
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            PrecisionSwitchRow(
                title = tr("Bidirectional Clipboard Sync"),
                subtitle = tr("Automatically sync text, images, and copied files with the remote device"),
                checked = state.clipboard,
                onCheckedChange = { enabled ->
                    if (connected) WorkspaceClient.enableClipboard(enabled)
                },
                leadingSymbol = MobileSymbol.Clipboard
            )

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                PrecisionButton(
                    text = tr("Send phone clipboard"),
                    leadingSymbol = MobileSymbol.Send,
                    enabled = connected && state.clipboard,
                    style = PrecisionButtonStyle.PRIMARY,
                    compact = true,
                    onClick = {
                        onSendPhoneClipboard()
                        feedback = tr("Phone clipboard sent to remote")
                    }
                )
                Spacer(Modifier.weight(1f))
                if (state.clipboardHistory.any { !it.pinned }) {
                    PrecisionButton(
                        text = tr("Clear history"),
                        style = PrecisionButtonStyle.SUBTLE,
                        compact = true,
                        onClick = { WorkspaceClient.clearClipboardHistory() }
                    )
                }
            }

            // Quick snippet composer: push to remote clipboard or type directly into active remote editor
            PrecisionCard(sunken = true, contentPadding = PaddingValues(10.dp)) {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    PrecisionTextField(
                        value = draftText,
                        onValueChange = { draftText = it },
                        label = tr("Quick text or snippet for remote")
                    )
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        PrecisionButton(
                            text = tr("Sync to remote"),
                            leadingSymbol = MobileSymbol.Copy,
                            enabled = connected && draftText.isNotBlank(),
                            style = PrecisionButtonStyle.PRIMARY,
                            compact = true,
                            onClick = {
                                if (!state.clipboard) WorkspaceClient.enableClipboard(true)
                                WorkspaceClient.sendClipboard(JSONObject().put("text", draftText))
                                feedback = tr("Synced to remote clipboard")
                                draftText = ""
                            }
                        )
                        if (onTypeIntoRemote != null) {
                            PrecisionButton(
                                text = tr("Type into remote"),
                                leadingSymbol = MobileSymbol.Keyboard,
                                enabled = draftText.isNotBlank(),
                                style = PrecisionButtonStyle.TONAL,
                                compact = true,
                                onClick = {
                                    onTypeIntoRemote(draftText)
                                    feedback = tr("Typed into remote cursor")
                                    draftText = ""
                                }
                            )
                        }
                    }
                }
            }

            feedback?.let { msg ->
                Text(msg, color = p.emeraldText, fontSize = 11.5.sp, fontWeight = FontWeight.Medium)
            }

            if (state.clipboardHistory.isEmpty()) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(12.dp))
                        .background(p.surfaceSunken)
                        .border(BorderStroke(1.dp, p.borderMuted), RoundedCornerShape(12.dp))
                        .padding(14.dp),
                    contentAlignment = Alignment.Center
                ) {
                    Text(
                        tr("No clipboard items synced yet in this session"),
                        color = p.textSecondary,
                        fontSize = 12.5.sp
                    )
                }
            } else {
                LazyColumn(
                    Modifier.heightIn(max = 260.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    items(state.clipboardHistory, key = { it.id }) { entry ->
                        Column(
                            modifier = Modifier
                                .fillMaxWidth()
                                .clip(RoundedCornerShape(12.dp))
                                .background(p.surfaceSunken)
                                .border(
                                    BorderStroke(1.dp, if (entry.pinned) p.brandBorder else p.borderMuted),
                                    RoundedCornerShape(12.dp)
                                )
                                .padding(10.dp),
                            verticalArrangement = Arrangement.spacedBy(6.dp)
                        ) {
                            Row(
                                modifier = Modifier.fillMaxWidth(),
                                horizontalArrangement = Arrangement.spacedBy(6.dp),
                                verticalAlignment = Alignment.CenterVertically
                            ) {
                                if (entry.pinned) {
                                    PrecisionPillBadge(text = tr("Pinned"), tone = PillTone.BRAND)
                                }
                                PrecisionPillBadge(
                                    text = if (entry.fromRemote) tr("From remote") else tr("From phone"),
                                    tone = if (entry.fromRemote) PillTone.CYAN else PillTone.NEUTRAL
                                )
                                Spacer(Modifier.weight(1f))
                                Text(
                                    text = formatShortTimestamp(entry.timestampMs),
                                    color = p.textTertiary,
                                    fontSize = 10.5.sp,
                                    fontFamily = FontFamily.Monospace
                                )
                            }

                            if (!entry.text.isNullOrBlank()) {
                                Text(
                                    text = entry.text,
                                    color = p.textPrimary,
                                    fontSize = 12.5.sp,
                                    maxLines = 3,
                                    overflow = TextOverflow.Ellipsis
                                )
                            } else if (!entry.imagePath.isNullOrBlank()) {
                                val thumb = remember(entry.imagePath) {
                                    runCatching {
                                        val opts = BitmapFactory.Options().apply { inSampleSize = 4 }
                                        BitmapFactory.decodeFile(entry.imagePath, opts)?.asImageBitmap()
                                    }.getOrNull()
                                }
                                if (thumb != null) {
                                    Image(
                                        bitmap = thumb,
                                        contentDescription = tr("Clipboard image"),
                                        modifier = Modifier
                                            .heightIn(max = 88.dp)
                                            .clip(RoundedCornerShape(8.dp))
                                    )
                                } else {
                                    Text(tr("Clipboard image"), color = p.textSecondary, fontSize = 12.sp)
                                }
                            } else if (entry.filePaths.isNotEmpty()) {
                                Text(
                                    text = trFormat("Clipboard files (%d)", entry.filePaths.size),
                                    color = p.textPrimary,
                                    fontSize = 12.5.sp
                                )
                            }

                            Row(
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(6.dp)
                            ) {
                                PrecisionButton(
                                    text = tr("Copy"),
                                    leadingSymbol = MobileSymbol.Copy,
                                    style = PrecisionButtonStyle.TONAL,
                                    compact = true,
                                    onClick = { copyEntryToPhone(entry) }
                                )
                                if (connected && (!entry.text.isNullOrBlank() || !entry.imagePath.isNullOrBlank())) {
                                    PrecisionButton(
                                        text = tr("Resend"),
                                        leadingSymbol = MobileSymbol.Send,
                                        style = PrecisionButtonStyle.SUBTLE,
                                        compact = true,
                                        onClick = {
                                            if (!state.clipboard) WorkspaceClient.enableClipboard(true)
                                            if (!entry.text.isNullOrBlank()) {
                                                WorkspaceClient.sendClipboard(JSONObject().put("text", entry.text))
                                            } else if (!entry.imagePath.isNullOrBlank()) {
                                                WorkspaceClient.sendClipboard(JSONObject().put("image", entry.imagePath).put("mime", "image/png"))
                                            }
                                            feedback = tr("Synced to remote clipboard")
                                        }
                                    )
                                }
                                if (!entry.text.isNullOrBlank() && onTypeIntoRemote != null) {
                                    MobileAction(
                                        symbol = MobileSymbol.Keyboard,
                                        label = tr("Type into remote"),
                                        tonalWell = true,
                                        modifier = Modifier.size(32.dp)
                                    ) {
                                        onTypeIntoRemote(entry.text)
                                        feedback = tr("Typed into remote cursor")
                                    }
                                }
                                Spacer(Modifier.weight(1f))
                                if (!entry.text.isNullOrBlank()) {
                                    PrecisionButton(
                                        text = if (entry.pinned) tr("Unpin") else tr("Pin"),
                                        style = PrecisionButtonStyle.SUBTLE,
                                        compact = true,
                                        onClick = { WorkspaceClient.togglePinClipboardEntry(entry.id) }
                                    )
                                }
                                MobileAction(
                                    symbol = MobileSymbol.Close,
                                    label = tr("Delete"),
                                    destructive = true,
                                    modifier = Modifier.size(32.dp)
                                ) { WorkspaceClient.removeClipboardEntry(entry.id) }
                            }
                        }
                    }
                }
            }
        }
    }
}
