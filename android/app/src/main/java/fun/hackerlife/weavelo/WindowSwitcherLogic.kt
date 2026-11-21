package `fun`.hackerlife.weavelo

internal fun cycleWorkspacePaneId(paneIds: List<Int>, currentId: Int, delta: Int): Int? {
    if (paneIds.isEmpty()) return null
    val idx = paneIds.indexOf(currentId).let { if (it >= 0) it else 0 }
    val nextIdx = Math.floorMod(idx + delta, paneIds.size)
    return paneIds[nextIdx]
}
