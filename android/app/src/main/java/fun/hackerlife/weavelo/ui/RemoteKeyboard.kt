package `fun`.hackerlife.weavelo.ui

import android.content.Context
import android.text.Editable
import android.text.SpannableStringBuilder
import android.text.InputType
import android.view.KeyEvent
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView

/** This editor owns composition only. It never reads or replaces a remote field. */
internal class RemoteImeView(context: Context, private val commit: (String) -> Unit,
    private val key: (String, Boolean) -> Unit) : View(context) {
    var acceptingInput = true
    private var currentInput: InputConnection? = null
    fun discardInput() { acceptingInput=false; currentInput?.finishComposingText() }
    init { isFocusable = true; isFocusableInTouchMode = true }
    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean = currentInput?.sendKeyEvent(event) == true || super.onKeyDown(keyCode,event)
    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean = currentInput?.sendKeyEvent(event) == true || super.onKeyUp(keyCode,event)
    override fun onCheckIsTextEditor() = true
    override fun onCreateInputConnection(out: EditorInfo): InputConnection {
        out.inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        out.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI or EditorInfo.IME_FLAG_NO_FULLSCREEN or EditorInfo.IME_FLAG_NO_PERSONALIZED_LEARNING
        out.initialSelStart = 0; out.initialSelEnd = 0
        val connection = object : BaseInputConnection(this, true) {
            private val composing = SpannableStringBuilder().apply { android.text.Selection.setSelection(this,0) }
            override fun getEditable(): Editable = composing
            override fun setComposingText(text: CharSequence?, newCursorPosition: Int): Boolean =
                acceptingInput && (text?.length ?: 0) <= 65536 && super.setComposingText(text,newCursorPosition)
            override fun commitText(text: CharSequence?, newCursorPosition: Int): Boolean {
                if (!acceptingInput) return false
                if ((text?.length ?: 0) > 65536) return false
                if (!text.isNullOrEmpty()) commit(text.toString())
                composing.clear(); removeComposingSpans(composing)
                return true
            }
            override fun finishComposingText(): Boolean {
                if (acceptingInput && getComposingSpanStart(composing) >= 0 && composing.isNotEmpty()) commit(composing.toString())
                composing.clear(); removeComposingSpans(composing)
                return true
            }
            override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
                if (!acceptingInput) return false
                if (composing.isNotEmpty()) return deleteLocal(beforeLength,afterLength,false)
                if (beforeLength !in 0..64 || afterLength !in 0..64) return false
                repeat(beforeLength) { key("backspace", true); key("backspace", false) }
                repeat(afterLength) { key("delete", true); key("delete", false) }
                return true
            }
            override fun deleteSurroundingTextInCodePoints(beforeLength: Int, afterLength: Int): Boolean =
                if (!acceptingInput) false else if (composing.isNotEmpty()) deleteLocal(beforeLength,afterLength,true)
                else deleteSurroundingText(beforeLength, afterLength)
            private fun deleteLocal(before: Int, after: Int, codePoints: Boolean): Boolean {
                if (before < 0 || after < 0) return false
                val text=composing.toString()
                val a=android.text.Selection.getSelectionStart(composing).coerceIn(0,text.length)
                val b=android.text.Selection.getSelectionEnd(composing).coerceIn(a,text.length)
                var start = if(codePoints) text.offsetByCodePoints(a,-minOf(before,text.codePointCount(0,a))) else (a-before).coerceAtLeast(0)
                var end = if(codePoints) text.offsetByCodePoints(b,minOf(after,text.codePointCount(b,text.length))) else (b+after.coerceAtMost(text.length-b))
                if (start in 1 until text.length && Character.isLowSurrogate(text[start]) && Character.isHighSurrogate(text[start-1])) start--
                if (end in 1 until text.length && Character.isLowSurrogate(text[end]) && Character.isHighSurrogate(text[end-1])) end++
                composing.delete(b,end);composing.delete(start,a)
                return true
            }
            override fun sendKeyEvent(event: KeyEvent): Boolean {
                if (!acceptingInput) return false
                val name = when (event.keyCode) {
                    KeyEvent.KEYCODE_DEL -> "backspace"; KeyEvent.KEYCODE_FORWARD_DEL -> "delete"
                    KeyEvent.KEYCODE_ENTER -> "enter"; KeyEvent.KEYCODE_TAB -> "tab"
                    KeyEvent.KEYCODE_DPAD_LEFT -> "left"; KeyEvent.KEYCODE_DPAD_RIGHT -> "right"
                    KeyEvent.KEYCODE_DPAD_UP -> "up"; KeyEvent.KEYCODE_DPAD_DOWN -> "down"
                    KeyEvent.KEYCODE_ESCAPE -> "escape"; else -> null
                }
                if (name != null) {
                    if (composing.isNotEmpty() && name in listOf("backspace","delete")) {
                        if (event.action==KeyEvent.ACTION_DOWN) deleteLocal(if(name=="backspace") 1 else 0,if(name=="delete") 1 else 0,true)
                    } else key(name,event.action==KeyEvent.ACTION_DOWN)
                    return true
                }
                val character=event.unicodeChar
                if (Character.isValidCodePoint(character) && character != 0 && !Character.isISOControl(character) && !event.isCtrlPressed && !event.isMetaPressed) {
                    if(event.action==KeyEvent.ACTION_DOWN) commit(String(Character.toChars(character)))
                    return true
                }
                return false
            }
            override fun performEditorAction(actionCode: Int): Boolean {if (!acceptingInput) return false;key("enter",true);key("enter",false);return true}
        }
        currentInput=connection
        return connection
    }
}

@Composable
internal fun RemoteKeyboard(identity: Any, visible: Boolean, enabled: Boolean,
    onText: (String) -> Unit, onKey: (String, Boolean) -> Unit) {
    val context = LocalContext.current
    // A new source owns a different View/InputConnection. Late IME callbacks
    // retain the old callbacks and must fail the native exact-binding check.
    key(identity) {
        var editor by remember { mutableStateOf<RemoteImeView?>(null) }
        AndroidView(factory = { RemoteImeView(it,onText,onKey).also { view -> editor=view } },
            modifier = Modifier.size(1.dp))
        LaunchedEffect(editor, visible, enabled) {
            val view=editor ?: return@LaunchedEffect
            val imm=context.getSystemService(InputMethodManager::class.java)
            view.acceptingInput=visible && enabled
            if (visible && enabled) {view.requestFocus();imm.showSoftInput(view,InputMethodManager.SHOW_IMPLICIT)}
            else {view.discardInput();imm.hideSoftInputFromWindow(view.windowToken,0);view.clearFocus()}
        }
        DisposableEffect(editor) {
            // The initial null -> View recomposition must not dispose the new
            // editor by reading the mutable state from the old effect.
            val owned = editor
            onDispose { owned?.let { it.discardInput();context.getSystemService(InputMethodManager::class.java).hideSoftInputFromWindow(it.windowToken,0);it.clearFocus() } }
        }
    }
}
