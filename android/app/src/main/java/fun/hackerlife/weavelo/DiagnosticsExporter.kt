package `fun`.hackerlife.weavelo

import android.content.Context
import android.os.Build
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

data class DiagnosticsReportInput(
    val appVersionName: String = BuildConfig.VERSION_NAME,
    val appVersionCode: Int = BuildConfig.VERSION_CODE,
    val deviceManufacturer: String = Build.MANUFACTURER.orEmpty(),
    val deviceModel: String = Build.MODEL.orEmpty(),
    val sdkInt: Int = Build.VERSION.SDK_INT,
    val sessionState: SessionState = SessionState.DISCONNECTED,
    val routePreference: RoutePreference = RoutePreference.AUTO,
    val discoveryDiagnostics: DiscoveryDiagnostics = DiscoveryDiagnostics(),
    val relayProbe: PlatformRelayProbeResult? = null,
    val telemetry: TelemetrySnapshot = TelemetrySnapshot(),
    val virtualDisplayCount: Int = 0,
    val portForwardCount: Int = 0,
    val recentLogs: List<String> = emptyList(),
    val maxLogLines: Int = DiagnosticsExporter.DEFAULT_MAX_LOG_LINES,
    val maxLogLineChars: Int = DiagnosticsExporter.DEFAULT_MAX_LOG_LINE_CHARS,
    val maxTotalLogChars: Int = DiagnosticsExporter.DEFAULT_MAX_TOTAL_LOG_CHARS,
    val nativeLibraryLoaded: Boolean = true,
    val nativeStartupError: String = "",
    val sessionError: String = "",
    val peerCount: Int = 0,
    val generatedAtEpochMs: Long = System.currentTimeMillis(),
)

data class BoundedLogTail(
    val lines: List<String>,
    val totalLines: Int,
    val omittedLines: Int,
    val truncated: Boolean,
)

object DiagnosticsExporter {
    const val SCHEMA_VERSION: Int = 1
    const val DEFAULT_MAX_LOG_LINES: Int = 200
    const val DEFAULT_MAX_LOG_LINE_CHARS: Int = 512
    const val DEFAULT_MAX_TOTAL_LOG_CHARS: Int = 32_768

    private const val REDACTED = "[REDACTED]"
    private const val REDACTED_TOKEN = "[REDACTED_TOKEN]"
    private const val REDACTED_PEM = "[REDACTED_PEM]"
    private const val REDACTED_IP = "[REDACTED_IP]"
    private const val TRUNCATION_SUFFIX = "...[truncated]"

    private val prettyJson = Json { prettyPrint = true }
    private val compactJson = Json { prettyPrint = false }

    private val pemBlockRegex = Regex(
        """-----BEGIN [A-Z0-9 ]+-----[\s\S]*?-----END [A-Z0-9 ]+-----""",
    )
    private val unclosedPemRegex = Regex(
        """-----BEGIN [A-Z0-9 ]*(?:KEY|CERTIFICATE|CERT|PRIVATE)-----[\s\S]*$""",
    )
    private val pemBeginLineRegex = Regex("""-----BEGIN [A-Z0-9 ]+-----""")
    private val pemEndLineRegex = Regex("""-----END [A-Z0-9 ]+-----""")

    private val authHeaderRegex = Regex(
        """(?i)(\bAuthorization\s*[:=]\s*(?:Bearer|Basic|Token)\s+)([^\s,;'"}\]]+)""",
    )
    private val bearerTokenRegex = Regex(
        """(?i)\b(Bearer\s+)([A-Za-z0-9._~+/=-]{6,})""",
    )

    private const val SENSITIVE_KEY_PATTERN =
        """password|passwd|passphrase|secret|client_secret|token|access_token|refresh_token|auth_token|id_token|api_key|apikey|private_key|session_key"""

    private val jsonSensitiveKeyRegex = Regex(
        """(?i)("($SENSITIVE_KEY_PATTERN)"\s*:\s*")((?:\\.|[^"\\])*)(")""",
    )
    private val quotedKeyValueRegex = Regex(
        """(?i)\b(($SENSITIVE_KEY_PATTERN)\s*[=:]\s*)(["'])((?:\\.|(?!\3).)*)\3""",
    )
    private val unquotedKeyValueRegex = Regex(
        """(?i)\b(($SENSITIVE_KEY_PATTERN)\s*[=:]\s*)([^\s&;,)"'}\]<>]+)""",
    )
    private val ipv4Regex = Regex(
        """(?<![0-9.])(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})(?![0-9.])""",
    )

