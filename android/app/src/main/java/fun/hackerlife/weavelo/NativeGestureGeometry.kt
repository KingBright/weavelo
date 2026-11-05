package `fun`.hackerlife.weavelo

/** GestureDescription rounds to physical pixels before MotionEvent injection. */
internal fun nativeGestureMoved(oldX: Float, oldY: Float, x: Float, y: Float): Boolean =
    Math.round(oldX) != Math.round(x) || Math.round(oldY) != Math.round(y)
