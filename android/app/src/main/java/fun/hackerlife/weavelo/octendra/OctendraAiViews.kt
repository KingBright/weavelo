package `fun`.hackerlife.weavelo.octendra

import android.os.SystemClock
import `fun`.hackerlife.weavelo.GpuScreenPublisher
import `fun`.hackerlife.weavelo.PublisherClient
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean

internal class WindowCatalogCoordinator(
    private val publishAction: (String) -> Unit = { payload ->
        PublisherClient.action(43, payload)
    },
) {
    private val lock = Any()
    private var managedEntries: List<JSONObject> = emptyList()
    private var aiEntries: List<JSONObject> = emptyList()
    private var lastPublishedJson: String? = null

    fun updateManaged(entries: List<JSONObject>) {
        synchronized(lock) {
            managedEntries = entries
            publishLocked()
        }
    }

    fun updateAi(entries: List<JSONObject>) {
        synchronized(lock) {
            aiEntries = entries
            publishLocked()
        }
    }

    fun close() {
        synchronized(lock) {
            managedEntries = emptyList()
            aiEntries = emptyList()
            if (lastPublishedJson != null && lastPublishedJson != "[]") {
                runCatching { publishAction("[]") }
                lastPublishedJson = "[]"
            }
        }
    }

    private fun publishLocked() {
        val merged = JSONArray()
        val seenIds = HashSet<Long>(managedEntries.size + aiEntries.size)
        for (entry in aiEntries) {
            if (merged.length() >= 256) break
            val id = entry.optJSONObject("source")?.optLong("Window", -1L) ?: -1L
            if (id > 0L && seenIds.add(id)) {
                merged.put(entry)
            }
        }
        for (entry in managedEntries) {
            if (merged.length() >= 256) break
            val id = entry.optJSONObject("source")?.optLong("Window", -1L) ?: -1L
            if (id > 0L && seenIds.add(id)) {
                merged.put(entry)
            }
        }
        val payload = merged.toString()
        if (lastPublishedJson == null && merged.length() == 0) return
        if (payload != lastPublishedJson) {
            publishAction(payload)
            lastPublishedJson = payload
        }
    }
}

internal interface AiVideoPublisher : AutoCloseable {
    val inputSurface: Any
    val pendingFrames: Boolean
    fun update(views: JSONArray): Boolean
    fun awaitFrame(timeoutMs: Long)
    fun finish()
}

internal fun interface AiVideoPublisherFactory {
    fun create(width: Int, height: Int, rotation: Int): AiVideoPublisher
}

internal interface AiBridge {
    fun applicationDemands(): String
    fun configureGeometry(windowId: Long, grant: Long, epoch: Long, width: Int, height: Int, rotationDegrees: Int)
    fun setApplicationArtwork(windowId: Long, grant: Long, epoch: Long)
    fun failScreenView(stream: Long, grant: Long, reason: String)
    fun publishCatalog(entries: List<JSONObject>)
}

internal object ProductionVideoPublisherFactory : AiVideoPublisherFactory {
    override fun create(width: Int, height: Int, rotation: Int): AiVideoPublisher {
        val gpu = GpuScreenPublisher(width, height, rotation)
        return object : AiVideoPublisher {
            override val inputSurface: Any get() = gpu.input
            override val pendingFrames: Boolean get() = gpu.pendingFrames
            override fun update(views: JSONArray): Boolean = gpu.update(views)
            override fun awaitFrame(timeoutMs: Long) = gpu.awaitFrame(timeoutMs)
            override fun finish() = gpu.finish()
            override fun close() = gpu.close()
        }
    }
}

internal class ProductionAiBridge(
    private val catalogCoordinator: WindowCatalogCoordinator,
) : AiBridge {
    override fun applicationDemands(): String = PublisherClient.action(44)

    override fun configureGeometry(
        windowId: Long,
        grant: Long,
        epoch: Long,
        width: Int,
        height: Int,
        rotationDegrees: Int,
    ) {
        PublisherClient.action(
            3,
            JSONObject()
                .put("source", JSONObject().put("Window", windowId))
                .put("grant", grant)
                .put("epoch", epoch)
                .put("width", width)
                .put("height", height)
                .put("rotation", rotationDegrees)
                .toString(),
        )
    }

    override fun setApplicationArtwork(windowId: Long, grant: Long, epoch: Long) {
        PublisherClient.action(
            45,
            JSONObject()
                .put("source", JSONObject().put("Window", windowId))
                .put("grant", grant)
                .put("epoch", epoch)
                .put("icon", JSONObject.NULL)
                .toString(),
        )
    }

    override fun failScreenView(stream: Long, grant: Long, reason: String) {
        PublisherClient.action(
            36,
            JSONObject()
                .put("stream", stream)
                .put("grant", grant)
                .put("reason", reason.take(200))
                .toString(),
        )
    }

    override fun publishCatalog(entries: List<JSONObject>) {
        catalogCoordinator.updateAi(entries)
    }
}

