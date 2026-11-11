package `fun`.hackerlife.weavelo.ui

import android.content.Intent
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.Uri
import android.provider.Settings
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.repeatOnLifecycle
import `fun`.hackerlife.weavelo.AppUpdate
import `fun`.hackerlife.weavelo.BuildConfig
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch

internal class AppUpdateUi {
    var release by mutableStateOf<AppUpdate.Release?>(null)
    var status by mutableStateOf<String?>(null)
    var checking by mutableStateOf(false)
    var downloading by mutableStateOf(false)
    val checks = Channel<Unit>(Channel.CONFLATED)

    fun refresh() { checks.trySend(Unit) }
}

@Composable
internal fun rememberAppUpdate(): AppUpdateUi {
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    val network = LocalContext.current.getSystemService(ConnectivityManager::class.java)
    val update = remember { AppUpdateUi() }
    LaunchedEffect(lifecycle, network) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
            val callback = object : ConnectivityManager.NetworkCallback() {
                private var validated: Network? = null
                override fun onCapabilitiesChanged(network: Network, capabilities: NetworkCapabilities) {
                    if (capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)) {
                        if (validated != network) update.refresh()
                        validated = network
                    } else if (validated == network) validated = null
                }
                override fun onLost(network: Network) {
                    if (validated == network) validated = null
                }
            }
            network.registerDefaultNetworkCallback(callback)
            update.refresh()
            try {
                for (check in update.checks) {
                    if (update.downloading) continue
                    update.checking = true
                    try {
                        update.release = AppUpdate.latest()
                        update.status = if (update.release == null) "No newer compatible update" else null
                    } catch (cancelled: CancellationException) {
                        throw cancelled
                    } catch (_: Exception) {
                        update.status = "Could not check for updates. Try again when online."
                    } finally {
                        update.checking = false
                    }
                }
            } finally {
                network.unregisterNetworkCallback(callback)
            }
        }
    }
    return update
}

@Composable
internal fun AppUpdateCard(
    update: AppUpdateUi,
    installAllowed: Boolean,
    onPrepareInstall: () -> Unit = {}
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val stillAllowed by rememberUpdatedState(installAllowed)
    Column(
        Modifier.fillMaxWidth().background(ColorSurfaceCard, RoundedCornerShape(12.dp)).padding(14.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp)
    ) {
        Text(tr("App update"), color = ColorTextPrimary)
        Text(trFormat("Current version: v%s (Build %d)", BuildConfig.VERSION_NAME, BuildConfig.VERSION_CODE),
            color = ColorTextSecondary)
        update.release?.let { release ->
            Text(trFormat("Latest version: v%s (Build %d)", release.version, release.code), color = ColorAccentEmerald)
            val sizeMb = String.format(java.util.Locale.ROOT, "%.1f", release.size / (1024.0 * 1024.0))
            Text(
                if (release.commit.isNotEmpty()) trFormat("Package size: %s MB · %s", sizeMb, release.commit)
                else trFormat("Package size: %s MB", sizeMb),
                color = ColorTextSecondary
            )
            if (release.notes.isNotEmpty()) {
                Column(Modifier.padding(top = 2.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                    release.notes.forEach { note ->
                        Text("• $note", color = ColorTextSecondary)
                    }
                }
            }
            Text(
                tr("Signature and SHA-256 are verified before launch. If your phone prompts a security check, choose Continue installation."),
                color = ColorTextSecondary
            )
        }
        val status = when {
            update.downloading -> "Downloading update…"
            update.checking -> "Checking for updates…"
            else -> update.status
        }
        status?.let { Text(tr(it), color = ColorTextSecondary) }
        if (!installAllowed && update.release != null) {
            Text(tr("Finish active sharing or file transfers before installing."), color = ColorTextSecondary)
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            val idle = !update.checking && !update.downloading
            TextButton(onClick = update::refresh, enabled = idle) { Text(tr("Check for updates")) }
            if (update.release != null) {
                TextButton(enabled = idle && installAllowed, onClick = {
                    update.downloading = true
                    update.status = null
                    scope.launch {
                        try {
                            if (!context.packageManager.canRequestPackageInstalls()) {
                                update.status = "Allow Weavelo to install updates in system settings, then return here."
                                context.startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                                    Uri.fromParts("package", context.packageName, null)))
                                return@launch
                            }
                            // Recheck at download time in case a newer release was published while offline.
                            val latest = AppUpdate.latest()
                            update.release = latest
                            if (latest == null) {
                                update.status = "No newer compatible update"
                                return@launch
                            }
                            val installer = AppUpdate.download(context, latest)
                            if (stillAllowed) {
                                onPrepareInstall()
                                context.startActivity(installer)
                            } else {
                                update.status = "Finish active sharing or file transfers before installing."
                            }
                        } catch (cancelled: CancellationException) {
                            throw cancelled
                        } catch (_: Exception) {
                            update.status = "Could not verify or open the update. Try again."
                        } finally {
                            update.downloading = false
                        }
                    }
                }) { Text(tr("Download and install")) }
            }
        }
    }
}
