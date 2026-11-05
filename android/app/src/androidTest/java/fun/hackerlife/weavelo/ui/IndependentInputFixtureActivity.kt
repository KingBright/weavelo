package `fun`.hackerlife.weavelo.ui

import android.app.Activity
import android.os.Bundle
import android.os.Build
import android.text.Editable
import android.text.TextWatcher
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.UUID

/** Test APK only. Records this fixture's native events, never another app's input. */
class IndependentInputFixtureActivity : Activity() {
    private val nonce=UUID.randomUUID().toString()
    private val touch=JSONArray()
    private val keys=JSONArray()
    private lateinit var editor: EditText
    private var clicks=0
    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        window.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_HIDDEN)
        val root=LinearLayout(this).apply {orientation=LinearLayout.VERTICAL;setPadding(24,24,24,24);setBackgroundColor(0xfff5f5f5.toInt())}
        root.addView(TextView(this).apply {text="Independent input fixture";textSize=24f;setTextColor(0xff111111.toInt())})
        editor=EditText(this).apply {setSingleLine(false);setTextColor(0xff111111.toInt());hint="Type here";textSize=22f;minLines=3}
        root.addView(editor)
        root.addView(Button(this).apply {
            text="Count 0"
            setOnClickListener {clicks++;text="Count $clicks";writeProof()}
        })
        root.addView(object: View(this) {
            init {setBackgroundColor(0xff126a72.toInt());contentDescription="Native multi-touch area"}
            override fun onTouchEvent(event: MotionEvent):Boolean=true
        },LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT,0,1f))
        setContentView(root)
        editor.addTextChangedListener(object: TextWatcher {
            override fun beforeTextChanged(s:CharSequence?,start:Int,count:Int,after:Int) {}
            override fun onTextChanged(s:CharSequence?,start:Int,before:Int,count:Int) {}
            override fun afterTextChanged(s:Editable?) {writeProof()}
        })
        editor.requestFocus()
        root.post {writeProof()}
    }
    override fun dispatchTouchEvent(event: MotionEvent):Boolean {
        if(touch.length()<128) touch.put(JSONObject().put("action",event.actionMasked).put("index",event.actionIndex)
            .put("time",event.eventTime).put("points",JSONArray().apply {
                for(index in 0 until event.pointerCount) put(JSONObject().put("id",event.getPointerId(index))
                    .put("x",event.getX(index)).put("y",event.getY(index)))
            }))
        return super.dispatchTouchEvent(event).also {writeProof()}
    }
    override fun dispatchKeyEvent(event: KeyEvent):Boolean {
        if(keys.length()<128) keys.put(JSONObject().put("action",event.action).put("code",event.keyCode))
        return super.dispatchKeyEvent(event).also {writeProof()}
    }
    private fun writeProof() {
        if(!::editor.isInitialized) return
        @Suppress("DEPRECATION")
        val displayId=if(Build.VERSION.SDK_INT>=30) display?.displayId else windowManager.defaultDisplay.displayId
        File(checkNotNull(getExternalFilesDir(null)),"independent-input.json").writeText(JSONObject()
            .put("nonce",nonce).put("display",displayId).put("text",editor.text.toString())
            .put("clicks",clicks).put("touch",touch).put("keys",keys).toString())
    }
}
