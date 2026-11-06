package `fun`.hackerlife.weavelo

import android.app.UiAutomation
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.Surface
import androidx.test.platform.app.InstrumentationRegistry
import `fun`.hackerlife.weavelo.managed.IManagedScreen
import `fun`.hackerlife.weavelo.managed.ManagedApplicationInput
import `fun`.hackerlife.weavelo.managed.ManagedApplications
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

/** Android MotionEvents without injecting into a user window or changing any permission. */
class ManagedApplicationInputTest {
    private class Recorder : IManagedScreen.Stub() {
        val events=mutableListOf<Pair<Int,Int>>()
        override fun info()="{}"
        override fun applications()="[]"
        override fun applicationIcon(packageName:String)=byteArrayOf()
        override fun createMirror(width:Int,height:Int,density:Int)=error("No display creation in this test")
        override fun createApplication(packageName:String,width:Int,height:Int,density:Int)=error("No app launch in this test")
        override fun setSurface(id:Int,surface:Surface?)=error("No capture in this test")
        override fun resize(id:Int,width:Int,height:Int,density:Int)=error("No resize in this test")
        override fun release(id:Int)=error("No display release in this test")
        override fun injectKey(id:Int,event:KeyEvent):Boolean {assertEquals(7,id);return true}
        override fun injectMotion(id:Int,event:MotionEvent):Boolean {
            assertEquals(7,id);events.add(event.actionMasked to event.pointerCount);return true
        }
    }
    private fun contact(id:Long,action:String)=JSONObject().put("pointer_id",id).put("action",action)
        .put("normalized_x",.3).put("normalized_y",.6).put("pressure",if(action=="Up"||action=="Cancel") 0 else 1)
    private fun frame(vararg contacts:JSONObject)=JSONObject().put("TouchFrame",JSONArray(contacts.toList()))
    @Test fun cancelEndsTheWholeGestureWithoutProducingAClick() {
        InstrumentationRegistry.getInstrumentation().getUiAutomation(UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)
        val host=Recorder();val input=ManagedApplicationInput(ManagedApplications.Target(host,7,"test.fixture",720,1280))
        assertTrue(input.execute(frame(contact(1,"Down"),contact(2,"Down"))))
        assertTrue(input.execute(frame(contact(1,"Cancel"),contact(2,"Cancel"))))
        assertEquals(listOf(MotionEvent.ACTION_DOWN to 1,MotionEvent.ACTION_POINTER_DOWN to 2,
            MotionEvent.ACTION_CANCEL to 2),host.events)
        assertTrue(input.execute(frame(contact(3,"Down"))))
        assertTrue(input.execute(frame(contact(3,"Up"))))
        assertEquals(MotionEvent.ACTION_DOWN to 1,host.events[3])
        assertEquals(MotionEvent.ACTION_UP to 1,host.events[4])
        input.release()
        assertEquals("Ended contacts must not be released twice",5,host.events.size)
    }
    @Test fun releasingOneFingerPreservesTheOtherUntilSessionCancellation() {
        InstrumentationRegistry.getInstrumentation().getUiAutomation(UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)
        val host=Recorder();val input=ManagedApplicationInput(ManagedApplications.Target(host,7,"test.fixture",720,1280))
        assertTrue(input.execute(frame(contact(1,"Down"),contact(2,"Down"))))
        assertTrue(input.execute(frame(contact(2,"Up"))))
        assertTrue(input.execute(frame(contact(1,"Move"))))
        input.release()
        assertEquals(listOf(MotionEvent.ACTION_DOWN to 1,MotionEvent.ACTION_POINTER_DOWN to 2,
            MotionEvent.ACTION_POINTER_UP to 2,MotionEvent.ACTION_MOVE to 1,MotionEvent.ACTION_CANCEL to 1),host.events)
    }
}
