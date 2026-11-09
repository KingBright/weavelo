package `fun`.hackerlife.weavelo

import android.content.Context
import org.json.JSONObject

/** Limits for the phone's outgoing screen, separate from its receiving preferences. */
internal data class OutgoingVideoQuality(val shortEdge: Int = 1080, val fps: Int = 60, val bitrateKbps: Int = 20_000) {
    fun valid() = (shortEdge == 0 || shortEdge in 240..4320) && fps in 1..120 && bitrateKbps in 100..100_000
    fun json(id: Long) = JSONObject().put("id", id).put("short_edge", shortEdge)
        .put("fps", fps).put("bitrate_kbps", bitrateKbps).toString()
    fun initialJson() = JSONObject().put("short_edge", shortEdge).put("fps", fps).put("bitrate_kbps", bitrateKbps)
}

internal object OutgoingVideoQualityStore {
    private const val PREFS = "outgoing_screen_quality"

    fun load(context: Context): OutgoingVideoQuality {
        val prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        return OutgoingVideoQuality(prefs.getInt("short_edge", 1080), prefs.getInt("fps", 60),
            prefs.getInt("bitrate_kbps", 20_000)).takeIf { it.valid() } ?: OutgoingVideoQuality()
    }

    /** The native bridge must atomically check the delivery ID before changing any live demand. */
    fun save(context: Context, id: Long, quality: OutgoingVideoQuality) {
        require(quality.valid()) { "Invalid outgoing screen quality" }
        if (id != 0L) PublisherClient.action(37, quality.json(id))
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .putInt("short_edge", quality.shortEdge).putInt("fps", quality.fps)
            .putInt("bitrate_kbps", quality.bitrateKbps).apply()
    }
}
