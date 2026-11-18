package `fun`.hackerlife.weavelo

import `fun`.hackerlife.weavelo.managed.ManagedApplications
import `fun`.hackerlife.weavelo.managed.ManagedFallbackInput
import `fun`.hackerlife.weavelo.octendra.AiBridge
import `fun`.hackerlife.weavelo.octendra.AiVideoPublisher
import `fun`.hackerlife.weavelo.octendra.AiVideoPublisherFactory
import `fun`.hackerlife.weavelo.octendra.OctendraAiClient
import `fun`.hackerlife.weavelo.octendra.OctendraAiInput
import `fun`.hackerlife.weavelo.octendra.OctendraAiViews
import `fun`.hackerlife.weavelo.octendra.WindowCatalogCoordinator
import `fun`.hackerlife.weavelo.octendra.isTrustedOctendraPeer
import `fun`.hackerlife.weavelo.octendra.normalizeRotationQuadrant
import `fun`.hackerlife.weavelo.octendra.sha256Hex
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test
import java.util.zip.CRC32

class OctendraAiBridgeTest {

    private class FakeTransport : OctendraAiClient.Transport {
        var available = true
        var throwOnDescribe: Throwable? = null
        var viewsJson: String? = "[]"
        var nextConsumerSeq = 1
        val activeConsumers = linkedMapOf<String, JSONObject>()
        val attachCalls = mutableListOf<Triple<String, String, Any>>()
        val detachCalls = mutableListOf<String>()
        val dispatchCalls = mutableListOf<DispatchRecord>()
        val releaseCalls = mutableListOf<String>()
        var dispatchError: Pair<String, Long?>? = null
        var resourceEpoch = 1L

        data class DispatchRecord(
            val consumerId: String,
            val viewId: String,
            val generation: String,
            val sourceRevision: Long,
            val eventJson: String,
        )

        override fun isAvailable(): Boolean = available

        override fun describeViews(): String? {
            throwOnDescribe?.let { throw it }
            return if (available) viewsJson else null
        }

        override fun attachVideo(viewId: String, generation: String, surface: Any): String? {
            if (!available) return null
            attachCalls.add(Triple(viewId, generation, surface))
            val arr = JSONArray(viewsJson ?: "[]")
            for (i in 0 until arr.length()) {
                val view = arr.getJSONObject(i)
                if (view.getString("view_id") == viewId && view.getString("generation") == generation) {
                    val consumerId = "consumer-${nextConsumerSeq++}"
                    val record = JSONObject(view.toString()).put("consumer_id", consumerId)
                    activeConsumers[consumerId] = record
                    return JSONObject()
                        .put("ok", true)
                        .put("consumer_id", consumerId)
                        .put("view_id", viewId)
                        .put("display_id", view.getInt("display_id"))
                        .put("generation", generation)
                        .put("source_revision", view.getLong("source_revision"))
                        .put("resource_epoch", view.getLong("resource_epoch"))
                        .put("width", view.getInt("width"))
                        .put("height", view.getInt("height"))
                        .put("rotation", view.getInt("rotation"))
                        .toString()
                }
            }
            return JSONObject().put("ok", false).put("error", "view_not_found").toString()
        }

        override fun detachVideo(consumerId: String): String? {
            if (!available) return null
            detachCalls.add(consumerId)
            val removed = activeConsumers.remove(consumerId) != null
            return JSONObject().put("ok", removed).toString()
        }

        override fun dispatchInput(
            consumerId: String,
            viewId: String,
            generation: String,
            sourceRevision: Long,
            eventJson: String,
        ): String? {
            if (!available) return null
            dispatchCalls.add(DispatchRecord(consumerId, viewId, generation, sourceRevision, eventJson))
            val forcedError = dispatchError
            if (forcedError != null) {
                return JSONObject()
                    .put("ok", false)
                    .put("error", forcedError.first)
                    .apply { if (forcedError.second != null) put("source_revision", forcedError.second) }
                    .toString()
            }
            val active = activeConsumers[consumerId]
            if (active == null || active.getString("view_id") != viewId || active.getString("generation") != generation) {
                return JSONObject().put("ok", false).put("error", "stale_view_or_consumer").toString()
            }
            val currentRevision = active.getLong("source_revision")
            if (currentRevision != sourceRevision) {
                return JSONObject()
                    .put("ok", false)
                    .put("error", "source_revision_mismatch")
                    .put("source_revision", currentRevision)
                    .toString()
            }
            resourceEpoch += 1L
            return JSONObject()
                .put("ok", true)
                .put("accepted", true)
                .put("resource_epoch", resourceEpoch)
                .toString()
        }