    /**
     * Scrubs PEM key/certificate blocks, bearer/authorization tokens, secret/password/token
     * key-value pairs, and non-loopback IPv4 addresses while preserving loopback (127.0.0.0/8).
     */
    fun redactSensitiveText(raw: String): String {
        if (raw.isEmpty()) return raw
        var result = pemBlockRegex.replace(raw, REDACTED_PEM)
        result = unclosedPemRegex.replace(result, REDACTED_PEM)
        result = authHeaderRegex.replace(result) { match ->
            "${match.groupValues[1]}$REDACTED_TOKEN"
        }
        result = bearerTokenRegex.replace(result) { match ->
            "${match.groupValues[1]}$REDACTED_TOKEN"
        }
        result = jsonSensitiveKeyRegex.replace(result) { match ->
            "${match.groupValues[1]}$REDACTED${match.groupValues[4]}"
        }
        result = quotedKeyValueRegex.replace(result) { match ->
            val quote = match.groupValues[3]
            "${match.groupValues[1]}$quote$REDACTED$quote"
        }
        result = unquotedKeyValueRegex.replace(result) { match ->
            val value = match.groupValues[3]
            if (value == REDACTED || value == REDACTED_TOKEN) {
                match.value
            } else {
                "${match.groupValues[1]}$REDACTED"
            }
        }
        result = ipv4Regex.replace(result) { match ->
            val o1 = match.groupValues[1].toIntOrNull()
            val o2 = match.groupValues[2].toIntOrNull()
            val o3 = match.groupValues[3].toIntOrNull()
            val o4 = match.groupValues[4].toIntOrNull()
            if (o1 == null || o2 == null || o3 == null || o4 == null ||
                o1 !in 0..255 || o2 !in 0..255 || o3 !in 0..255 || o4 !in 0..255
            ) {
                match.value
            } else if (o1 == 127) {
                match.value
            } else {
                REDACTED_IP
            }
        }
        return result
    }

    /**
     * Collapses multi-line PEM blocks, redacts sensitive tokens/secrets/IPs, and bounds the log
     * tail by line count, per-line length, and total character budget (prioritizing newest lines).
     */
    fun tailAndRedactLogs(
        lines: List<String>,
        maxLines: Int = DEFAULT_MAX_LOG_LINES,
        maxLineChars: Int = DEFAULT_MAX_LOG_LINE_CHARS,
        maxTotalChars: Int = DEFAULT_MAX_TOTAL_LOG_CHARS,
    ): BoundedLogTail {
        val totalLines = lines.size
        val lineLimit = maxLines.coerceAtLeast(0)
        val perLineLimit = maxLineChars.coerceAtLeast(1)
        val totalCharLimit = maxTotalChars.coerceAtLeast(0)

        if (totalLines == 0) {
            return BoundedLogTail(
                lines = emptyList(),
                totalLines = 0,
                omittedLines = 0,
                truncated = false,
            )
        }
        if (lineLimit == 0 || totalCharLimit == 0) {
            return BoundedLogTail(
                lines = emptyList(),
                totalLines = totalLines,
                omittedLines = totalLines,
                truncated = true,
            )
        }

        // Collapse multi-line PEM blocks across the entire input before tailing so slicing
        // cannot expose the body of a PEM block.
        val pemCollapsed = ArrayList<String>(totalLines)
        var inPemBlock = false
        for (rawLine in lines) {
            if (inPemBlock) {
                if (pemEndLineRegex.containsMatchIn(rawLine)) {
                    inPemBlock = false
                }
                continue
            }
            if (pemBeginLineRegex.containsMatchIn(rawLine)) {
                if (pemEndLineRegex.containsMatchIn(rawLine)) {
                    pemCollapsed += redactSensitiveText(rawLine)
                } else {
                    val prefix = rawLine.substringBefore("-----BEGIN").trimEnd()
                    val marker = if (prefix.isEmpty()) REDACTED_PEM else "${redactSensitiveText(prefix)} $REDACTED_PEM"
                    pemCollapsed += marker
                    inPemBlock = true
                }
            } else {
                pemCollapsed += rawLine
            }
        }

        val tailSlice = if (pemCollapsed.size > lineLimit) {
            pemCollapsed.subList(pemCollapsed.size - lineLimit, pemCollapsed.size)
        } else {
            pemCollapsed
        }

        val redactedTail = tailSlice.map { line ->
            val sanitized = redactSensitiveText(line)
                .replace('\r', ' ')
                .replace('\n', ' ')
            truncateLine(sanitized, perLineLimit)
        }

        // Enforce total character budget from newest line backwards.
        val boundedReversed = ArrayList<String>(redactedTail.size)
        var usedChars = 0
        for (index in redactedTail.indices.reversed()) {
            val candidate = redactedTail[index]
            if (usedChars + candidate.length <= totalCharLimit) {
                boundedReversed += candidate
                usedChars += candidate.length
            } else if (boundedReversed.isEmpty() && totalCharLimit > 0) {
                boundedReversed += truncateLine(candidate, totalCharLimit)
                usedChars = totalCharLimit
                break
            } else {
                break
            }
        }
        boundedReversed.reverse()

        val includedCount = boundedReversed.size
        val omittedLines = (totalLines - includedCount).coerceAtLeast(0)
        val anyLineClipped = tailSlice.size != redactedTail.size ||
            redactedTail.any { it.endsWith(TRUNCATION_SUFFIX) }
        val truncated = omittedLines > 0 || anyLineClipped

        return BoundedLogTail(
            lines = boundedReversed,
            totalLines = totalLines,
            omittedLines = omittedLines,
            truncated = truncated,
        )
    }

