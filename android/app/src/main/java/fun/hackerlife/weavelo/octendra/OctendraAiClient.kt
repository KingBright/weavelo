package `fun`.hackerlife.weavelo.octendra

import android.content.ComponentName
import android.content.Context
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.view.Surface
import org.json.JSONArray
import org.json.JSONObject
import java.security.MessageDigest
import java.util.Locale

internal data class AiViewDescriptor(
    val viewId: String,
    val displayId: Int,
    val generation: String,
    val sourceRevision: Long,
    val resourceEpoch: Long,
    val packageName: String,
    val label: String,
    val userId: Int,
    val width: Int,
    val height: Int,
    val density: Int,
    val rotation: Int,
    val humanActive: Boolean,
    val supportsInput: Boolean,
)

internal data class AttachResult(
    val ok: Boolean,
    val consumerId: String = "",
    val viewId: String = "",
    val displayId: Int = -1,
    val generation: String = "",
    val sourceRevision: Long = 0L,
    val resourceEpoch: Long = 0L,
    val width: Int = 0,
    val height: Int = 0,
    val rotation: Int = 0,
    val error: String? = null,
)

internal data class DispatchInputResult(
    val ok: Boolean,
    val accepted: Boolean = false,
    val resourceEpoch: Long = 0L,
    val error: String? = null,
    val currentSourceRevision: Long? = null,
)

internal fun sha256Hex(bytes: ByteArray): String {
    val digest = MessageDigest.getInstance("SHA-256").digest(bytes)
    val sb = StringBuilder(digest.size * 2)
    for (byte in digest) {
        sb.append(String.format(Locale.ROOT, "%02x", byte.toInt() and 0xFF))
    }
    return sb.toString()
}

internal fun normalizeRotationQuadrant(raw: Int): Int = when (raw) {
    0, 90, 180, 270 -> (raw / 90) and 3
    1, 2, 3 -> raw
    else -> raw and 3
}

internal fun isTrustedOctendraPeer(
    providerPackage: String?,
    peerCertSha256s: List<String>,
    ownCertSha256s: Set<String>,
): Boolean {
    if (providerPackage != OctendraAiClient.OCTENDRA_PACKAGE) return false
    if (peerCertSha256s.isEmpty()) return false
    val allowed = HashSet<String>(ownCertSha256s.size + 1).apply {
        add(OctendraAiClient.OWNER_CERT_SHA256)
        for (cert in ownCertSha256s) {
            val normalized = cert.trim().lowercase(Locale.ROOT)
            if (normalized.length == 64) add(normalized)
        }
    }
    return peerCertSha256s.all { cert ->
        val normalized = cert.trim().lowercase(Locale.ROOT)
        normalized.length == 64 && normalized in allowed
    }
}

/**
 * Signature-verified ContentProvider client for Octendra AI views (`fun.hackerlife.octendra.aiview`).
 * Gracefully degrades to empty/unavailable if Octendra is absent, disabled, or untrusted.
 */
