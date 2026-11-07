package `fun`.hackerlife.weavelo

// One observed display state is consumed atomically by the capture worker.
// Rotation is metadata for source/input fencing; the projection surface is
// already oriented by Android and must not be rotated a second time.
internal data class ProjectionCaptureGeometry(val width: Int, val height: Int, val density: Int, val rotation: Int)

internal fun captureReconfiguration(applied: ProjectionCaptureGeometry?, current: ProjectionCaptureGeometry): ProjectionCaptureGeometry? =
    current.takeIf { it != applied }

internal fun displayRotationDegrees(rotation: Int): Int = when (rotation) {
    1 -> 90
    2 -> 180
    3 -> 270
    else -> 0
}