        override fun releaseInput(consumerId: String): String? {
            if (!available) return null
            releaseCalls.add(consumerId)
            return JSONObject().put("ok", true).toString()
        }
    }

    private class FakePublisher(
        val width: Int,
        val height: Int,
        val rotation: Int,
    ) : AiVideoPublisher {
        override val inputSurface: Any = "fake-surface-${width}x$height-r$rotation-${System.nanoTime()}"
        override val pendingFrames: Boolean = false
        var updates = 0
        var finished = false
        var closed = false

        override fun update(views: JSONArray): Boolean {
            updates++
            return true
        }

        override fun awaitFrame(timeoutMs: Long) = Unit
        override fun finish() { finished = true }
        override fun close() { closed = true }
    }

    private class FakeBridge : AiBridge {
        val publishedCatalogs = mutableListOf<List<JSONObject>>()
        val configuredGeometries = mutableListOf<JSONObject>()
        val artworkUpdates = mutableListOf<Triple<Long, Long, Long>>()
        val failedViews = mutableListOf<Triple<Long, Long, String>>()
        var demandsJson: String = "[]"

        override fun applicationDemands(): String = demandsJson

        override fun configureGeometry(
            windowId: Long,
            grant: Long,
            epoch: Long,
            width: Int,
            height: Int,
            rotationDegrees: Int,
        ) {
            configuredGeometries.add(
                JSONObject()
                    .put("window_id", windowId)
                    .put("grant", grant)
                    .put("epoch", epoch)
                    .put("width", width)
                    .put("height", height)
                    .put("rotation", rotationDegrees)
            )
        }

        override fun setApplicationArtwork(windowId: Long, grant: Long, epoch: Long) {
            artworkUpdates.add(Triple(windowId, grant, epoch))
        }

        override fun failScreenView(stream: Long, grant: Long, reason: String) {
            failedViews.add(Triple(stream, grant, reason))
        }

        override fun publishCatalog(entries: List<JSONObject>) {
            publishedCatalogs.add(entries)
        }
    }

    private fun viewJson(
        displayId: Int = 5,
        generation: String = "gen-1",
        sourceRevision: Long = 1L,
        resourceEpoch: Long = 1L,
        pkg: String = "com.android.chrome",
        label: String = "Chrome",
        width: Int = 1080,
        height: Int = 1920,
        rotation: Int = 0,
        supportsInput: Boolean = true,
    ): JSONObject = JSONObject()
        .put("view_id", "display:$displayId")
        .put("display_id", displayId)
        .put("generation", generation)
        .put("source_revision", sourceRevision)
        .put("resource_epoch", resourceEpoch)
        .put("package", pkg)
        .put("label", label)
        .put("user_id", 0)
        .put("width", width)
        .put("height", height)
        .put("density", 440)
        .put("rotation", rotation)
        .put("human_active", false)
        .put("supports_input", supportsInput)

    private fun demandRow(
        windowId: Long,
        subscriptions: Int = 1,
        grant: Long = 77L,
        epoch: Long = 10L,
        artworkEpoch: Long = 1L,
        streams: List<Long> = listOf(101L),
    ): JSONObject {
        val views = JSONArray()
        for (stream in streams) {
            views.put(
                JSONObject()
                    .put("stream", stream)
                    .put("grant", grant)
                    .put("keyframe", 0L)
                    .put("fps", 60)
                    .put("bitrate_kbps", 8000)
                    .put(
                        "geometry",
                        JSONObject()
                            .put("width", 1080)
                            .put("height", 1920)
                            .put("source_width", 1080)
                            .put("source_height", 1920)
                            .put("crop_x", 0)
                            .put("crop_y", 0)
                            .put("crop_width", 1080)
                            .put("crop_height", 1920)
                            .put("revision", 1L),
                    ),
            )
        }
        return JSONObject()
            .put("source", JSONObject().put("Window", windowId))
            .put("artwork_epoch", artworkEpoch)
            .put(
                "demand",
                JSONObject()
                    .put("subscriptions", subscriptions)
                    .put("grant", grant)
                    .put("epoch", epoch)
                    .put("views", views),
            )
    }

