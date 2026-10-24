package computer.extend.android;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.pm.ServiceInfo;
import android.hardware.usb.UsbAccessory;
import android.hardware.usb.UsbManager;
import android.os.IBinder;
import android.os.ParcelFileDescriptor;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import org.json.JSONObject;

/** Native USB bulk workers. No encoded/encrypted video crosses the React bridge. */
public final class UsbAccessoryService extends Service {
    private static final String STOP = "computer.extend.android.STOP_USB";
    private volatile boolean running;
    private String id;
    private UsbAccessory accessory;
    private ParcelFileDescriptor descriptor;
    private FileInputStream input;
    private FileOutputStream output;
    private final BroadcastReceiver detached = new BroadcastReceiver() {
        @Override public void onReceive(Context context, Intent intent) {
            UsbAccessory removed = UsbAccessoryActivity.accessory(intent);
            if (UsbManager.ACTION_USB_ACCESSORY_DETACHED.equals(intent.getAction()) && removed != null && removed.equals(accessory)) stopSelf();
        }
    };
    @Override public void onCreate() {
        super.onCreate();
        NotificationManager manager = getSystemService(NotificationManager.class);
        manager.createNotificationChannel(new NotificationChannel("extend-usb", "USB connection", NotificationManager.IMPORTANCE_LOW));
        Intent open = getPackageManager().getLaunchIntentForPackage(getPackageName());
        PendingIntent launch = PendingIntent.getActivity(this, 3403, open, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        PendingIntent stop = PendingIntent.getService(this, 3404, new Intent(this, UsbAccessoryService.class).setAction(STOP), PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        Notification notification = new Notification.Builder(this, "extend-usb").setSmallIcon(android.R.drawable.ic_menu_view)
            .setContentTitle("extend.computer").setContentText("USB connection active").setContentIntent(launch)
            .setOngoing(true).addAction(new Notification.Action.Builder(null, "Disconnect", stop).build()).build();
        startForeground(3403, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE);
        if (android.os.Build.VERSION.SDK_INT >= 33) registerReceiver(detached, new IntentFilter(UsbManager.ACTION_USB_ACCESSORY_DETACHED), Context.RECEIVER_NOT_EXPORTED);
        else registerReceiver(detached, new IntentFilter(UsbManager.ACTION_USB_ACCESSORY_DETACHED));
    }
    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent == null || STOP.equals(intent.getAction())) { stopSelf(); return START_NOT_STICKY; }
        synchronized (this) {
            if (running) return START_NOT_STICKY;
            accessory = UsbAccessoryActivity.accessory(intent);
            running = true;
        }
        Thread setup = new Thread(() -> {
            try {
                UsbManager manager = getSystemService(UsbManager.class);
                if (!UsbAccessoryActivity.attached(manager, accessory) || !manager.hasPermission(accessory)) throw new IllegalStateException("USB accessory permission unavailable");
                NativeEngine.identity(this);
                synchronized (this) {
                    if (!running) return;
                    descriptor = manager.openAccessory(accessory);
                    if (descriptor == null) throw new IllegalStateException("Couldn't open USB accessory");
                    input = new FileInputStream(descriptor.getFileDescriptor());
                    output = new FileOutputStream(descriptor.getFileDescriptor());
                    id = new JSONObject(NativeEngine.attachUsb(accessory.getSerial())).getString("id");
                }
                String session = id;
                Thread reader = new Thread(() -> read(session), "extend-usb-read");
                Thread writer = new Thread(() -> write(session), "extend-usb-write");
                reader.setDaemon(true); writer.setDaemon(true); reader.start(); writer.start();
                android.util.Log.i("ExtendUsb", "Accessory ready: " + NativeEngine.usbState());
            } catch (Exception error) { failed(error); }
        }, "extend-usb-open");
        setup.setDaemon(true); setup.start();
        return START_NOT_STICKY;
    }
    private void read(String session) {
        byte[] bytes = new byte[16384];
        try {
            while (running) {
                int length = input.read(bytes);
                if (length < 0) throw new java.io.EOFException("USB accessory disconnected");
                int offset = 0;
                while (running && offset < length) {
                    int count = NativeEngine.receiveUsb(session, bytes, offset, length - offset);
                    if (count < 0 || count > length - offset) throw new IllegalStateException("USB framing failed");
                    if (count == 0) Thread.sleep(1); else offset += count;
                }
            }
        } catch (Exception error) { failed(error); }
    }
    private void write(String session) {
        try {
            long logged = android.os.SystemClock.elapsedRealtime();
            while (running) {
                byte[] frame = NativeEngine.nextUsbFrame(session);
                if (frame == null) Thread.sleep(1); else output.write(frame);
                long now = android.os.SystemClock.elapsedRealtime();
                if (now - logged >= 3000) { android.util.Log.i("ExtendUsb", "Accessory stats: " + NativeEngine.usbState()); logged = now; }
            }
        } catch (Exception error) { failed(error); }
    }
    private void failed(Exception error) {
        if (running) android.util.Log.w("ExtendUsb", "Accessory stopped", error);
        stopSelf();
    }
    @Override public void onDestroy() {
        synchronized (this) {
            running = false;
            if (id != null) NativeEngine.stopUsb(id);
            // FileInputStream.close uses Android's asynchronous-close monitor
            // to signal a blocked read; stop never waits on a bulk read in UI.
            try { if (input != null) input.close(); } catch (Exception ignored) { }
            try { if (output != null) output.close(); } catch (Exception ignored) { }
            try { if (descriptor != null) descriptor.close(); } catch (Exception ignored) { }
        }
        unregisterReceiver(detached); super.onDestroy();
    }
    @Override public IBinder onBind(Intent intent) { return null; }
}