internal class OctendraAiClient(
    private val transport: Transport,
    private val logger: (String, Throwable?) -> Unit = { msg, err ->
        android.util.Log.w("WeaveloOctendra", msg, err)
    },
) {
    companion object {
        const val OCTENDRA_PACKAGE = "fun.hackerlife.octendra"
        const val AI_VIEW_AUTHORITY = "fun.hackerlife.octendra.aiview"
        const val OWNER_CERT_SHA256 = "f84d9b71f1a00f4b5e3084420e5cee3327053389983b2428d9bec71755e5539d"

        const val METHOD_DESCRIBE_VIEWS = "describe_views"
        const val METHOD_ATTACH_VIDEO = "attach_video"
        const val METHOD_DETACH_VIDEO = "detach_video"
        const val METHOD_DISPATCH_INPUT = "dispatch_input"
        const val METHOD_RELEASE_INPUT = "release_input"

        fun create(context: Context): OctendraAiClient =
            OctendraAiClient(AndroidTransport(context.applicationContext))
    }

    interface Transport {
        fun isAvailable(): Boolean
        fun describeViews(): String?
        fun attachVideo(viewId: String, generation: String, surface: Any): String?
        fun detachVideo(consumerId: String): String?
        fun dispatchInput(
            consumerId: String,
            viewId: String,
            generation: String,
            sourceRevision: Long,
            eventJson: String,
        ): String?

        fun releaseInput(consumerId: String): String?
    }

    fun isAvailable(): Boolean = runCatching { transport.isAvailable() }.getOrDefault(false)

    fun describeViews(): List<AiViewDescriptor> {
        val raw = try {
            transport.describeViews() ?: return emptyList()
        } catch (error: Throwable) {
            logger("Octendra describe_views unavailable", error)
            return emptyList()
        }
        return runCatching {
            val array = JSONArray(raw)
            val result = ArrayList<AiViewDescriptor>(array.length())
            val seenDisplays = HashSet<Int>(array.length())
            for (index in 0 until array.length()) {
                val obj = array.optJSONObject(index) ?: continue
                val viewId = obj.optString("view_id", "").trim()
                val displayId = obj.optInt("display_id", -1)
                val generation = obj.optString("generation", "").trim()
                val width = obj.optInt("width", 0)
                val height = obj.optInt("height", 0)
                if (viewId.isEmpty() || displayId < 0 || generation.isEmpty()) continue
                if (width !in 2..8192 || height !in 2..8192) continue
                if (!seenDisplays.add(displayId)) continue
                result.add(
                    AiViewDescriptor(
                        viewId = viewId,
                        displayId = displayId,
                        generation = generation,
                        sourceRevision = obj.optLong("source_revision", 0L),
                        resourceEpoch = obj.optLong("resource_epoch", 0L),
                        packageName = obj.optString("package", "").trim(),
                        label = obj.optString("label", "").trim(),
                        userId = obj.optInt("user_id", 0),
                        width = width,
                        height = height,
                        density = obj.optInt("density", 440).coerceIn(72, 1000),
                        rotation = normalizeRotationQuadrant(obj.optInt("rotation", 0)),
                        humanActive = obj.optBoolean("human_active", false),
                        supportsInput = obj.optBoolean("supports_input", false),
                    )
                )
            }
            result
        }.getOrElse { error ->
            logger("Malformed Octendra describe_views payload", error)
            emptyList()
        }
    }

    fun attachVideo(viewId: String, generation: String, surface: Any): AttachResult {
        if (viewId.isBlank() || generation.isBlank()) {
            return AttachResult(ok = false, error = "invalid_attach_arguments")
        }
        val raw = try {
            transport.attachVideo(viewId, generation, surface)
                ?: return AttachResult(ok = false, error = "octendra_unavailable")
        } catch (error: Throwable) {
            logger("Octendra attach_video failed", error)
            return AttachResult(ok = false, error = error.message ?: "attach_video_failed")
        }
        return runCatching {
            val json = JSONObject(raw)
            val ok = json.optBoolean("ok", false)
            if (!ok) {
                return@runCatching AttachResult(
                    ok = false,
                    error = json.optString("error", "attach_video_rejected").ifBlank { "attach_video_rejected" },
                )
            }
            val consumerId = json.optString("consumer_id", "").trim()
            val resolvedViewId = json.optString("view_id", viewId).trim()
            val displayId = json.optInt("display_id", -1)
            val resolvedGeneration = json.optString("generation", generation).trim()
            val width = json.optInt("width", 0)
            val height = json.optInt("height", 0)
            if (consumerId.isEmpty() || resolvedViewId.isEmpty() || displayId < 0 ||
                resolvedGeneration.isEmpty() || width !in 2..8192 || height !in 2..8192
            ) {
                return@runCatching AttachResult(ok = false, error = "invalid_attach_response")
            }
            AttachResult(
                ok = true,
                consumerId = consumerId,
                viewId = resolvedViewId,
                displayId = displayId,
                generation = resolvedGeneration,
                sourceRevision = json.optLong("source_revision", 0L),
                resourceEpoch = json.optLong("resource_epoch", 0L),
                width = width,
                height = height,
                rotation = normalizeRotationQuadrant(json.optInt("rotation", 0)),
            )
        }.getOrElse { error ->
            logger("Malformed Octendra attach_video response", error)
            AttachResult(ok = false, error = "malformed_attach_response")
        }
    }

    fun detachVideo(consumerId: String): Boolean {
        if (consumerId.isBlank()) return false
        val raw = try {
            transport.detachVideo(consumerId) ?: return false
        } catch (error: Throwable) {
            logger("Octendra detach_video failed", error)
            return false
        }
        return runCatching { JSONObject(raw).optBoolean("ok", false) }.getOrDefault(false)
    }

    fun dispatchInput(
        consumerId: String,
        viewId: String,
        generation: String,
        sourceRevision: Long,
        eventJson: String,
    ): DispatchInputResult {
        if (consumerId.isBlank() || viewId.isBlank() || generation.isBlank() || eventJson.isBlank()) {
            return DispatchInputResult(ok = false, accepted = false, error = "invalid_input_arguments")
        }
        val raw = try {
            transport.dispatchInput(consumerId, viewId, generation, sourceRevision, eventJson)
                ?: return DispatchInputResult(ok = false, accepted = false, error = "octendra_unavailable")
        } catch (error: Throwable) {
            logger("Octendra dispatch_input failed", error)
            return DispatchInputResult(
                ok = false,
                accepted = false,
                error = error.message ?: "dispatch_input_failed",
            )
        }
        return runCatching {
            val json = JSONObject(raw)
            val ok = json.optBoolean("ok", false)
            val accepted = ok && json.optBoolean("accepted", false)
            val error = if (json.has("error") && !json.isNull("error")) {
                json.optString("error", "").takeIf { it.isNotBlank() }
            } else null
            val currentRevision = if (json.has("source_revision") && !json.isNull("source_revision")) {
                json.optLong("source_revision")
            } else null
            DispatchInputResult(
                ok = ok,
                accepted = accepted,
                resourceEpoch = json.optLong("resource_epoch", 0L),
                error = error,
                currentSourceRevision = currentRevision,
            )
        }.getOrElse { error ->
            logger("Malformed Octendra dispatch_input response", error)
            DispatchInputResult(ok = false, accepted = false, error = "malformed_dispatch_response")
        }
    }

    fun releaseInput(consumerId: String): Boolean {
        if (consumerId.isBlank()) return false
        val raw = try {
            transport.releaseInput(consumerId) ?: return false
        } catch (error: Throwable) {
            logger("Octendra release_input failed", error)
            return false
        }
        return runCatching { JSONObject(raw).optBoolean("ok", false) }.getOrDefault(false)
    }

    private class AndroidTransport(private val context: Context) : Transport {
        private val trustLock = Any()
        @Volatile private var cachedTrusted = false
        @Volatile private var cachedAtMs = 0L

        override fun isAvailable(): Boolean = verifyPeer(force = true)

        override fun describeViews(): String? {
            if (!verifyPeer(force = true)) return null
            return callProvider(METHOD_DESCRIBE_VIEWS, null, null)?.getString("views_json")
        }

        override fun attachVideo(viewId: String, generation: String, surface: Any): String? {
            val androidSurface = surface as? Surface ?: return null
            if (!verifyPeer(force = true)) return null
            val extras = Bundle().apply {
                putString("generation", generation)
                putParcelable("surface", androidSurface)
            }
            return callProvider(METHOD_ATTACH_VIDEO, viewId, extras)?.getString("result_json")
        }

        override fun detachVideo(consumerId: String): String? {
            if (!verifyPeer(force = false)) return null
            return callProvider(METHOD_DETACH_VIDEO, consumerId, null)?.getString("result_json")
        }

        override fun dispatchInput(
            consumerId: String,
            viewId: String,
            generation: String,
            sourceRevision: Long,
            eventJson: String,
        ): String? {
            if (!verifyPeer(force = false)) return null
            val extras = Bundle().apply {
                putString("view_id", viewId)
                putString("generation", generation)
                putLong("source_revision", sourceRevision)
                putString("event_json", eventJson)
            }
            return callProvider(METHOD_DISPATCH_INPUT, consumerId, extras)?.getString("result_json")
        }

        override fun releaseInput(consumerId: String): String? {
            if (!verifyPeer(force = false)) return null
            return callProvider(METHOD_RELEASE_INPUT, consumerId, null)?.getString("result_json")
        }

        private fun invalidateTrustCache() {
            synchronized(trustLock) {
                cachedTrusted = false
                cachedAtMs = 0L
            }
        }

        private fun verifyPeer(force: Boolean): Boolean {
            val now = SystemClock.elapsedRealtime()
            synchronized(trustLock) {
                if (!force && cachedTrusted && now - cachedAtMs in 0L..5_000L) {
                    return true
                }
                val trusted = runCatching { checkPeerTrustUncached() }.getOrDefault(false)
                cachedTrusted = trusted
                cachedAtMs = if (trusted) now else 0L
                return trusted
            }
        }

        private fun checkPeerTrustUncached(): Boolean {
            val pm = context.packageManager
            val provider = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                pm.resolveContentProvider(AI_VIEW_AUTHORITY, PackageManager.ComponentInfoFlags.of(0L))
            } else {
                @Suppress("DEPRECATION")
                pm.resolveContentProvider(AI_VIEW_AUTHORITY, 0)
            } ?: return false
            if (!provider.enabled || provider.applicationInfo?.enabled != true) return false
            if (provider.packageName != OCTENDRA_PACKAGE) return false
            val componentState = pm.getComponentEnabledSetting(ComponentName(provider.packageName, provider.name))
            if (componentState == PackageManager.COMPONENT_ENABLED_STATE_DISABLED ||
                componentState == PackageManager.COMPONENT_ENABLED_STATE_DISABLED_USER ||
                componentState == PackageManager.COMPONENT_ENABLED_STATE_DISABLED_UNTIL_USED
            ) return false
            val peerInfo = signingPackageInfo(pm, OCTENDRA_PACKAGE) ?: return false
            if (peerInfo.applicationInfo?.enabled != true) return false
            val peerCerts = extractCertSha256s(peerInfo)
            val ownCerts = signingPackageInfo(pm, context.packageName)
                ?.let(::extractCertSha256s)
                ?.toSet()
                .orEmpty()
            return isTrustedOctendraPeer(provider.packageName, peerCerts, ownCerts)
        }

        private fun signingPackageInfo(pm: PackageManager, packageName: String): PackageInfo? =
            runCatching {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    pm.getPackageInfo(
                        packageName,
                        PackageManager.PackageInfoFlags.of(PackageManager.GET_SIGNING_CERTIFICATES.toLong()),
                    )
                } else if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                    @Suppress("DEPRECATION")
                    pm.getPackageInfo(packageName, PackageManager.GET_SIGNING_CERTIFICATES)
                } else {
                    @Suppress("DEPRECATION")
                    pm.getPackageInfo(packageName, PackageManager.GET_SIGNATURES)
                }
            }.getOrNull()

        private fun extractCertSha256s(info: PackageInfo): List<String> {
            val signatures = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                info.signingInfo?.apkContentsSigners
            } else {
                @Suppress("DEPRECATION")
                info.signatures
            } ?: return emptyList()
            return signatures.map { sha256Hex(it.toByteArray()) }
        }

        private fun callProvider(method: String, arg: String?, extras: Bundle?): Bundle? {
            return try {
                val client = context.contentResolver.acquireUnstableContentProviderClient(AI_VIEW_AUTHORITY)
                    ?: return null
                client.use { it.call(method, arg, extras) }
            } catch (_: Throwable) {
                invalidateTrustCache()
                null
            }
        }
    }
}