    @Test
    fun managedFallbackPollsForAiOnlyAndRequiresControl() {
        assertTrue(ManagedFallbackInput.shouldPollInput(null, true, true))
        assertFalse(ManagedFallbackInput.shouldPollInput(null, false, true))
        assertFalse(ManagedFallbackInput.shouldPollInput(null, true, false))
        assertTrue(ManagedFallbackInput.shouldPollInput(1080 to 2400, false, false))
    }

    @Test
    fun certificateVerificationAllowsOwnerAndMatchingOwnCertAndDeniesOthers() {
        val owner = OctendraAiClient.OWNER_CERT_SHA256
        val own = sha256Hex("weavelo-dev-signing-cert".toByteArray())
        val rogue = sha256Hex("rogue-signer".toByteArray())

        assertEquals(64, own.length)
        assertTrue(isTrustedOctendraPeer("fun.hackerlife.octendra", listOf(owner), emptySet()))
        assertTrue(isTrustedOctendraPeer("fun.hackerlife.octendra", listOf(owner.uppercase()), emptySet()))
        assertTrue(isTrustedOctendraPeer("fun.hackerlife.octendra", listOf(own), setOf(own)))

        assertFalse(isTrustedOctendraPeer("fun.hackerlife.octendra", listOf(rogue), setOf(own)))
        assertFalse(isTrustedOctendraPeer("fun.hackerlife.spoof", listOf(owner), setOf(own)))
        assertFalse(isTrustedOctendraPeer(null, listOf(owner), setOf(own)))
        assertFalse(isTrustedOctendraPeer("fun.hackerlife.octendra", emptyList(), setOf(own)))
        assertFalse(isTrustedOctendraPeer("fun.hackerlife.octendra", listOf(owner, rogue), setOf(own)))
    }

    @Test
    fun unavailableOrMalformedOctendraDegradesGracefully() {
        val transport = FakeTransport()
        val client = OctendraAiClient(transport, logger = { _, _ -> })

        transport.available = false
        assertFalse(client.isAvailable())
        assertTrue(client.describeViews().isEmpty())
        assertFalse(client.attachVideo("display:1", "gen-1", "surf").ok)
        assertFalse(client.detachVideo("c1"))
        assertFalse(client.dispatchInput("c1", "display:1", "gen-1", 1L, "{}").ok)
        assertFalse(client.releaseInput("c1"))

        transport.available = true
        transport.throwOnDescribe = SecurityException("denied")
        assertTrue(client.describeViews().isEmpty())

        transport.throwOnDescribe = null
        transport.viewsJson = JSONArray()
            .put(JSONObject().put("view_id", "")) // invalid
            .put(viewJson(displayId = 2, rotation = 90))
            .toString()
        val parsed = client.describeViews()
        assertEquals(1, parsed.size)
        assertEquals(2, parsed[0].displayId)
        assertEquals(1, parsed[0].rotation)
        assertEquals(3, normalizeRotationQuadrant(270))
    }

