package `fun`.hackerlife.weavelo.ui

/** Local tap intent is single-use and cannot survive a new gesture or dismissal. */
internal class EditorFocusGate<T> {
    private var generation = 0L
    private var pending: Triple<String, Long, T>? = null
    @Synchronized fun invalidate() { generation++; pending = null }
    @Synchronized fun intent(): Long = generation
    @Synchronized fun record(intent: Long, sequence: String, at: Long, geometry: T) {
        if (generation == intent) pending = Triple(sequence, at, geometry)
    }
    @Synchronized fun take(sequence: String, now: Long): T? {
        val value = pending ?: return null
        if (value.first != sequence) return null
        pending = null
        return if (now - value.second in 0L..600L) value.third else null
    }
}
