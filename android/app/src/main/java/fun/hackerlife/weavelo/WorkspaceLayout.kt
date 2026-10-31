package `fun`.hackerlife.weavelo

/** Maximize visible video area; each row shares its available height. Units must match. */
internal fun workspaceGridColumns(width: Float, height: Float, aspects: List<Float>, chrome: Float = 78f): Int {
    if (width <= 0 || height <= 0 || !width.isFinite() || !height.isFinite() ||
        aspects.isEmpty() || aspects.any { it <= 0 || !it.isFinite() }) return 1
    fun area(columns: Int): Float {
        val rows = aspects.chunked(columns)
        val videoHeight = (height / rows.size - chrome).coerceAtLeast(0f)
        return rows.sumOf { row ->
            val videoWidth = (width / row.size - 6f).coerceAtLeast(0f)
            row.sumOf { aspect ->
                val fittedHeight = minOf(videoHeight, videoWidth / aspect)
                (fittedHeight * fittedHeight * aspect).toDouble()
            }
        }.toFloat()
    }
    return if (aspects.size > 1 && area(2) > area(1)) 2 else 1
}
