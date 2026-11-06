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
        val packageName: String, val width: Int, val height: Int)
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
    init {
        refreshCatalog()
        context.registerReceiver(packageUpdates,IntentFilter().apply {
            addAction(Intent.ACTION_PACKAGE_ADDED);addAction(Intent.ACTION_PACKAGE_REMOVED)
            addAction(Intent.ACTION_PACKAGE_CHANGED);addDataScheme("package")
        },Context.RECEIVER_NOT_EXPORTED)
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
        if(latest==apps) return
        val catalog = JSONArray()
        latest.values.forEach { app -> catalog.put(JSONObject().put("source", JSONObject().put("Window", app.id))
            .put("title", app.name).put("application", app.name).put("process_id", JSONObject.NULL)
            .put("width", width).put("height", height).put("supports_input", false).put("camera", JSONObject.NULL)) }
        PublisherClient.action(43, catalog.toString())
        apps=latest
    }
    fun update() {
        if(catalogChanged.getAndSet(false)) refreshCatalog()
        val list = JSONArray(PublisherClient.action(44))
        val subscribed = mutableSetOf<Long>()
        val streams = mutableSetOf<Long>()
        for (i in 0 until list.length()) {
            val row = list.getJSONObject(i)
            val id = row.getJSONObject("source").getLong("Window")
            val demand = row.getJSONObject("demand")
            demand.put("artwork_epoch",row.getLong("artwork_epoch"))
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
                    val png=host.applicationIcon(app.packageName)
                    require(png.size in 1..256*1024)
                    val digest=java.security.MessageDigest.getInstance("SHA-256").digest(png)
                    JSONObject().put("id",JSONArray(digest.map { it.toInt() and 255 }))
                        .put("png",JSONArray(png.map { it.toInt() and 255 }))
                }.onFailure { android.util.Log.w("Weavelo","Native application artwork unavailable",it) }.getOrNull()
                var artworkEpoch = -1L
                display = host.createApplication(app.packageName, width, height, density)
                target = Target(host, display, app.packageName, width, height)
                targets[app.id] = target
                while (alive.get()) {
                    val current = demand
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
                            publisher = GpuScreenPublisher(width,height)
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
        workers.values.forEach { it.stop() }
        workers.values.forEach { runCatching { it.close() } };workers.clear()
        runCatching { PublisherClient.action(43,"[]") }
    }
}