    @Test
    fun reservedIdRangeAvoidsCollisionAndCoordinatorMergesCatalogs() {
        assertEquals(1329812480L, OctendraAiViews.AI_VIEW_ID_BASE)
        assertEquals(0x4F435400L, ManagedApplications.AI_VIEW_ID_BASE)
        assertEquals(0x4F435405L, OctendraAiViews.aiViewWindowId(5))

        // "com.test.app2515557" has raw CRC32 = 0x4f43569a inside 0x4F435400..=0x4F4357FF
        val collidingPkg = "com.test.app2515557"
        val rawCrc = CRC32().apply { update(collidingPkg.toByteArray(Charsets.UTF_8)) }.value
        assertTrue(rawCrc in ManagedApplications.AI_VIEW_ID_BASE..ManagedApplications.AI_VIEW_ID_RANGE_END)

        val mappedId = ManagedApplications.windowIdForPackage(collidingPkg)
        assertEquals(rawCrc + ManagedApplications.AI_VIEW_COLLISION_OFFSET, mappedId)
        assertFalse(mappedId in ManagedApplications.AI_VIEW_ID_BASE..ManagedApplications.AI_VIEW_ID_RANGE_END)

        val normalRaw = CRC32().apply { update("com.android.settings".toByteArray(Charsets.UTF_8)) }.value
        assertEquals(normalRaw, ManagedApplications.windowIdForPackage("com.android.settings"))

        // Catalog coordinator merges managed apps + AI views without dropping either.
        val published = mutableListOf<String>()
        val coordinator = WindowCatalogCoordinator { published.add(it) }
        val appEntry = JSONObject().put("source", JSONObject().put("Window", mappedId)).put("title", "Test App")
        val aiEntry = JSONObject().put("source", JSONObject().put("Window", 0x4F435405L)).put("title", "AI 工作区 · Chrome")

        coordinator.updateManaged(listOf(appEntry))
        assertEquals(1, published.size)
        coordinator.updateAi(listOf(aiEntry))
        assertEquals(2, published.size)
        val merged = JSONArray(published.last())
        assertEquals(2, merged.length())

        // Duplicate identical AI update should not trigger redundant action(43).
        coordinator.updateAi(listOf(aiEntry))
        assertEquals(2, published.size)

        // Removing AI views preserves managed app.
        coordinator.updateAi(emptyList())
        assertEquals(3, published.size)
        val onlyManaged = JSONArray(published.last())
        assertEquals(1, onlyManaged.length())
        assertEquals(mappedId, onlyManaged.getJSONObject(0).getJSONObject("source").getLong("Window"))
    }

    @Test
    fun aiCatalogPublishesCapabilityChangesWithoutResubscription() {
        val transport = FakeTransport()
        transport.viewsJson = JSONArray().put(viewJson(displayId = 5, generation = "g", supportsInput = false)).toString()
        val bridge = FakeBridge()
        val views = OctendraAiViews(OctendraAiClient(transport, logger = { _, _ -> }), bridge,
            publisherFactory = AiVideoPublisherFactory { w, h, r -> FakePublisher(w, h, r) },
            clockMs = { 1000L }, threaded = false, logger = { _, _ -> })
        views.use {
            it.update()
            assertFalse(bridge.publishedCatalogs.last().single().getBoolean("supports_input"))
            transport.viewsJson = JSONArray().put(viewJson(displayId = 5, generation = "g", supportsInput = true)).toString()
            it.requestRefresh(); it.update()
            assertTrue(bridge.publishedCatalogs.last().single().getBoolean("supports_input"))
            assertEquals(2, bridge.publishedCatalogs.size)
        }
    }

    @Test
    fun attachDetachLifecyclePreservesAiViewAndSeparatesStreams() {
        val transport = FakeTransport()
        transport.viewsJson = JSONArray().put(viewJson(displayId = 5, generation = "gen-1")).toString()
        val client = OctendraAiClient(transport, logger = { _, _ -> })
        val bridge = FakeBridge()
        val publishers = mutableListOf<FakePublisher>()
        var nowMs = 1000L

        val views = OctendraAiViews(
            client = client,
            bridge = bridge,
            publisherFactory = AiVideoPublisherFactory { w, h, r ->
                FakePublisher(w, h, r).also { publishers.add(it) }
            },
            clockMs = { nowMs },
            threaded = false,
            attachToGlobalRegistry = true,
            logger = { _, _ -> },
        )

        views.use {
            it.update()
            assertEquals(1, bridge.publishedCatalogs.size)
            val catalogItem = bridge.publishedCatalogs[0].single()
            assertEquals(0x4F435405L, catalogItem.getJSONObject("source").getLong("Window"))
            assertEquals("AI 工作区 · Chrome", catalogItem.getString("title"))
            assertEquals("Octendra AI", catalogItem.getString("application"))

            // Viewer subscribes to the AI view.
            bridge.demandsJson = JSONArray().put(demandRow(0x4F435405L, subscriptions = 1)).toString()
            it.update()
            it.stepWorkersForTest()

            assertEquals(1, publishers.size)
            assertEquals(1, transport.attachCalls.size)
            assertEquals(1, publishers[0].updates)
            assertEquals(1, bridge.configuredGeometries.size)
            assertEquals(0, bridge.configuredGeometries[0].getInt("rotation"))

            val targetSource = JSONObject().put("Window", 0x4F435405L)
            val target = OctendraAiViews.target(targetSource)
            assertNotNull(target)
            assertEquals("consumer-1", target!!.consumerId)
            assertEquals("gen-1", target.generation)
            assertTrue(OctendraAiViews.hasActiveTargets())

            // Viewer unsubscribes: detaches consumer surface WITHOUT closing Octendra's AI view.
            bridge.demandsJson = JSONArray().put(demandRow(0x4F435405L, subscriptions = 0)).toString()
            it.update()

            assertEquals(listOf("consumer-1"), transport.detachCalls)
            assertTrue(publishers[0].closed)
            assertNull(OctendraAiViews.target(targetSource))
            assertFalse(OctendraAiViews.hasActiveTargets())
            assertEquals(1, client.describeViews().size)
        }
    }

