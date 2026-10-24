package computer.extend.android;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.net.nsd.NsdManager;
import android.net.nsd.NsdServiceInfo;
import android.os.IBinder;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import org.json.JSONObject;

/** User-enabled availability for authenticated peers, independent of React navigation. */
public final class ReceivingService extends Service {
    private static final String STOP = "computer.extend.android.STOP_RECEIVING";
    private static volatile ReceivingService active;
    private volatile String phase = "starting";
    private final ExecutorService worker = Executors.newSingleThreadExecutor();
    private NsdManager nsd;
    private NsdManager.RegistrationListener advertisement;
    public static boolean enabled(Context context) { return context.getSharedPreferences("engine-local", 0).getBoolean("receiving", false); }
    public static void enable(Context context, boolean enabled) {
        if (!context.getSharedPreferences("engine-local", 0).edit().putBoolean("receiving", enabled).commit()) throw new IllegalStateException("Couldn't save availability");
        if (!enabled) context.stopService(new Intent(context, ReceivingService.class));
        else {
            try { context.startForegroundService(new Intent(context, ReceivingService.class)); }
            catch (RuntimeException error) {
                context.getSharedPreferences("engine-local", 0).edit().putBoolean("receiving", false).commit();
                throw error;
            }
        }
    }
    public static void resume(Context context) {
        if (enabled(context) && active == null) {
            try { context.startForegroundService(new Intent(context, ReceivingService.class)); }
            catch (RuntimeException unavailable) { android.util.Log.w("ExtendReceiving", "Availability must be restarted from the app"); }
        }
    }
    public static JSONObject state(Context context) throws Exception {
        ReceivingService service = active;
        JSONObject snapshot = new JSONObject(NativeEngine.receivingState());
        return new JSONObject().put("enabled", enabled(context)).put("state", service == null ? "stopped" : service.phase)
            .put("port", service != null && "available".equals(service.phase) ? snapshot.getInt("port") : 0);
    }
    @Override public void onCreate() {
        super.onCreate();
        synchronized (ReceivingService.class) { active = this; }
        NotificationManager manager = (NotificationManager) getSystemService(NOTIFICATION_SERVICE);
        manager.createNotificationChannel(new NotificationChannel("extend-availability", "Device availability", NotificationManager.IMPORTANCE_LOW));
        Intent open = getPackageManager().getLaunchIntentForPackage(getPackageName());
        PendingIntent launch = PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        PendingIntent stop = PendingIntent.getService(this, 1, new Intent(this, ReceivingService.class).setAction(STOP), PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        Notification notification = new Notification.Builder(this, "extend-availability").setSmallIcon(android.R.drawable.ic_menu_view)
            .setContentTitle("extend.computer").setContentText("Available to paired devices").setContentIntent(launch)
            .setOngoing(true).addAction(new Notification.Action.Builder(null, "Stop", stop).build()).build();
        startForeground(3401, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE);
    }
    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        if ((intent != null && STOP.equals(intent.getAction())) || !enabled(this)) {
            getSharedPreferences("engine-local", 0).edit().putBoolean("receiving", false).commit(); stopSelf(); return START_NOT_STICKY;
        }
        worker.execute(() -> {
            try {
                JSONObject identity = NativeEngine.identity(this);
                synchronized (ReceivingService.class) {
                    if (active != this || !enabled(this) || "available".equals(phase)) return;
                    String name = NativeEngine.localName(this);
                    NativeEngine.startReceiving(name);
                    NativeEngine.startDisplayReceiver(MediaSession.capabilities(this).toString());
                    JSONObject presence = new JSONObject(NativeEngine.startPresence(name));
                    nsd = (NsdManager) getSystemService(NSD_SERVICE);
                    NsdServiceInfo info = new NsdServiceInfo();
                    info.setServiceName("extend-presence-" + identity.getString("fingerprint").substring(0,12));
                    info.setServiceType("_extendpresence._tcp."); info.setPort(presence.getInt("port"));
                    info.setAttribute("version", "1"); info.setAttribute("capability", "presence");
                    advertisement = new NsdManager.RegistrationListener() {
                        public void onServiceRegistered(NsdServiceInfo info) {}
                        public void onRegistrationFailed(NsdServiceInfo info, int error) {}
                        public void onServiceUnregistered(NsdServiceInfo info) {}
                        public void onUnregistrationFailed(NsdServiceInfo info, int error) {}
                    };
                    try { if (nsd != null) nsd.registerService(info, NsdManager.PROTOCOL_DNS_SD, advertisement); }
                    catch (RuntimeException unavailable) { advertisement = null; }
                    phase = "available";
                }
            } catch (Exception error) {
                synchronized (ReceivingService.class) {
                    if (active == this) { phase = "failed"; NativeEngine.stopDisplayReceiver(); NativeEngine.stopPresence(); NativeEngine.stopReceiving(); }
                }
                stopSelf();
            }
        });
        return START_NOT_STICKY;
    }
    @Override public void onDestroy() {
        synchronized (ReceivingService.class) {
            if (active == this) {
                active = null;
                if (advertisement != null && nsd != null) try { nsd.unregisterService(advertisement); } catch (RuntimeException ignored) {}
                advertisement = null; nsd = null;
                NativeEngine.stopDisplayReceiver(); NativeEngine.stopPresence(); NativeEngine.stopReceiving(); phase = "stopped";
            }
        }
        worker.shutdownNow(); super.onDestroy();
    }
    @Override public IBinder onBind(Intent intent) { return null; }
}
