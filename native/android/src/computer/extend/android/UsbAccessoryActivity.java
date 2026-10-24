package computer.extend.android;

import android.app.Activity;
import android.app.PendingIntent;
import android.content.Intent;
import android.hardware.usb.UsbAccessory;
import android.hardware.usb.UsbManager;
import android.os.Bundle;
import android.widget.Toast;

/** OS accessory entry point; permission does not replace pinned peer trust. */
public final class UsbAccessoryActivity extends Activity {
    private static final String RESULT = "computer.extend.android.USB_PERMISSION_RESULT";
    @SuppressWarnings("deprecation")
    static UsbAccessory accessory(Intent intent) {
        return android.os.Build.VERSION.SDK_INT >= 33
            ? intent.getParcelableExtra(UsbManager.EXTRA_ACCESSORY, UsbAccessory.class)
            : intent.getParcelableExtra(UsbManager.EXTRA_ACCESSORY);
    }
    static boolean supported(UsbAccessory accessory) {
        return accessory != null && "extend.computer".equals(accessory.getManufacturer())
            && "extend.computer".equals(accessory.getModel())
            && accessory.getSerial() != null && accessory.getSerial().matches("[0-9a-f]{64}");
    }
    static boolean attached(UsbManager manager, UsbAccessory accessory) {
        if (!supported(accessory)) return false;
        UsbAccessory[] accessories = manager.getAccessoryList();
        if (accessories != null) for (UsbAccessory current : accessories) if (current.equals(accessory)) return true;
        return false;
    }
    @Override public void onCreate(Bundle state) { super.onCreate(state); connect(getIntent()); }
    @Override public void onNewIntent(Intent intent) { super.onNewIntent(intent); setIntent(intent); connect(intent); }
    private void connect(Intent intent) {
        try {
            UsbManager manager = (UsbManager) getSystemService(USB_SERVICE);
            UsbAccessory accessory = accessory(intent);
            if (accessory == null) {
                UsbAccessory[] accessories = manager.getAccessoryList();
                if (accessories != null) for (UsbAccessory current : accessories) if (supported(current)) { accessory = current; break; }
            }
            if (!attached(manager, accessory)) throw new IllegalStateException("Extend USB accessory isn't connected");
            if (manager.hasPermission(accessory)) {
                startForegroundService(new Intent(this, UsbAccessoryService.class).putExtra(UsbManager.EXTRA_ACCESSORY, accessory));
                Intent open = getPackageManager().getLaunchIntentForPackage(getPackageName());
                if (open != null) startActivity(open.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP));
                finish();
            } else if (RESULT.equals(intent.getAction())) {
                throw new IllegalStateException("USB access wasn't allowed");
            } else {
                // Original accessory is part of the immutable callback. Read
                // the real OS permission state rather than fill-in extras.
                Intent callback = new Intent(this, UsbAccessoryActivity.class).setAction(RESULT)
                    .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP)
                    .putExtra(UsbManager.EXTRA_ACCESSORY, accessory);
                PendingIntent result = PendingIntent.getActivity(this, 3403, callback,
                    PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
                manager.requestPermission(accessory, result);
            }
        } catch (Exception error) {
            android.util.Log.w("ExtendUsb", "USB connection unavailable", error);
            Toast.makeText(this, error.getMessage(), Toast.LENGTH_LONG).show(); finish();
        }
    }
}