    @Test
    fun generationSwitchInvalidatesOldTargetAndRebindsCleanly() {
        val transport = FakeTransport()
        transport.viewsJson = JSONArray().put(viewJson(displayId = 3, generation = "gen-1", width = 1080, height = 1920)).toString()
        val client = OctendraAiClient(transport, logger = { _, _ -> })
        val bridge = FakeBridge()
        val publishers = mutableListOf<FakePublisher>()
        var nowMs = 1000L

        val views = OctendraAiViews(
            client = client,
            bridge = bridge,
            publisherFactory = AiVideoPublisherFactory { w, h, r ->
                FakePublisher(w, h, r).also { publishers.add(it) }
            },
            clockMs = { nowMs },
            pollIntervalMs = 500L,
            threaded = false,
            attachToGlobalRegistry = true,
            logger = { _, _ -> },
        )

        views.use {
            val windowId = 0x4F435403L
            val source = JSONObject().put("Window", windowId)
            bridge.demandsJson = JSONArray().put(demandRow(windowId, subscriptions = 1)).toString()
            it.update()
            it.stepWorkersForTest()

            val target1 = OctendraAiViews.target(source)!!
            val oldInput = OctendraAiInput(target1, client)
            assertTrue(oldInput.execute(JSONObject().put("Text", "hello")))

            // Switch generation & dimensions on Octendra side.
            nowMs += 600L
            transport.viewsJson = JSONArray()
                .put(viewJson(displayId = 3, generation = "gen-2", width = 1920, height = 1080, sourceRevision = 2L))
                .toString()

            // First poll immediately invalidates old target in registry.
            it.update()
            assertNull(OctendraAiViews.target(source))

            // Worker step detaches consumer-1, closes old publisher, attaches gen-2 with new publisher.
            it.stepWorkersForTest()
            assertEquals(listOf("consumer-1"), transport.detachCalls)
            assertTrue(publishers[0].closed)
            assertEquals(2, publishers.size)
            assertEquals(1920, publishers[1].width)
            assertEquals(1080, publishers[1].height)

            val target2 = OctendraAiViews.target(source)!!
            assertEquals("consumer-2", target2.consumerId)
            assertEquals("gen-2", target2.generation)
            assertEquals(2L, target2.sourceRevision)

            // Old input fails cleanly against consumer-1/gen-1 without touching gen-2.
            assertFalse(oldInput.execute(JSONObject().put("Text", "stale")))

            val newInput = OctendraAiInput(target2, client)
            assertTrue(newInput.execute(JSONObject().put("Text", "fresh")))
        }
    }