/**
 * Discovers active Octendra AI background views, exposes them as `CaptureSource::Window` entries
 * in both managed and MediaProjection modes, and streams each subscribed view via zero-copy
 * `GpuScreenPublisher` -> `attach_video` without closing Octendra's AI task on viewer detach.
 */
internal class OctendraAiViews(
    private val client: OctendraAiClient,
    private val bridge: AiBridge,
    private val publisherFactory: AiVideoPublisherFactory = ProductionVideoPublisherFactory,
    private val clockMs: () -> Long = SystemClock::elapsedRealtime,
    private val pollIntervalMs: Long = 500L,
    private val threaded: Boolean = true,
    private val attachToGlobalRegistry: Boolean = true,
    private val logger: (String, Throwable?) -> Unit = { msg, err ->
        android.util.Log.w("WeaveloOctendra", msg, err)
    },
) : AutoCloseable {

    data class Target(
        val windowId: Long,
        val viewId: String,
        val displayId: Int,
        val generation: String,
        val sourceRevision: Long,
        val consumerId: String,
        val width: Int,
        val height: Int,
        val rotation: Int,
        val supportsInput: Boolean,
    )

    companion object {
        const val AI_VIEW_ID_BASE = 0x4F435400L // 1329812480L
        const val AI_VIEW_ID_RANGE_END = 0x4F4357FFL
        const val AI_APPLICATION_NAME = "Octendra AI"
        private const val TITLE_PREFIX = "AI 工作区 · "
        private const val MAX_TITLE_UTF8_BYTES = 1024
        private const val MAX_WORKERS = 8

        private val targets = ConcurrentHashMap<Long, Target>()
        @Volatile private var activeInstance: OctendraAiViews? = null
        @Volatile private var activeClient: OctendraAiClient? = null
        @Volatile var controlEnabled: Boolean = false
            internal set

        fun target(source: Any?): Target? {
            val obj = source as? JSONObject ?: return null
            if (!obj.has("Window")) return null
            val id = obj.optLong("Window", -1L)
            return if (id > 0L) targets[id] else null
        }

        fun hasActiveTargets(): Boolean = targets.isNotEmpty()

        fun clientOrNull(): OctendraAiClient? = activeClient

        fun invalidateTarget(target: Target, forceRebind: Boolean = false) {
            targets.remove(target.windowId, target)
            activeInstance?.onTargetInvalidated(target, forceRebind)
        }

        fun aiViewWindowId(displayId: Int): Long? {
            if (displayId < 0) return null
            val id = AI_VIEW_ID_BASE + displayId.toLong()
            return if (id <= 0xFFFFFFFFL) id else null
        }

        fun aiViewTitle(descriptor: AiViewDescriptor): String {
            val label = descriptor.label.ifBlank { descriptor.packageName }.ifBlank { "Display ${descriptor.displayId}" }
            return clampUtf8("$TITLE_PREFIX$label", MAX_TITLE_UTF8_BYTES)
        }

        private fun clampUtf8(value: String, maxBytes: Int): String {
            if (value.toByteArray(Charsets.UTF_8).size <= maxBytes) return value
            val sb = StringBuilder()
            var used = 0
            var i = 0
            while (i < value.length) {
                val cp = value.codePointAt(i)
                val chars = Character.toChars(cp)
                val charCount = chars.size
                val byteCount = String(chars).toByteArray(Charsets.UTF_8).size
                if (used + byteCount > maxBytes) break
                sb.append(chars)
                used += byteCount
                i += charCount
            }
            return sb.toString()
        }
    }

    private data class CatalogSignature(
        val windowId: Long,
        val title: String,
        val width: Int,
        val height: Int,
    )

    private val closed = AtomicBoolean(false)
    private val forceRefresh = AtomicBoolean(true)
    private var lastPollAtMs = Long.MIN_VALUE / 4
    private var latestViews: Map<Long, AiViewDescriptor> = emptyMap()
    private var publishedSignature: List<CatalogSignature>? = null

    private val workers = mutableMapOf<Long, Worker>()
    private val failed = mutableSetOf<Long>()
    private val localTargets = ConcurrentHashMap<Long, Target>()

    init {
        if (attachToGlobalRegistry) {
            activeInstance = this
            activeClient = client
        }
    }

    fun requestRefresh() {
        forceRefresh.set(true)
    }

    fun currentTarget(windowId: Long): Target? = localTargets[windowId]

    internal fun onTargetInvalidated(target: Target, forceRebind: Boolean) {
        localTargets.remove(target.windowId, target)
        workers[target.windowId]?.markInvalidated(target, forceRebind)
        forceRefresh.set(true)
    }

    fun update() {
        if (closed.get()) return
        val now = clockMs()
        if (forceRefresh.getAndSet(false) || now - lastPollAtMs >= pollIntervalMs) {
            lastPollAtMs = now
            refreshViews()
        }
        if (latestViews.isEmpty() && workers.isEmpty()) return

        val list = runCatching { JSONArray(bridge.applicationDemands()) }.getOrElse { error ->
            logger("Failed to poll Octendra AI view demands", error)
            return
        }
        val subscribed = mutableSetOf<Long>()
        val streams = mutableSetOf<Long>()
        for (i in 0 until list.length()) {
            val row = list.getJSONObject(i)
            val id = row.getJSONObject("source").getLong("Window")
            val demand = row.getJSONObject("demand")
            demand.put("artwork_epoch", row.getLong("artwork_epoch"))
            val allViews = demand.optJSONArray("views") ?: JSONArray()
            val views = JSONArray()
            for (index in 0 until allViews.length()) {
                val view = allViews.getJSONObject(index)
                val stream = view.getLong("stream")
                streams.add(stream)
                if (stream !in failed) views.put(view)
            }
            demand.put("views", views)
            if (id !in latestViews && id !in workers) continue
            if (demand.optInt("subscriptions") == 0) continue
            subscribed.add(id)
            val descriptor = latestViews[id]
            val existing = workers[id]
            val worker = if (existing != null) {
                existing
            } else if (views.length() > 0) {
                if (descriptor == null) {
                    fail(demand, "AI view is no longer available")
                    continue
                }
                if (workers.size >= MAX_WORKERS) {
                    fail(demand, "Eight AI views are already open")
                    continue
                }
                Worker(id, descriptor, demand).also {
                    workers[id] = it
                    if (threaded) it.start()
                }
            } else {
                continue
            }
            worker.demand = demand
            worker.descriptor = descriptor
            worker.error?.let { err ->
                fail(demand, err)
                workers.remove(id)?.close()
            }
        }
        for (id in workers.keys.toList()) {
            if (id !in subscribed) {
                workers.remove(id)?.close()
            }
        }
        failed.retainAll(streams)
    }

    internal fun stepWorkersForTest() {
        for (id in workers.keys.toList()) {
            val worker = workers[id] ?: continue
            val keepGoing = runCatching { worker.step(allowSleep = false) }.getOrElse { failure ->
                worker.error = failure.cause?.message ?: failure.message ?: "AI view stopped"
                false
            }
            if (!keepGoing) {
                worker.cleanupAttachment()
            }
            worker.error?.let { err ->
                fail(worker.demand, err)
                workers.remove(id)?.close()
            }
        }
    }

    private fun refreshViews() {
        val described = client.describeViews()
        val nextViews = linkedMapOf<Long, AiViewDescriptor>()
        val entries = ArrayList<JSONObject>(described.size)
        val signatures = ArrayList<CatalogSignature>(described.size)
        for (view in described.sortedBy { it.displayId }) {
            val windowId = aiViewWindowId(view.displayId) ?: continue
            if (nextViews.containsKey(windowId)) continue
            nextViews[windowId] = view
            val title = aiViewTitle(view)
            signatures.add(CatalogSignature(windowId, title, view.width, view.height))
            entries.add(
                JSONObject()
                    .put("source", JSONObject().put("Window", windowId))
                    .put("title", title)
                    .put("application", AI_APPLICATION_NAME)
                    .put("process_id", JSONObject.NULL)
                    .put("width", view.width)
                    .put("height", view.height)
                    .put("supports_input", false)
                    .put("camera", JSONObject.NULL)
            )
        }
        latestViews = nextViews

        for ((id, worker) in workers) {
            val nextDesc = nextViews[id]
            worker.descriptor = nextDesc
            val active = localTargets[id]
            if (active != null && (nextDesc == null ||
                    nextDesc.generation != active.generation ||
                    nextDesc.width != active.width ||
                    nextDesc.height != active.height ||
                    nextDesc.rotation != active.rotation ||
                    nextDesc.sourceRevision != active.sourceRevision ||
                    nextDesc.supportsInput != active.supportsInput)
            ) {
                removeTarget(id, active)
            }
        }

        val currentSig = signatures
        if (publishedSignature == null && currentSig.isEmpty()) return
        if (publishedSignature != currentSig) {
            bridge.publishCatalog(entries)
            publishedSignature = currentSig
        }
    }

    private fun setTarget(target: Target) {
        localTargets[target.windowId] = target
        if (attachToGlobalRegistry) {
            targets[target.windowId] = target
        }
    }

    private fun removeTarget(windowId: Long, expected: Target) {
        localTargets.remove(windowId, expected)
        if (attachToGlobalRegistry) {
            targets.remove(windowId, expected)
        }
    }

    private fun fail(demand: JSONObject, reason: String) {
        val views = demand.optJSONArray("views") ?: JSONArray()
        for (index in 0 until views.length()) {
            val view = views.getJSONObject(index)
            val stream = view.getLong("stream")
            failed.add(stream)
            runCatching { bridge.failScreenView(stream, view.getLong("grant"), reason) }
        }
    }

    private inner class Worker(
        private val windowId: Long,
        initialDescriptor: AiViewDescriptor,
        initialDemand: JSONObject,
    ) : AutoCloseable {
        private val alive = AtomicBoolean(true)
        private val rebindRequested = AtomicBoolean(false)
        @Volatile var demand: JSONObject = initialDemand
        @Volatile var descriptor: AiViewDescriptor? = initialDescriptor
        @Volatile var error: String? = null
        @Volatile private var invalidatedTarget: Target? = null

        private var publisher: AiVideoPublisher? = null
        private var attachedConsumerId: String? = null
        private var boundAttach: AttachResult? = null
        private var configuredGeometry = false
        private var configuredTarget: Target? = null
        private var artworkEpoch = -1L
        private var attachFailures = 0

        private val thread = Thread({ runLoop() }, "weavelo-ai-$windowId")

        fun start() = thread.start()

        fun stop() {
            alive.set(false)
            thread.interrupt()
        }

        fun markInvalidated(target: Target, forceRebind: Boolean) {
            invalidatedTarget = target
            if (forceRebind) rebindRequested.set(true)
        }

        private fun runLoop() {
            try {
                while (alive.get()) {
                    if (!step(allowSleep = true)) break
                }
            } catch (_: InterruptedException) {
                // Closing cleanly.
            } catch (failure: Exception) {
                if (alive.get()) {
                    error = failure.cause?.message ?: failure.message ?: "AI view stopped"
                }
            } finally {
                cleanupAttachment()
            }
        }

        fun step(allowSleep: Boolean): Boolean {
            if (!alive.get()) return false
            val currentDemand = demand
            val currentDesc = descriptor
            if (currentDesc == null) {
                cleanupAttachment()
                error = "AI view closed"
                return false
            }

            val nextArtworkEpoch = currentDemand.optLong("artwork_epoch", -1L)
            if (nextArtworkEpoch >= 0L && artworkEpoch != nextArtworkEpoch) {
                artworkEpoch = nextArtworkEpoch
                runCatching {
                    bridge.setApplicationArtwork(windowId, currentDemand.getLong("grant"), nextArtworkEpoch)
                }.onFailure {
                    logger("AI view artwork update unavailable", it)
                }
            }

            val views = currentDemand.optJSONArray("views") ?: JSONArray()
            if (views.length() == 0) {
                cleanupAttachment()
                if (allowSleep) Thread.sleep(16L)
                return true
            }

            val currentAttach = boundAttach
            if (rebindRequested.getAndSet(false) || (currentAttach != null && (
                    currentAttach.generation != currentDesc.generation ||
                        currentAttach.width != currentDesc.width ||
                        currentAttach.height != currentDesc.height ||
                        currentAttach.rotation != currentDesc.rotation
                    ))
            ) {
                cleanupAttachment()
            }

            if (publisher == null || attachedConsumerId == null) {
                val nextPub = publisherFactory.create(currentDesc.width, currentDesc.height, currentDesc.rotation)
                val attach = client.attachVideo(currentDesc.viewId, currentDesc.generation, nextPub.inputSurface)
                if (!attach.ok) {
                    runCatching { nextPub.close() }
                    attachFailures++
                    forceRefresh.set(true)
                    if (attachFailures >= 5) {
                        error = attach.error ?: "Unable to attach Octendra AI view"
                        return false
                    }
                    if (allowSleep) Thread.sleep(50L)
                    return true
                }
                if (attach.width != currentDesc.width ||
                    attach.height != currentDesc.height ||
                    attach.rotation != currentDesc.rotation
                ) {
                    runCatching { client.detachVideo(attach.consumerId) }
                    runCatching { nextPub.close() }
                    descriptor = currentDesc.copy(
                        width = attach.width,
                        height = attach.height,
                        rotation = attach.rotation,
                        sourceRevision = maxOf(currentDesc.sourceRevision, attach.sourceRevision),
                    )
                    forceRefresh.set(true)
                    return true
                }
                attachFailures = 0
                publisher = nextPub
                attachedConsumerId = attach.consumerId
                boundAttach = attach
                configuredGeometry = false
                configuredTarget = null
            }

            val attach = checkNotNull(boundAttach)
            if (!configuredGeometry) {
                val configured = runCatching {
                    bridge.configureGeometry(
                        windowId = windowId,
                        grant = currentDemand.getLong("grant"),
                        epoch = currentDemand.getLong("epoch"),
                        width = attach.width,
                        height = attach.height,
                        rotationDegrees = attach.rotation * 90,
                    )
                }
                configured.exceptionOrNull()?.let { failure ->
                    val msg = failure.message.orEmpty()
                    if (!msg.contains("Capture configuration changed")) throw failure
                }
                if (configured.isFailure) {
                    if (allowSleep) Thread.sleep(16L)
                    return true
                }
                configuredGeometry = true
                val newTarget = Target(
                    windowId = windowId,
                    viewId = attach.viewId,
                    displayId = attach.displayId,
                    generation = attach.generation,
                    sourceRevision = maxOf(attach.sourceRevision, currentDesc.sourceRevision),
                    consumerId = attach.consumerId,
                    width = attach.width,
                    height = attach.height,
                    rotation = attach.rotation,
                    supportsInput = currentDesc.supportsInput,
                )
                configuredTarget = newTarget
                invalidatedTarget = null
                setTarget(newTarget)
            } else {
                val existingTarget = configuredTarget
                val expectedRevision = maxOf(attach.sourceRevision, currentDesc.sourceRevision)
                if (existingTarget != null) {
                    val updatedTarget = existingTarget.copy(
                        sourceRevision = expectedRevision,
                        supportsInput = currentDesc.supportsInput,
                    )
                    val shouldPublish = existingTarget != updatedTarget ||
                        (currentTarget(windowId) != updatedTarget && invalidatedTarget != updatedTarget)
                    if (shouldPublish) {
                        removeTarget(windowId, existingTarget)
                        configuredTarget = updatedTarget
                        invalidatedTarget = null
                        setTarget(updatedTarget)
                    }
                }
            }

            val pub = checkNotNull(publisher)
            pub.update(views)
            if (allowSleep) pub.awaitFrame(if (pub.pendingFrames) 1L else 16L)
            return true
        }

        fun cleanupAttachment() {
            configuredTarget?.let { removeTarget(windowId, it) }
            configuredTarget = null
            configuredGeometry = false
            boundAttach = null
            val consumer = attachedConsumerId
            attachedConsumerId = null
            if (consumer != null) {
                runCatching { client.detachVideo(consumer) }
            }
            val pub = publisher
            publisher = null
            if (pub != null) {
                runCatching { pub.finish() }
                runCatching { pub.close() }
            }
        }

        override fun close() {
            stop()
            if (threaded) {
                val interrupted = Thread.interrupted()
                try {
                    thread.join(2500L)
                } finally {
                    if (interrupted) Thread.currentThread().interrupt()
                }
            } else {
                cleanupAttachment()
            }
        }
    }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        workers.values.forEach { it.stop() }
        workers.values.forEach { runCatching { it.close() } }
        workers.clear()
        for ((id, target) in localTargets.entries) {
            if (attachToGlobalRegistry) targets.remove(id, target)
        }
        localTargets.clear()
        latestViews = emptyMap()
        if (publishedSignature != null && publishedSignature!!.isNotEmpty()) {
            runCatching { bridge.publishCatalog(emptyList()) }
        }
        publishedSignature = emptyList()
        if (attachToGlobalRegistry) {
            if (activeInstance === this) activeInstance = null
            if (activeClient === client) activeClient = null
        }
    }
}
