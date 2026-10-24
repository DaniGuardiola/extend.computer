package computer.extend.android.displayprobe;

import android.app.Activity;
import android.os.Bundle;
import android.widget.TextView;

/** Disposable foreground owner for an ordinary-UID secondary-display probe. */
public final class ProbeActivity extends Activity {
    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        TextView text = new TextView(this);
        text.setText("Testing Android display extension");
        text.setTextSize(24);
        text.setPadding(40, 80, 40, 40);
        setContentView(text);
    }
}
