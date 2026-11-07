package `fun`.hackerlife.weavelo

import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import `fun`.hackerlife.weavelo.managed.cancelManagedKey
import `fun`.hackerlife.weavelo.managed.cancelManagedMotion
import org.junit.Assert.*
import org.junit.Test

/** Exercises cancellation without injecting events into a device or changing permissions. */
class ManagedInputCancellationTest {
    @Test fun staleMultitouchIsCanceledAtTheCurrentTime() {
        val properties = arrayOf(2,7).map { pointer -> MotionEvent.PointerProperties().apply {
            id=pointer;toolType=MotionEvent.TOOL_TYPE_FINGER
        } }.toTypedArray()
        val coords = arrayOf(45f,90f).map { px -> MotionEvent.PointerCoords().apply {
            x=px;y=px+10;pressure=.5f;size=1f
        } }.toTypedArray()
        val held=MotionEvent.obtain(100,150,MotionEvent.ACTION_MOVE,2,properties,coords,
            0,0,1f,1f,12,0,InputDevice.SOURCE_TOUCHSCREEN,0)
        val cancel=cancelManagedMotion(held,20_000)
        try {
            assertEquals(MotionEvent.ACTION_CANCEL,cancel.actionMasked)
            assertEquals(100L,cancel.downTime)
            assertEquals(20_000L,cancel.eventTime)
            assertEquals(2,cancel.pointerCount)
            assertEquals(2,cancel.getPointerId(0));assertEquals(7,cancel.getPointerId(1))
            assertEquals(45f,cancel.getX(0),0f);assertEquals(100f,cancel.getY(1),0f)
            assertEquals(InputDevice.SOURCE_TOUCHSCREEN,cancel.source)
            assertEquals(MotionEvent.ACTION_MOVE,held.actionMasked)
            assertEquals(150L,held.eventTime)
        } finally { cancel.recycle();held.recycle() }
    }

    @Test fun staleEnterIsReleasedAsCanceledAtTheCurrentTime() {
        val held=KeyEvent(100,150,KeyEvent.ACTION_DOWN,KeyEvent.KEYCODE_ENTER,0,
            KeyEvent.META_SHIFT_ON,12,30,0,InputDevice.SOURCE_KEYBOARD)
        val cancel=cancelManagedKey(held,20_000)
        assertEquals(KeyEvent.ACTION_UP,cancel.action)
        assertTrue(cancel.isCanceled)
        assertEquals(KeyEvent.KEYCODE_ENTER,cancel.keyCode)
        assertEquals(100L,cancel.downTime);assertEquals(20_000L,cancel.eventTime)
        assertEquals(KeyEvent.META_SHIFT_ON,cancel.metaState)
        assertEquals(12,cancel.deviceId);assertEquals(30,cancel.scanCode)
        assertEquals(InputDevice.SOURCE_KEYBOARD,cancel.source)
        assertEquals(KeyEvent.ACTION_DOWN,held.action)
    }
}