    fun buildRedactedJson(
        input: DiagnosticsReportInput,
        prettyPrint: Boolean = true,
    ): String {
        val logTail = tailAndRedactLogs(
            lines = input.recentLogs,
            maxLines = input.maxLogLines,
            maxLineChars = input.maxLogLineChars,
            maxTotalChars = input.maxTotalLogChars,
        )

        val root = buildJsonObject {
            put("schema_version", SCHEMA_VERSION)
            put("redacted", true)
            put("generated_at_epoch_ms", input.generatedAtEpochMs)

            put("app", buildJsonObject {
                put("version_name", redactSensitiveText(input.appVersionName))
                put("version_code", input.appVersionCode)
                put("native_library_loaded", input.nativeLibraryLoaded)
                put("native_startup_error", redactSensitiveText(input.nativeStartupError))
            })

            put("device", buildJsonObject {
                put("manufacturer", redactSensitiveText(input.deviceManufacturer.ifBlank { "unknown" }))
                put("model", redactSensitiveText(input.deviceModel.ifBlank { "unknown" }))
                put("sdk_int", input.sdkInt)
            })

            put("session", buildJsonObject {
                put("state", input.sessionState.name)
                put("route_preference", input.routePreference.wire)
                put("peer_count", input.peerCount.coerceAtLeast(0))
                put("error", redactSensitiveText(input.sessionError))
            })

            put("features", buildJsonObject {
                put("virtual_display_count", input.virtualDisplayCount.coerceAtLeast(0))
                put("port_forward_count", input.portForwardCount.coerceAtLeast(0))
            })

            put("discovery", buildDiscoveryObject(input.discoveryDiagnostics))

            val probe = input.relayProbe
            if (probe != null) {
                put("relay_probe", buildRelayProbeObject(probe))
            } else {
                put("relay_probe", JsonNull)
            }

            put("telemetry", buildTelemetryObject(input.telemetry))

            put("logs", buildJsonObject {
                put("total_lines", logTail.totalLines)
                put("included_lines", logTail.lines.size)
                put("omitted_lines", logTail.omittedLines)
                put("truncated", logTail.truncated)
                put("lines", JsonArray(logTail.lines.map(::JsonPrimitive)))
            })
        }

        val formatter = if (prettyPrint) prettyJson else compactJson
        return formatter.encodeToString(JsonObject.serializer(), root)
    }

    fun collectLiveInput(
        context: Context,
        recentLogs: List<String> = emptyList(),
        relayProbe: PlatformRelayProbeResult? = null,
    ): DiagnosticsReportInput {
        val appContext = context.applicationContext
        val virtualDisplayCount = runCatching { VirtualDisplayStore.list(appContext).size }.getOrDefault(0)
        val portForwardCount = runCatching { PortForwardStore.list(appContext).size }.getOrDefault(0)
        return DiagnosticsReportInput(
            appVersionName = BuildConfig.VERSION_NAME,
            appVersionCode = BuildConfig.VERSION_CODE,
            deviceManufacturer = Build.MANUFACTURER.orEmpty(),
            deviceModel = Build.MODEL.orEmpty(),
            sdkInt = Build.VERSION.SDK_INT,
            sessionState = WeaveloClient.sessionState.value,
            routePreference = WeaveloClient.routePreference.value,
            discoveryDiagnostics = WeaveloClient.discoveryDiagnostics.value,
            relayProbe = relayProbe,
            telemetry = WeaveloClient.telemetry.value,
            virtualDisplayCount = virtualDisplayCount,
            portForwardCount = portForwardCount,
            recentLogs = recentLogs,
            nativeLibraryLoaded = WeaveloClient.nativeStartupError.isEmpty(),
            nativeStartupError = WeaveloClient.nativeStartupError,
            sessionError = WeaveloClient.sessionError.value,
            peerCount = WeaveloClient.devices.value.size,
        )
    }