    @Test
    fun humanTakeoverRoutesTextAndMultiTouchAndReleasesInputCleanly() {
        // Verify RemoteControlService polling gate does NOT block AI views when main controlSize is null.
        assertTrue(
            RemoteControlService.shouldPollInput(
                serviceConnected = true,
                controlSize = null,
                aiControlEnabled = true,
                aiHasActiveTargets = true,
            )
        )
        assertFalse(
            RemoteControlService.shouldPollInput(
                serviceConnected = true,
                controlSize = null,
                aiControlEnabled = false,
                aiHasActiveTargets = true,
            )
        )
        assertFalse(
            RemoteControlService.shouldPollInput(
                serviceConnected = false,
                controlSize = Pair(1080, 1920),
                aiControlEnabled = true,
                aiHasActiveTargets = true,
            )
        )

        val transport = FakeTransport()
        transport.viewsJson = JSONArray()
            .put(viewJson(displayId = 4, generation = "gen-x", sourceRevision = 7L, supportsInput = true))
            .toString()
        val client = OctendraAiClient(transport, logger = { _, _ -> })
        val attach = client.attachVideo("display:4", "gen-x", "fake-surface")
        assertTrue(attach.ok)

        val target = OctendraAiViews.Target(
            windowId = 0x4F435404L,
            viewId = attach.viewId,
            displayId = attach.displayId,
            generation = attach.generation,
            sourceRevision = attach.sourceRevision,
            consumerId = attach.consumerId,
            width = attach.width,
            height = attach.height,
            rotation = attach.rotation,
            supportsInput = true,
        )

        val fallbackInput = OctendraAiInput(target, client)
        val fallbackEvents = JSONArray().put(JSONObject().put("Text", "fallback"))
        assertTrue(ManagedFallbackInput.dispatchAiEvents(fallbackEvents, fallbackInput) { true })
        assertEquals("fallback", JSONObject(transport.dispatchCalls.last().eventJson).getString("Text"))
        assertFalse(ManagedFallbackInput.dispatchAiEvents(fallbackEvents, fallbackInput) { false })
        fallbackInput.release()
        transport.dispatchCalls.clear()
        transport.releaseCalls.clear()

        val staleReports = mutableListOf<Pair<OctendraAiViews.Target, Boolean>>()
        val input = OctendraAiInput(target, client) { stale, forceRebind ->
            staleReports.add(stale to forceRebind)
        }

        // Committed UTF-8 text (including Chinese & emoji) routes through dispatch_input.
        assertTrue(input.execute(JSONObject().put("Text", "你好世界🙂")))
        // Multi-touch TouchFrame routes through dispatch_input.
        val touchFrame = JSONArray()
            .put(JSONObject().put("action", "Down").put("pointer_id", 0).put("normalized_x", 0.25).put("normalized_y", 0.5).put("pressure", 1.0))
            .put(JSONObject().put("action", "Down").put("pointer_id", 1).put("normalized_x", 0.75).put("normalized_y", 0.5).put("pressure", 1.0))
        assertTrue(input.execute(JSONObject().put("TouchFrame", touchFrame)))
        // Key event preserves macOS key_code and adds android_key_code.
        assertTrue(input.execute(JSONObject().put("Key", JSONObject().put("key_code", 0x24).put("pressed", true).put("modifiers", 0))))
        // MouseMoveAbsolute & MouseDown/Up route cleanly too.
        assertTrue(input.execute(JSONObject().put("MouseMoveAbsolute", JSONObject().put("x", 32768).put("y", 16384))))
        assertTrue(input.execute(JSONObject().put("MouseDown", 0)))
        assertTrue(input.execute(JSONObject().put("MouseUp", 0)))

        assertEquals(6, transport.dispatchCalls.size)
        assertEquals("你好世界🙂", JSONObject(transport.dispatchCalls[0].eventJson).getString("Text"))
        assertEquals(2, JSONObject(transport.dispatchCalls[1].eventJson).getJSONArray("TouchFrame").length())
        val sentKey = JSONObject(transport.dispatchCalls[2].eventJson).getJSONObject("Key")
        assertEquals(0x24, sentKey.getInt("key_code"))
        assertEquals(66, sentKey.getInt("android_key_code"))

        // Empty or oversized text (>1024 UTF-8 bytes) is rejected locally without calling transport.
        assertFalse(input.execute(JSONObject().put("Text", "")))
        assertFalse(input.execute(JSONObject().put("Text", "a".repeat(1025))))
        assertEquals(6, transport.dispatchCalls.size)

        // Source revision mismatch invalidates input and notifies listener.
        transport.dispatchError = "source_revision_mismatch" to 8L
        assertFalse(input.execute(JSONObject().put("Text", "after-resize")))
        assertEquals(listOf(target to false), staleReports)
        assertFalse(input.isActive)

        // Idempotent releaseInput cleanup on disconnect / switch / reset.
        input.release()
        input.release()
        assertEquals(listOf(attach.consumerId), transport.releaseCalls)
    }
}
