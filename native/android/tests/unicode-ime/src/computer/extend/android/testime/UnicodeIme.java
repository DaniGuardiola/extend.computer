package computer.extend.android.testime;

import android.inputmethodservice.InputMethodService;
import android.os.SystemClock;
import android.util.Log;
import android.view.View;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.widget.TextView;
import org.json.JSONObject;

/** Test-only final-commit driver. No editor reads, clipboard access, or arbitrary payloads. */
public final class UnicodeIme extends InputMethodService {
    static UnicodeIme active;
    private static final String TARGET = "computer.extend.android.development";
    public static final String FINAL = "Unicode e\u0301 café 👩‍💻 中文测试 日本語 ✅";
    public static final String DRAFT = "Draft e\u0301 👩‍💻 中文";
    public static final String STALE = "STALE e\u0301 👩‍💻 中文 forbidden";
    private InputConnection retained;
    private long epoch, retainedEpoch, sequence;
    private boolean live, allowed;

    @Override public void onCreate() { super.onCreate(); active = this; }
    @Override public boolean onEvaluateFullscreenMode() { return false; }
    @Override public View onCreateInputView() {
        TextView view = new TextView(this);
        view.setText("Extend Unicode test keyboard");
        view.setTextSize(18); view.setPadding(20, 16, 20, 16);
        return view;
    }
    @Override public void onStartInput(EditorInfo info, boolean restarting) {
        super.onStartInput(info, restarting);
        epoch++; live = true; allowed = info != null && TARGET.equals(info.packageName);
        record("start", null, null, false);
    }
    @Override public void onFinishInput() {
        live = false; epoch++; record("finish_input", null, null, false);
        super.onFinishInput();
    }
    @Override public void onDestroy() {
        retained = null; live = false; if (active == this) active = null;
        super.onDestroy();
    }
    String command(String action) {
        if (action == null) return record("invalid", "missing_command", null, false);
        if (action.equals("state")) return record(action, null, null, false);
        if (action.equals("clear")) { retained = null; return record(action, null, null, false); }
        boolean stale = action.startsWith("stale-");
        // A saved connection is always captured from the allowlisted package. Never use a
        // live connection for any other editor, including when testing a stale object.
        if ((!stale && (!live || !allowed)) || (stale && live && !allowed))
            return record(action, "target_not_allowed", null, stale);
        InputConnection connection = stale ? retained : getCurrentInputConnection();
        if (connection == null) return record(action, "no_connection", null, stale);
        if (action.equals("retain")) {
            retained = connection; retainedEpoch = epoch;
            return record(action, null, null, false);
        }
        Boolean result;
        String payload = null;
        try {
            switch (action) {
                case "draft": payload = DRAFT; result = connection.setComposingText(payload, 1); break;
                case "commit": payload = FINAL; result = connection.commitText(payload, 1); break;
                case "finish": result = connection.finishComposingText(); break;
                case "stale-commit": payload = STALE; result = connection.commitText(payload, 1); break;
                case "stale-finish": result = connection.finishComposingText(); break;
                default: return record(action, "unknown_command", null, stale);
            }
        } catch (RuntimeException error) {
            return record(action, error.getClass().getSimpleName(), null, stale);
        }
        return record(action, null, result, stale, payload);
    }
    private String record(String action, String error, Boolean result, boolean stale) {
        return record(action, error, result, stale, null);
    }
    private String record(String action, String error, Boolean result, boolean stale, String payload) {
        JSONObject receipt = new JSONObject();
        try {
            receipt.put("sequence", ++sequence).put("elapsed_realtime_ms", SystemClock.elapsedRealtime());
            receipt.put("command", action).put("epoch", epoch).put("live", live).put("target_allowed", allowed);
            receipt.put("retained", retained != null).put("retained_epoch", retainedEpoch).put("stale_call", stale);
            if (result != null) receipt.put("call_return", result);
            if (error != null) receipt.put("error", error);
            if (payload != null) receipt.put("fixture", action).put("utf16_units", payload.length())
                .put("unicode_scalars", payload.codePointCount(0, payload.length()));
        } catch (Exception impossible) { throw new AssertionError(impossible); }
        String json = receipt.toString(); Log.i("ExtendTestIme", json); return json;
    }
}
