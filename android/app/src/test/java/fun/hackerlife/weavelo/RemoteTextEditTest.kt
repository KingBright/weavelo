package `fun`.hackerlife.weavelo

import org.junit.Assert.*
import org.junit.Test

class RemoteTextEditTest {
    @Test fun physicalKeyZeroAndShiftProduceTheRequestedText() {
        var edit = RemoteTextEdit.edit("",0,0,0,0)!!
        edit = RemoteTextEdit.edit(edit.text,edit.start,edit.end,0,1)!!
        assertEquals("aA",edit.text)
        assertEquals("!",RemoteTextEdit.edit("",0,0,0x12,1)!!.text)
        assertEquals("a",RemoteTextEdit.edit("",0,0,0,0x21)!!.text)
    }
    @Test fun unicodeDeletionAndSelectionsPreserveCodePoints() {
        assertEquals("中",RemoteTextEdit.edit("中😀",3,3,0x33,0)!!.text)
        assertEquals("中",RemoteTextEdit.edit("😀中",0,0,0x75,0)!!.text)
        assertEquals("aXc",RemoteTextEdit.edit("abc",1,2,0x07,1)!!.text)
        assertEquals(0,RemoteTextEdit.edit("😀",2,2,0x7B,0)!!.end)
    }
    @Test fun unknownChordsAndUnknownSelectionNeverBecomeBareText() {
        assertNull(RemoteTextEdit.edit("abc",3,3,0x09,2))
        assertNull(RemoteTextEdit.edit("abc",3,3,0,4))
        assertNull(RemoteTextEdit.edit("abc",-1,-1,0,0))
        assertNull(RemoteTextEdit.edit("abc",0,0,0xFFFF,0))
        assertEquals(RemoteTextEdit.Edit("abc",0,3),RemoteTextEdit.edit("abc",3,3,0,2))
    }
}
