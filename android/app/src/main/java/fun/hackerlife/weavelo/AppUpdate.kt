package `fun`.hackerlife.weavelo

import android.content.Context
import android.content.Intent
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.content.FileProvider
import androidx.core.content.pm.PackageInfoCompat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.File
import java.net.URL
import java.security.MessageDigest
import javax.net.ssl.HttpsURLConnection

internal object AppUpdate {
    private const val base = "https://weavelo.hackerlife.fun:8443/download/"
    private const val maxMetadata = 64 * 1024
    private const val maxPackage = 256L * 1024 * 1024
    private const val releaseCertificate =
        "215aedabf17f40a454ad381d9027772eef9446346cae3db031cac2a5ae51c3a5"
    private val packageFile = Regex("""Weavelo-Android-[A-Za-z0-9._-]{1,120}\.apk""")
    private val sha256 = Regex("[a-f0-9]{64}")
    private val downloadLock = Mutex()

    data class Release(
        val version: String,
        val code: Long,
        val file: String,
        val size: Long,
        val digest: String,
        val commit: String = "",
        val publishedAt: String = "",
        val notes: List<String> = emptyList()
    )

    suspend fun latest(): Release? = withContext(Dispatchers.IO) {
        val metadata = get("release.json") { connection ->
            connection.inputStream.buffered().use { input ->
                val bytes = ByteArray(maxMetadata + 1)
                var offset = 0
                while (offset < bytes.size) {
                    ensureActive()
                    val count = input.read(bytes, offset, bytes.size - offset)
                    if (count == -1) break
                    offset += count
                }
                check(offset <= maxMetadata) { "Update metadata is too large" }
                JSONObject(String(bytes, 0, offset, Charsets.UTF_8))
            }
        }
        check(metadata.getString("product") == "Weavelo" && metadata.getString("platform") == "android")
        val code = metadata.getLong("version_code")
        if (code <= BuildConfig.VERSION_CODE || metadata.getInt("min_sdk") > Build.VERSION.SDK_INT) return@withContext null
        val version = metadata.getString("version")
        val file = metadata.getString("file")
        val size = metadata.getLong("size_bytes")
        val digest = metadata.getString("sha256")
        val commit = metadata.optString("commit", "").trim().take(12)
        val publishedAt = metadata.optString("published_at", "").trim().take(32)
        val notes = metadata.optJSONArray("notes")?.let { items ->
            (0 until minOf(items.length(), 8))
                .mapNotNull { index -> items.optString(index).trim().takeIf(String::isNotEmpty) }
        } ?: emptyList()
        check(version.matches(Regex("[0-9][A-Za-z0-9.+-]{0,63}")) &&
            file.matches(packageFile) && size in 1..maxPackage && digest.matches(sha256))
        Release(version, code, file, size, digest, commit, publishedAt, notes)
    }

    suspend fun download(context: Context, release: Release): Intent = downloadLock.withLock {
        withContext(Dispatchers.IO) { downloadVerified(context, release) }
    }

    @Suppress("DEPRECATION") // The Android installer supplies the user confirmation UI.
    private suspend fun downloadVerified(context: Context, release: Release): Intent {
        val coroutine = kotlinx.coroutines.currentCoroutineContext()
        val directory = File(context.cacheDir, "app-update")
        check(directory.isDirectory || directory.mkdirs())
        val part = File(directory, "download.part.apk")
        try {
            val digest = MessageDigest.getInstance("SHA-256")
            get(release.file) { connection ->
                connection.inputStream.buffered().use { input ->
                    part.outputStream().buffered().use { output ->
                        val buffer = ByteArray(64 * 1024)
                        var remaining = release.size
                        while (true) {
                            coroutine.ensureActive()
                            val count = input.read(buffer)
                            if (count == -1) break
                            remaining -= count
                            check(remaining >= 0) { "Update is larger than the published package" }
                            output.write(buffer, 0, count)
                            digest.update(buffer, 0, count)
                        }
                        check(remaining == 0L) { "Update download is incomplete" }
                    }
                }
            }
            check(digest.digest().hex() == release.digest) { "Update checksum does not match" }
            verifyPackage(context, part, release)
            coroutine.ensureActive()
            val ready = File(directory, "Weavelo.apk")
            check(part.renameTo(ready))
            return Intent(Intent.ACTION_INSTALL_PACKAGE, FileProvider.getUriForFile(
                context, "${context.packageName}.files", ready
            )).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        } finally {
            part.delete()
        }
    }

    @Suppress("DEPRECATION")
    private fun verifyPackage(context: Context, file: File, release: Release) {
        val manager = context.packageManager
        val flags = if (Build.VERSION.SDK_INT >= 28) PackageManager.GET_SIGNING_CERTIFICATES else PackageManager.GET_SIGNATURES
        val installed = manager.getPackageInfo(context.packageName, flags)
        val archive = checkNotNull(manager.getPackageArchiveInfo(file.path, flags))
        check(archive.packageName == installed.packageName &&
            PackageInfoCompat.getLongVersionCode(archive) == release.code &&
            PackageInfoCompat.getLongVersionCode(archive) > PackageInfoCompat.getLongVersionCode(installed) &&
            archive.applicationInfo?.minSdkVersion?.let { it <= Build.VERSION.SDK_INT } == true &&
            certificate(installed) == releaseCertificate && certificate(archive) == releaseCertificate
        ) { "Update package identity is incompatible with this installation" }
    }

    @Suppress("DEPRECATION")
    private fun certificate(info: PackageInfo): String? {
        val signatures = if (Build.VERSION.SDK_INT >= 28) info.signingInfo?.apkContentsSigners else info.signatures
        return signatures?.singleOrNull()?.let { MessageDigest.getInstance("SHA-256").digest(it.toByteArray()).hex() }
    }

    private fun <T> get(file: String, read: (HttpsURLConnection) -> T): T {
        val connection = URL(base + file).openConnection() as HttpsURLConnection
        try {
            connection.instanceFollowRedirects = false
            connection.useCaches = false
            connection.connectTimeout = 10_000
            connection.readTimeout = 15_000
            connection.setRequestProperty("Cache-Control", "no-cache")
            check(connection.responseCode == 200) { "Update service returned HTTP ${connection.responseCode}" }
            return read(connection)
        } finally {
            connection.disconnect()
        }
    }

    private fun ByteArray.hex() = joinToString("") { "%02x".format(it) }
}
