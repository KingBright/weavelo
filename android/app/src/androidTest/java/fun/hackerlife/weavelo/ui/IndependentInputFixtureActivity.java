package fun.hackerlife.weavelo.ui;

import android.app.Activity;
import android.os.Bundle;
import android.text.Editable;
import android.text.TextWatcher;
import android.util.DisplayMetrics;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.View;
import android.view.WindowManager;
import android.widget.Button;
import android.widget.EditText;
import android.widget.LinearLayout;
import android.widget.TextView;
import org.json.JSONArray;
import org.json.JSONObject;
import java.io.File;
import java.io.FileOutputStream;
import java.nio.charset.StandardCharsets;
import java.util.UUID;

/** Test APK only. Platform Java keeps standalone launch independent of the target APK classloader. */
public final class IndependentInputFixtureActivity extends Activity {
    private final String nonce = UUID.randomUUID().toString();
    private final JSONArray touch = new JSONArray(), keys = new JSONArray();
    private EditText editor;
    private int clicks;

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        getWindow().setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_HIDDEN);
        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        root.setPadding(24, 24, 24, 24);
        root.setBackgroundColor(0xfff5f5f5);
        TextView title = new TextView(this);
        title.setText("Independent input fixture"); title.setTextSize(24); title.setTextColor(0xff111111);
        root.addView(title);
        editor = new EditText(this);
        editor.setSingleLine(false); editor.setTextColor(0xff111111);
        editor.setHint("Type here"); editor.setTextSize(22); editor.setMinLines(3);
        root.addView(editor);
        Button button = new Button(this);
        button.setText("Count 0");
        button.setOnClickListener(view -> { clicks++; button.setText("Count " + clicks); writeProof(); });
        root.addView(button);
        View area = new View(this) {
            @Override public boolean onTouchEvent(MotionEvent event) { return true; }
        };
        area.setBackgroundColor(0xff126a72);
        area.setContentDescription("Native multi-touch area");
        root.addView(area, new LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1));
        setContentView(root);
        editor.addTextChangedListener(new TextWatcher() {
            public void beforeTextChanged(CharSequence s, int start, int count, int after) {}
            public void onTextChanged(CharSequence s, int start, int before, int count) {}
            public void afterTextChanged(Editable s) { writeProof(); }
        });
        editor.requestFocus();
        root.post(this::writeProof);
    }

    @Override public boolean dispatchTouchEvent(MotionEvent event) {
        try {
            if (touch.length() < 128) {
                JSONArray points = new JSONArray();
                for (int i = 0; i < event.getPointerCount(); i++)
                    points.put(new JSONObject().put("id", event.getPointerId(i))
                        .put("x", event.getX(i)).put("y", event.getY(i)));
                touch.put(new JSONObject().put("action", event.getActionMasked()).put("index", event.getActionIndex())
                    .put("time", event.getEventTime()).put("points", points));
            }
        } catch (org.json.JSONException failure) { throw new IllegalStateException(failure); }
        boolean handled = super.dispatchTouchEvent(event);
        writeProof(); return handled;
    }

    @Override public boolean dispatchKeyEvent(KeyEvent event) {
        try {
            if (keys.length() < 128) keys.put(new JSONObject().put("action", event.getAction()).put("code", event.getKeyCode()));
        } catch (org.json.JSONException failure) { throw new IllegalStateException(failure); }
        boolean handled = super.dispatchKeyEvent(event);
        writeProof(); return handled;
    }

    @SuppressWarnings("deprecation") private void writeProof() {
        if (editor == null) return;
        DisplayMetrics metrics = new DisplayMetrics();
        getWindowManager().getDefaultDisplay().getRealMetrics(metrics);
        try {
            JSONObject proof = new JSONObject().put("nonce", nonce)
                .put("display", getWindowManager().getDefaultDisplay().getDisplayId())
                .put("package", getPackageName()).put("width", metrics.widthPixels).put("height", metrics.heightPixels)
                .put("text", editor.getText().toString()).put("clicks", clicks).put("touch", touch).put("keys", keys);
            try (FileOutputStream output = new FileOutputStream(new File(getExternalFilesDir(null), "independent-input.json"))) {
                output.write(proof.toString().getBytes(StandardCharsets.UTF_8));
            }
        } catch (Exception failure) { throw new IllegalStateException(failure); }
    }
}