    private fun buildDiscoveryObject(diag: DiscoveryDiagnostics): JsonObject = buildJsonObject {
        put("phase", diag.phase)
        put("generation", diag.generation)
        put("reload_requested", diag.reloadRequested)
        put("reload_applied", diag.reloadApplied)
        put("network_name_sha256", diag.networkHash)
        put("device_id_sha256", diag.deviceHash)
        put("peer_device_id_sha256", JsonArray(diag.peerHashes.map(::JsonPrimitive)))
        put("routes", diag.routes)
        put("peers", diag.peers)
        put("p2p", diag.p2p)
        put("failure", redactSensitiveText(diag.failure))
        put("relay_control", buildRelayObject(diag.control))
        put("relay_discovery", buildRelayObject(diag.discovery))
    }

    private fun buildRelayObject(value: RelayDiagnostics): JsonObject = buildJsonObject {
        put("state", value.state)
        put("attempts", value.attempts)
        put("registrations_sent", value.registrationsSent)
        put("pongs_received", value.pongsReceived)
        put("data_frames_received", value.dataFramesReceived)
        put("error", redactSensitiveText(value.error))
        put("stage", value.stage)
        put("last_failure_stage", value.failedStage)
        put("dns_results", value.dnsResults)
        put("tcp_connections", value.tcpConnections)
        put("tls_handshakes", value.tlsHandshakes)
        put("websocket_upgrades", value.websocketUpgrades)
        put("tls_bytes_sent", value.tlsBytesSent)
        put("tls_bytes_received", value.tlsBytesReceived)
        put("errno", value.errno?.let(::JsonPrimitive) ?: JsonNull)
        put("http_status", value.httpStatus?.let(::JsonPrimitive) ?: JsonNull)
        put("dns_address_sha256", JsonArray(value.dnsHashes.map(::JsonPrimitive)))
        put("dns_benchmark_count", value.dnsBenchmarkCount)
    }

    private fun buildRelayProbeObject(probe: PlatformRelayProbeResult): JsonObject = buildJsonObject {
        put("phase", probe.phase)
        put("error", redactSensitiveText(probe.error))
        put("dns_count", probe.dnsCount)
        put("tcp_connected", probe.tcpConnected)
        put("tls_verified", probe.tlsVerified)
        put("http_status", probe.httpStatus)
        put("websocket_verified", probe.websocketVerified)
        put("dns_address_sha256", JsonArray(probe.dnsHashes.map(::JsonPrimitive)))
        put("dns_benchmark_count", probe.dnsBenchmarkCount)
        put("error_type", probe.errorType)
        put("cause_type", probe.causeType)
        put("os_error", probe.osError?.let(::JsonPrimitive) ?: JsonNull)
    }

    private fun buildTelemetryObject(t: TelemetrySnapshot): JsonObject = buildJsonObject {
        put("fps", finiteFloatPrimitive(t.fps))
        put("decode_latency_ms", finiteFloatPrimitive(t.latencyMs))
        put("capture_receive_estimate_ms", finiteDoublePrimitive(t.captureReceiveEstimateMs))
        put("network_rtt_ms", finiteDoublePrimitive(t.networkRttMs))
        put("raw_total_rtt_ms", finiteDoublePrimitive(t.rawTotalRttMs))
        put("clock_state", t.clockState)
        put("clock_uncertainty_ms", finiteDoublePrimitive(t.clockUncertaintyMs))
        put("jitter_ms", finiteFloatPrimitive(t.jitterMs))
        put("packet_loss_percent", finiteFloatPrimitive(t.packetLossPercent))
        put("video_bitrate_kbps", t.videoBitrateKbps.coerceAtLeast(0))
        put("audio_bitrate_kbps", t.audioBitrateKbps.coerceAtLeast(0))
        put("control_bitrate_kbps", t.controlBitrateKbps.coerceAtLeast(0))
        put("file_bitrate_kbps", t.fileBitrateKbps.coerceAtLeast(0))
        put("transport_health_score", finiteFloatPrimitive(t.transportHealthScore))
    }

    private fun finiteDoublePrimitive(value: Double?) =
        if (value != null && value.isFinite()) JsonPrimitive(value) else JsonNull

    private fun finiteFloatPrimitive(value: Float) =
        if (value.isFinite()) JsonPrimitive(value) else JsonNull

    private fun truncateLine(line: String, maxChars: Int): String {
        if (line.length <= maxChars) return line
        if (maxChars <= TRUNCATION_SUFFIX.length) return line.take(maxChars)
        return line.take(maxChars - TRUNCATION_SUFFIX.length) + TRUNCATION_SUFFIX
    }
}
