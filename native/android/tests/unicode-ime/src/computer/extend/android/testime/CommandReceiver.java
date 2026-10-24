package computer.extend.android.testime;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;

/** Explicit broadcasts require DUMP (shell/system); never exported to normal apps. */
public final class CommandReceiver extends BroadcastReceiver {
    @Override public void onReceive(Context context, Intent intent) {
        String command = intent.getStringExtra("command");
        UnicodeIme ime = UnicodeIme.active;
        String receipt = ime == null ? "{\"error\":\"ime_not_running\"}" : ime.command(command);
        setResultData(receipt);
    }
}
