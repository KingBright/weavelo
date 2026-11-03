package `fun`.hackerlife.weavelo.ui

import android.view.inputmethod.EditorInfo
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.*
import org.junit.Test

class RemoteKeyboardInteractionTest {
    @Test fun nativeMultipleCharacterKeyEventCommitsOnceAndCannotEscapeADiscardedEditor() {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        instrumentation.runOnMainSync {
            val text=mutableListOf<String>()
            val view=RemoteImeView(instrumentation.targetContext,{text.add(it)},{_,_->})
            view.onCreateInputConnection(EditorInfo())
            val event=android.view.KeyEvent(android.os.SystemClock.uptimeMillis(),"你好🙂",
                android.view.KeyCharacterMap.VIRTUAL_KEYBOARD,0)
            assertTrue(view.onKeyMultiple(android.view.KeyEvent.KEYCODE_UNKNOWN,1,event))
            assertEquals(listOf("你好🙂"),text)
            view.discardInput()
            assertFalse(view.onKeyMultiple(android.view.KeyEvent.KEYCODE_UNKNOWN,1,event))
            assertEquals(listOf("你好🙂"),text)
        }
    }
    @Test fun hidingTheKeyboardDiscardsCompositionAndLateCallbacks() {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        instrumentation.runOnMainSync {
            val text=mutableListOf<String>();val keys=mutableListOf<String>()
            val view=RemoteImeView(instrumentation.targetContext,{text.add(it)},{key,_->keys.add(key)})
            val connection=view.onCreateInputConnection(EditorInfo())
            connection.setComposingText("未提交🙂",1)
            view.discardInput()
            assertFalse(connection.commitText("late",1))
            connection.finishComposingText()
            assertFalse(connection.deleteSurroundingText(1,0))
            assertFalse(connection.performEditorAction(EditorInfo.IME_ACTION_DONE))
            assertTrue(text.isEmpty());assertTrue(keys.isEmpty())
        }
    }
    @Test fun nativeCompositionCommitsCjkAndWholeEmojiOnce() {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        instrumentation.runOnMainSync {
            val text=mutableListOf<String>();val keys=mutableListOf<String>()
            val view=RemoteImeView(instrumentation.targetContext,{text.add(it)},{key,down->keys.add("$key:$down")})
            val connection=view.onCreateInputConnection(EditorInfo())
            assertTrue(connection.setComposingText("ni",1))
            assertTrue(connection.setComposingText("你好👨‍👩‍👧‍👦",1))
            assertTrue(text.isEmpty())
            assertTrue(connection.commitText("你好👨‍👩‍👧‍👦",1))
            connection.finishComposingText()
            assertEquals(listOf("你好👨‍👩‍👧‍👦"),text)
            assertTrue(keys.isEmpty())
            connection.deleteSurroundingText(1,0)
            assertEquals(listOf("backspace:true","backspace:false"),keys)
        }
    }
    @Test fun deletionDuringCompositionNeverDeletesRemoteTextOrSplitsSurrogates() {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        instrumentation.runOnMainSync {
            val text=mutableListOf<String>();val keys=mutableListOf<String>()
            val view=RemoteImeView(instrumentation.targetContext,{text.add(it)},{key,_->keys.add(key)})
            val connection=view.onCreateInputConnection(EditorInfo())
            connection.setComposingText("中🙂",1)
            connection.deleteSurroundingTextInCodePoints(1,0)
            connection.finishComposingText()
            assertEquals(listOf("中"),text);assertTrue(keys.isEmpty())
        }
    }
    @Test fun retainedInputConnectionDoesNotAdoptTheReplacementViewsOwner() {
        val instrumentation=InstrumentationRegistry.getInstrumentation()
        instrumentation.runOnMainSync {
            val commits=mutableListOf<Pair<Int,String>>()
            fun input(owner:Int)=RemoteImeView(instrumentation.targetContext,{commits.add(owner to it)},{_,_->}).onCreateInputConnection(EditorInfo())
            val previous=input(1);previous.setComposingText("old",1)
            val current=input(2);current.commitText("new",1);previous.commitText("late",1)
            assertEquals(listOf(2 to "new",1 to "late"),commits)
        }
    }
}
