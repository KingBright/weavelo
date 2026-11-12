package `fun`.hackerlife.weavelo.managed

import `fun`.hackerlife.weavelo.GpuScreenPublisher
import `fun`.hackerlife.weavelo.PublisherClient
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean
import java.util.zip.CRC32

/** App identity, display ownership and capture live in Weavelo on this phone. */
@android.annotation.TargetApi(34)
internal class ManagedApplications(private val host: IManagedScreen, private val context: Context) : AutoCloseable {
    internal data class Target(val host: IManagedScreen, val displayId: Int,
        val packageName: String, val width: Int, val height: Int, val rotation: Int = 0)
    companion object {
        private val targets = ConcurrentHashMap<Long, Target>()
        fun target(source: Any?): Target? = (source as? JSONObject)?.optLong("Window")?.let(targets::get)
    }
    private data class App(val id: Long, val packageName: String, val name: String)
    private val dimensions = JSONObject(host.info())
    private val width = minOf(dimensions.getInt("width"), dimensions.getInt("height"))
    private val height = maxOf(dimensions.getInt("width"), dimensions.getInt("height"))
    private val density = dimensions.getInt("density")
    private var apps: Map<Long, App> = emptyMap()
    private val catalogChanged=AtomicBoolean(false)
    private val packageUpdates=object: BroadcastReceiver() {
        override fun onReceive(context:Context,intent:Intent) {catalogChanged.set(true)}
    }
    private val workers = mutableMapOf<Long, Capture>()
    private val failed = mutableSetOf<Long>()
    private val iconJobs = java.util.concurrent.LinkedBlockingQueue<Map<Long,App>>(1)
    private val iconResult = java.util.concurrent.atomic.AtomicReference<Pair<Map<Long,App>,JSONArray>?>()
    private val iconsAlive = AtomicBoolean(true)
    private val iconThread = Thread({
        try {
            while (iconsAlive.get()) {
                val catalog = iconJobs.take()
                val icons = JSONArray()
                for (app in catalog.values) {
                    if (!iconsAlive.get() || iconJobs.isNotEmpty()) break
                    runCatching { icon(host.applicationIcon(app.packageName,64)) }
                        .onSuccess { icons.put(JSONArray().put(JSONObject().put("Window",app.id)).put(it)) }
                        .onFailure { android.util.Log.w("Weavelo","Catalog icon unavailable: ${app.packageName}",it) }
                }
                if (iconsAlive.get() && iconJobs.isEmpty()) iconResult.set(catalog to icons)
            }
        } catch (_: InterruptedException) { /* closing */ }
    }, "weavelo-app-icons")
    private fun icon(png:ByteArray):JSONObject {
        val digest=java.security.MessageDigest.getInstance("SHA-256").digest(png)
        return JSONObject().put("id",JSONArray(digest.map { it.toInt() and 255 }))
            .put("png",JSONArray(png.map { it.toInt() and 255 }))
    }
    init {
        refreshCatalog()
        context.registerReceiver(packageUpdates,IntentFilter().apply {
            addAction(Intent.ACTION_PACKAGE_ADDED);addAction(Intent.ACTION_PACKAGE_REMOVED)
            addAction(Intent.ACTION_PACKAGE_CHANGED);addDataScheme("package")
        },Context.RECEIVER_NOT_EXPORTED)
        iconThread.start()
    }
    private fun refreshCatalog() {
        val list = JSONArray(host.applications())
        require(list.length() <= 256) { "Too many launchable apps" }
        val latest = (0 until list.length()).map { index ->
            val item = list.getJSONObject(index)
            val packageName = item.getString("package")
            val id = CRC32().apply { update(packageName.toByteArray(Charsets.UTF_8)) }.value.coerceAtLeast(1)
            App(id, packageName, item.getString("name"))
        }.associateBy { it.id }
        require(latest.size == list.length()) { "Independent app identity collision" }
        val catalog = JSONArray()
        latest.values.forEach { app -> catalog.put(JSONObject().put("source", JSONObject().put("Window", app.id))
            .put("title", app.name).put("application", app.name).put("process_id", JSONObject.NULL)
            .put("width", width).put("height", height).put("supports_input", false).put("camera", JSONObject.NULL)) }
        if(latest!=apps) PublisherClient.action(43, catalog.toString())
        apps=latest
        iconJobs.clear();iconJobs.offer(latest)
    }
    fun update() {
        if(catalogChanged.getAndSet(false)) refreshCatalog()
        iconResult.getAndSet(null)?.let { (catalog,icons) ->
            if(catalog===apps) runCatching {PublisherClient.action(46,icons.toString())}
                .onFailure {android.util.Log.w("Weavelo","Catalog artwork could not be published",it)}
        }
        val list = JSONArray(PublisherClient.action(44))
        val subscribed = mutableSetOf<Long>()
        val streams = mutableSetOf<Long>()
        for (i in 0 until list.length()) {
            val row = list.getJSONObject(i)
            val id = row.getJSONObject("source").getLong("Window")
            val demand = row.getJSONObject("demand")
            demand.put("artwork_epoch",row.getLong("artwork_epoch"))
            demand.put("requested_size",row.opt("requested_size") ?: JSONObject.NULL)
            val allViews=demand.optJSONArray("views") ?: JSONArray()
            val views=JSONArray()
            for(index in 0 until allViews.length()) {
                val view=allViews.getJSONObject(index);val stream=view.getLong("stream")
                streams.add(stream)
                if(stream !in failed) views.put(view)
            }
            demand.put("views",views)
            if (demand.optInt("subscriptions") == 0) continue
            subscribed.add(id)
            val app = apps[id] ?: continue
            val worker = workers[id] ?: if (views.length()>0) {
                if (workers.size >= 8) {
                    fail(app, demand, "Eight independent apps are already open"); continue
                }
                Capture(app, demand).also { workers[id] = it; it.start() }
            } else continue
            worker.demand = demand
            worker.error?.let { fail(app, demand, it); workers.remove(id)?.close() }
        }
        for (id in workers.keys.toList()) if (id !in subscribed) workers.remove(id)?.close()
        failed.retainAll(streams)
    }
    private fun fail(app: App, demand: JSONObject, reason: String) {
        val views=demand.optJSONArray("views") ?: JSONArray()
        for(index in 0 until views.length()) {
            val view=views.getJSONObject(index);val stream=view.getLong("stream")
            failed.add(stream)
            PublisherClient.action(36,JSONObject().put("stream",stream).put("grant",view.getLong("grant"))
                .put("reason",reason.take(200)).toString())
        }
    }
    private inner class Capture(private val app: App, initial: JSONObject) : AutoCloseable {
        private val alive = AtomicBoolean(true)
        @Volatile var demand = initial
        @Volatile var error: String? = null
        private val thread = Thread({ run() }, "weavelo-app-${app.id}")
        fun start() = thread.start()
        fun stop() { alive.set(false);thread.interrupt() }
        private fun run() {
            var display: Int? = null
            var publisher: GpuScreenPublisher? = null
            var target: Target? = null
            try {
                val icon=runCatching {
                    val png=host.applicationIcon(app.packageName,256)
                    require(png.size in 1..256*1024)
                    icon(png)
                }.onFailure { android.util.Log.w("Weavelo","Native application artwork unavailable",it) }.getOrNull()
                var artworkEpoch = -1L
                display = host.createApplication(app.packageName, width, height, density)
                var appliedRequestedSize: Pair<Int, Int>? = null
                var geometryAt = 0L
                var observed: Target? = null
                while (alive.get()) {
                    val current = demand
                    val reqArr = current.optJSONArray("requested_size")
                    val desiredSize = if (reqArr != null && reqArr.length() == 2) {
                        Pair(reqArr.getInt(0), reqArr.getInt(1))
                    } else null
                    if (desiredSize != appliedRequestedSize) {
                        val (targetW, targetH) = desiredSize ?: Pair(width, height)
                        val baseShort = minOf(width, height).coerceAtLeast(1)
                        val scaledDensity = ((density.toLong() * minOf(targetW, targetH)) / baseShort)
                            .toInt().coerceIn(120, 640)
                        runCatching {
                            host.resize(display, targetW, targetH, scaledDensity)
                            appliedRequestedSize = desiredSize
                            geometryAt = 0L
                        }
                    }
                    if (android.os.SystemClock.elapsedRealtime() >= geometryAt) {
                        val dimensions = JSONObject(host.applicationGeometry(display))
                        observed = Target(host,display,app.packageName,dimensions.getInt("width"),
                            dimensions.getInt("height"),dimensions.getInt("rotation"))
                        geometryAt = android.os.SystemClock.elapsedRealtime() + 50
                    }
                    val next = checkNotNull(observed)
                    if (target != next) {
                        target?.let { targets.remove(app.id,it) }
                        target = null
                        host.setSurface(display,null)
                        publisher?.finish();publisher?.close();publisher=null
                        // The source epoch may change concurrently with demand.
                        // Retry only after the next demand poll, leaving input
                        // disabled while capture geometry is unsettled.
                        val configured = runCatching {
                            PublisherClient.action(3,JSONObject().put("source",JSONObject().put("Window",app.id))
                                .put("grant",current.getLong("grant")).put("epoch",current.getLong("epoch"))
                                .put("width",next.width).put("height",next.height).put("rotation",next.rotation*90).toString())
                            host.acceptApplicationGeometry(display,next.width,next.height,next.rotation)
                        }
                        configured.exceptionOrNull()?.let { failure ->
                            val message = failure.message.orEmpty()
                            if (!message.contains("Capture configuration changed") && !message.contains("Application geometry changed again")) throw failure
                        }
                        if (configured.isFailure) { Thread.sleep(16);continue }
                        target=next;targets[app.id]=next
                    }
                    if(artworkEpoch!=current.getLong("artwork_epoch")) {
                        artworkEpoch=current.getLong("artwork_epoch")
                        val payload=JSONObject().put("source",JSONObject().put("Window",app.id))
                            .put("grant",current.getLong("grant")).put("epoch",artworkEpoch)
                            .put("icon",icon ?: JSONObject.NULL)
                        runCatching {PublisherClient.action(45,payload.toString())}.onFailure {
                            android.util.Log.w("Weavelo","Application artwork unavailable",it)
                            runCatching {PublisherClient.action(45,payload.put("icon",JSONObject.NULL).toString())}
                        }
                    }
                    val views = current.optJSONArray("views") ?: JSONArray()
                    if (views.length() > 0) {
                        if (publisher == null) {
                            publisher = GpuScreenPublisher(next.width,next.height,next.rotation)
                            host.setSurface(display,publisher.input)
                        }
                        publisher.update(views)
                        publisher.awaitFrame(if (publisher.pendingFrames) 1 else 16)
                    } else {
                        if (publisher != null) { host.setSurface(display,null);publisher.close();publisher=null }
                        Thread.sleep(16)
                    }
                }
            } catch (failure: Exception) {
                if (alive.get()) error = failure.cause?.message ?: failure.message ?: "Independent app stopped"
            } finally {
                target?.let { targets.remove(app.id,it) }
                display?.let { runCatching { host.setSurface(it,null) };runCatching { host.release(it) } }
                runCatching { publisher?.close() }
            }
        }
        override fun close() {
            stop()
            val interrupted=Thread.interrupted()
            try { thread.join(2500) } finally { if(interrupted) Thread.currentThread().interrupt() }
        }
    }
    override fun close() {
        context.unregisterReceiver(packageUpdates)
        iconsAlive.set(false);iconJobs.clear();iconThread.interrupt();iconResult.set(null)
        workers.values.forEach { it.stop() }
        workers.values.forEach { runCatching { it.close() } };workers.clear()
        runCatching { PublisherClient.action(43,"[]") }
    }
}
