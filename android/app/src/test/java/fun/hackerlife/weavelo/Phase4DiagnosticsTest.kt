package `fun`.hackerlife.weavelo

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.double
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class Phase4DiagnosticsTest {

    @Test
    fun redactsBearerTokensAndAuthorizationHeaders() {
        val raw = """
            Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.abc123.sig_xyz
            authorization=Basic dXNlcjpwYXNzMTIz
            retry with bearer my_opaque_token_9876543210
        """.trimIndent()

        val scrubbed = DiagnosticsExporter.redactSensitiveText(raw)
        assertFalse(scrubbed.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"))
        assertFalse(scrubbed.contains("dXNlcjpwYXNzMTIz"))
        assertFalse(scrubbed.contains("my_opaque_token_9876543210"))
        assertTrue(scrubbed.contains("Bearer [REDACTED_TOKEN]"))
        assertTrue(scrubbed.contains("Basic [REDACTED_TOKEN]"))
        assertTrue(scrubbed.contains("bearer [REDACTED_TOKEN]"))
    }

    @Test
    fun redactsPasswordSecretAndTokenKeyValuesAcrossFormats() {
        val raw = """
            https://relay.example.com/connect?user=alice&password=SuperSecret!123&token=tok_live_abc999
            config secret="my quoted secret" api_key='key_987654' client_secret=cs_112233
            {"username":"alice","password":"json_pw_value","access_token":"at_value_42","secret":"s3cr3t"}
        """.trimIndent()

        val scrubbed = DiagnosticsExporter.redactSensitiveText(raw)
        for (leaked in listOf("SuperSecret!123", "tok_live_abc999", "my quoted secret", "key_987654", "cs_112233", "json_pw_value", "at_value_42", "s3cr3t")) {
            assertFalse("Expected '$leaked' to be scrubbed, got:\n$scrubbed", scrubbed.contains(leaked))
        }
        assertTrue(scrubbed.contains("user=alice"))
        assertTrue(scrubbed.contains("password=[REDACTED]"))
        assertTrue(scrubbed.contains("token=[REDACTED]"))
        assertTrue(scrubbed.contains(""""password":"[REDACTED]""""))
        assertTrue(scrubbed.contains(""""access_token":"[REDACTED]""""))
    }

    @Test
    fun redactsPemKeyBlocksSingleAndMultiLine() {
        val pem = """
            loaded key:
            -----BEGIN PRIVATE KEY-----
            MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7SecretBodyLine1
            SecondSecretLine2+Base64Payload==
            -----END PRIVATE KEY-----
            done loading
        """.trimIndent()

        val scrubbed = DiagnosticsExporter.redactSensitiveText(pem)
        assertFalse(scrubbed.contains("SecretBodyLine1"))
        assertFalse(scrubbed.contains("SecondSecretLine2"))
        assertFalse(scrubbed.contains("BEGIN PRIVATE KEY"))
        assertTrue(scrubbed.contains("[REDACTED_PEM]"))
        assertTrue(scrubbed.contains("loaded key:"))
        assertTrue(scrubbed.contains("done loading"))
    }

    @Test
    fun redactsNonLoopbackIpv4WhilePreservingLoopbackAndInvalidQuads() {
        val raw = "peer=192.168.1.42:8443 wan=203.0.113.9 lan=10.0.0.5 local=127.0.0.1 alt_loopback=127.0.1.1 invalid=999.1.2.3"
        val scrubbed = DiagnosticsExporter.redactSensitiveText(raw)

        assertFalse(scrubbed.contains("192.168.1.42"))
        assertFalse(scrubbed.contains("203.0.113.9"))
        assertFalse(scrubbed.contains("10.0.0.5"))
        assertTrue(scrubbed.contains("peer=[REDACTED_IP]:8443"))
        assertTrue(scrubbed.contains("wan=[REDACTED_IP]"))
        assertTrue(scrubbed.contains("lan=[REDACTED_IP]"))
        assertTrue(scrubbed.contains("local=127.0.0.1"))
        assertTrue(scrubbed.contains("alt_loopback=127.0.1.1"))
        assertTrue(scrubbed.contains("invalid=999.1.2.3"))
    }

    @Test
    fun buildRedactedJsonProducesValidStructuredReportAndScrubsAllFreeText() {
        val input = DiagnosticsReportInput(
            appVersionName = "2.0.0-alpha.22",
            appVersionCode = 138,
            deviceManufacturer = "Google",
            deviceModel = "Pixel 9",
            sdkInt = 35,
            sessionState = SessionState.STREAMING,
            routePreference = RoutePreference.RELAY_WSS,
            discoveryDiagnostics = DiscoveryDiagnostics(
                phase = "discovering",
                generation = 4,
                reloadRequested = 2,
                reloadApplied = 2,
                routes = 3,
                peers = 2,
                p2p = "running",
                failure = "failed connecting 198.51.100.77 with token=secret_fail_tok",
                control = RelayDiagnostics(
                    state = "connected",
                    attempts = 3,
                    registrationsSent = 3,
                    pongsReceived = 12,
                    dataFramesReceived = 40,
                    stage = "active",
                    httpStatus = 101,
                ),
                discovery = RelayDiagnostics(
                    state = "connected",
                    attempts = 1,
                    pongsReceived = 5,
                    stage = "active",
                ),
            ),
            relayProbe = PlatformRelayProbeResult(
                phase = "complete",
                dnsCount = 2,
                tcpConnected = true,
                tlsVerified = true,
                httpStatus = 101,
                websocketVerified = true,
            ),
            telemetry = TelemetrySnapshot(
                fps = 59.5f,
                latencyMs = 4.2f,
                captureReceiveEstimateMs = 17.5,
                networkRttMs = 11.2,
                rawTotalRttMs = Double.NaN, // must serialize as null rather than throwing
                clockState = "converged",
                clockUncertaintyMs = Double.POSITIVE_INFINITY,
                jitterMs = 1.1f,
                packetLossPercent = 0.25f,
                videoBitrateKbps = 12_000,
                audioBitrateKbps = 128,
                controlBitrateKbps = 16,
                fileBitrateKbps = 0,
                transportHealthScore = 0.96f,
            ),
            virtualDisplayCount = 2,
            portForwardCount = 3,
            recentLogs = listOf(
                "info: loopback 127.0.0.1:8080 ok",
                "warn: connect 192.168.50.10 Bearer eyJsecretToken999 password=my_pw",
                "-----BEGIN RSA PRIVATE KEY-----",
                "SUPER_SECRET_PEM_BODY_SHOULD_NOT_LEAK",
                "-----END RSA PRIVATE KEY-----",
                "info: session healthy",
            ),
            nativeLibraryLoaded = true,
            nativeStartupError = "",
            sessionError = "auth failed for 10.20.30.40 secret=topsecret",
            peerCount = 2,
            generatedAtEpochMs = 1_760_000_000_000L,
        )

        val jsonText = DiagnosticsExporter.buildRedactedJson(input, prettyPrint = true)

        // Valid JSON in both kotlinx.serialization and org.json
        val parsed = Json.parseToJsonElement(jsonText).jsonObject
        val orgJson = JSONObject(jsonText)
        assertEquals(DiagnosticsExporter.SCHEMA_VERSION, orgJson.getInt("schema_version"))
        assertTrue(parsed["redacted"]!!.jsonPrimitive.boolean)
        assertEquals(1_760_000_000_000L, parsed["generated_at_epoch_ms"]!!.jsonPrimitive.long)

        val app = parsed["app"]!!.jsonObject
        assertEquals("2.0.0-alpha.22", app["version_name"]!!.jsonPrimitive.content)
        assertEquals(138, app["version_code"]!!.jsonPrimitive.int)
        assertTrue(app["native_library_loaded"]!!.jsonPrimitive.boolean)

        val device = parsed["device"]!!.jsonObject
        assertEquals("Google", device["manufacturer"]!!.jsonPrimitive.content)
        assertEquals("Pixel 9", device["model"]!!.jsonPrimitive.content)
        assertEquals(35, device["sdk_int"]!!.jsonPrimitive.int)

        val session = parsed["session"]!!.jsonObject
        assertEquals("STREAMING", session["state"]!!.jsonPrimitive.content)
        assertEquals("relay_wss", session["route_preference"]!!.jsonPrimitive.content)
        assertEquals(2, session["peer_count"]!!.jsonPrimitive.int)

        val features = parsed["features"]!!.jsonObject
        assertEquals(2, features["virtual_display_count"]!!.jsonPrimitive.int)
        assertEquals(3, features["port_forward_count"]!!.jsonPrimitive.int)

        val discovery = parsed["discovery"]!!.jsonObject
        assertEquals("discovering", discovery["phase"]!!.jsonPrimitive.content)
        assertEquals("connected", discovery["relay_control"]!!.jsonObject["state"]!!.jsonPrimitive.content)

        val probe = parsed["relay_probe"]!!.jsonObject
        assertTrue(probe["websocket_verified"]!!.jsonPrimitive.boolean)
        assertEquals(101, probe["http_status"]!!.jsonPrimitive.int)

        val telemetry = parsed["telemetry"]!!.jsonObject
        assertEquals(11.2, telemetry["network_rtt_ms"]!!.jsonPrimitive.double, 0.001)
        assertEquals(12_000, telemetry["video_bitrate_kbps"]!!.jsonPrimitive.int)
        assertTrue(orgJson.getJSONObject("telemetry").isNull("raw_total_rtt_ms"))
        assertTrue(orgJson.getJSONObject("telemetry").isNull("clock_uncertainty_ms"))

        // Verify zero sensitive leakage anywhere in the serialized JSON
        for (secret in listOf(
            "198.51.100.77",
            "secret_fail_tok",
            "192.168.50.10",
            "eyJsecretToken999",
            "my_pw",
            "SUPER_SECRET_PEM_BODY_SHOULD_NOT_LEAK",
            "10.20.30.40",
            "topsecret",
        )) {
            assertFalse("Sensitive value '$secret' leaked into JSON!", jsonText.contains(secret))
        }
        assertTrue(jsonText.contains("127.0.0.1:8080"))
    }

    @Test
    fun tailAndRedactLogsEnforcesLineCountPerLineLengthAndTotalCharBudget() {
        val rawLines = (1..250).map { idx -> "line-$idx normal message" }
        val tail = DiagnosticsExporter.tailAndRedactLogs(
            lines = rawLines,
            maxLines = 20,
            maxLineChars = 128,
            maxTotalChars = 32_768,
        )
        assertEquals(250, tail.totalLines)
        assertEquals(20, tail.lines.size)
        assertEquals(230, tail.omittedLines)
        assertTrue(tail.truncated)
        assertEquals("line-231 normal message", tail.lines.first())
        assertEquals("line-250 normal message", tail.lines.last())

        // Per-line truncation
        val longLine = "prefix-" + "x".repeat(600)
        val clipped = DiagnosticsExporter.tailAndRedactLogs(
            lines = listOf(longLine),
            maxLines = 10,
            maxLineChars = 64,
        )
        assertEquals(1, clipped.lines.size)
        assertEquals(64, clipped.lines.first().length)
        assertTrue(clipped.lines.first().endsWith("...[truncated]"))
        assertTrue(clipped.truncated)

        // Total char budget keeps newest lines within limit
        val manyLines = (1..30).map { idx -> "entry-$idx-" + "a".repeat(40) }
        val budgetTail = DiagnosticsExporter.tailAndRedactLogs(
            lines = manyLines,
            maxLines = 30,
            maxLineChars = 100,
            maxTotalChars = 160,
        )
        assertTrue(budgetTail.truncated)
        assertTrue(budgetTail.lines.sumOf { it.length } <= 160)
        assertEquals(manyLines.last(), budgetTail.lines.last())

        // Zero maxLines omits all lines cleanly
        val zeroTail = DiagnosticsExporter.tailAndRedactLogs(
            lines = listOf("a", "b"),
            maxLines = 0,
        )
        assertTrue(zeroTail.lines.isEmpty())
        assertEquals(2, zeroTail.totalLines)
        assertEquals(2, zeroTail.omittedLines)
        assertTrue(zeroTail.truncated)
    }

    @Test
    fun buildRedactedJsonReflectsTruncatedLogMetadata() {
        val logs = (1..15).map { "log #$it from 172.16.0.$it" }
        val jsonText = DiagnosticsExporter.buildRedactedJson(
            DiagnosticsReportInput(
                appVersionName = "2.0.0",
                appVersionCode = 1,
                deviceManufacturer = "Test",
                deviceModel = "Unit",
                sdkInt = 34,
                recentLogs = logs,
                maxLogLines = 5,
            ),
            prettyPrint = false,
        )

        val logsObj = Json.parseToJsonElement(jsonText).jsonObject["logs"]!!.jsonObject
        assertEquals(15, logsObj["total_lines"]!!.jsonPrimitive.int)
        assertEquals(5, logsObj["included_lines"]!!.jsonPrimitive.int)
        assertEquals(10, logsObj["omitted_lines"]!!.jsonPrimitive.int)
        assertTrue(logsObj["truncated"]!!.jsonPrimitive.boolean)
        val included = logsObj["lines"]!!.jsonArray.map { it.jsonPrimitive.content }
        assertEquals(5, included.size)
        assertEquals("log #11 from [REDACTED_IP]", included.first())
        assertEquals("log #15 from [REDACTED_IP]", included.last())
    }
}
