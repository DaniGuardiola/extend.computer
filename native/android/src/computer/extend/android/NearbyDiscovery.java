package computer.extend.android;

import android.content.Context;
import android.net.nsd.NsdManager;
import android.net.nsd.NsdServiceInfo;
import android.net.wifi.WifiManager;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.os.ext.SdkExtensions;
import java.net.InetAddress;
import java.nio.charset.StandardCharsets;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import org.json.JSONArray;
import org.json.JSONObject;

/** Foreground-only DNS-SD browsing. Discovery never grants identity or trust. */
public final class NearbyDiscovery {
    private static final String[] TYPES = {"_extendpair._tcp.", "_extendprobe._tcp."};
    private final String[] types;
    private final Context context;
    private final NsdManager manager;
    private final Handler main = new Handler(Looper.getMainLooper());
    private final List<NsdManager.DiscoveryListener> browsers = new ArrayList<>();
    private final Map<String, NsdManager.ServiceInfoCallback> updates = new HashMap<>();
    private final Map<String, NsdServiceInfo> found = new LinkedHashMap<>();
    private final Map<String, JSONObject> candidates = new LinkedHashMap<>();
    private final ArrayDeque<NsdServiceInfo> queue = new ArrayDeque<>();
    private boolean requested, running, resolving;
    private long generation;
    private String state = "idle";
    private WifiManager.MulticastLock multicast;

    public NearbyDiscovery(Context context) { this(context, false); }
    public NearbyDiscovery(Context context, boolean presenceOnly) {
        types = presenceOnly ? new String[]{"_extendpresence._tcp."} : TYPES;
        this.context = context.getApplicationContext();
        manager = (NsdManager) this.context.getSystemService(Context.NSD_SERVICE);
    }
    private void dispatch(Runnable action) {
        main.post(() -> { synchronized (NearbyDiscovery.this) { action.run(); } });
    }
    public void start() { dispatch(() -> { requested = true; begin(); }); }
    public void stop() { dispatch(() -> { requested = false; end(); }); }
    public void suspend() { dispatch(this::end); }
    public void resume() { dispatch(() -> { if (requested) begin(); }); }
    public synchronized JSONObject snapshot() {
        JSONArray items = new JSONArray();
        for (JSONObject item : candidates.values()) items.put(item);
        try { return new JSONObject().put("state", state).put("candidates", items); }
        catch (org.json.JSONException impossible) { throw new IllegalStateException(impossible); }
    }
    private static String key(NsdServiceInfo service) { return service.getServiceType() + "/" + service.getServiceName(); }
    private boolean current(long epoch) { return running && generation == epoch; }
    private void begin() {
        if (running) return;
        if (manager == null) { state = "failed"; return; }
        running = true; state = "searching"; final long epoch = ++generation;
        try {
            boolean needsLock = Build.VERSION.SDK_INT < 33 || (Build.VERSION.SDK_INT == 33 && SdkExtensions.getExtensionVersion(33) < 7);
            if (needsLock) {
                WifiManager wifi = (WifiManager) context.getSystemService(Context.WIFI_SERVICE);
                if (wifi != null) { multicast = wifi.createMulticastLock("extend-nearby"); multicast.setReferenceCounted(false); multicast.acquire(); }
            }
            for (String type : types) {
                NsdManager.DiscoveryListener listener = new NsdManager.DiscoveryListener() {
                    public void onDiscoveryStarted(String type) {}
                    public void onDiscoveryStopped(String type) {}
                    public void onStartDiscoveryFailed(String type, int error) { dispatch(() -> { if (current(epoch)) { end(); state = "failed"; } }); }
                    public void onStopDiscoveryFailed(String type, int error) {}
                    public void onServiceFound(NsdServiceInfo service) { dispatch(() -> {
                        if (!current(epoch) || found.size() >= 64 || found.containsKey(key(service))) return;
                        found.put(key(service), service);
                        if (Build.VERSION.SDK_INT >= 34) watch(service, epoch);
                        else { queue.add(service); resolveNext(epoch); }
                    }); }
                    public void onServiceLost(NsdServiceInfo service) { dispatch(() -> {
                        if (!current(epoch)) return;
                        String id = key(service); found.remove(id); candidates.remove(id);
                        queue.removeIf(queued -> id.equals(key(queued)));
                        NsdManager.ServiceInfoCallback callback = updates.remove(id);
                        if (callback != null && Build.VERSION.SDK_INT >= 34) {
                            try { manager.unregisterServiceInfoCallback(callback); } catch (RuntimeException ignored) {}
                        }
                    }); }
                };
                browsers.add(listener);
                manager.discoverServices(type, NsdManager.PROTOCOL_DNS_SD, listener);
            }
        } catch (RuntimeException failed) { end(); state = "failed"; }
    }
    @android.annotation.TargetApi(34)
    private void watch(NsdServiceInfo service, long epoch) {
        String id = key(service);
        NsdManager.ServiceInfoCallback callback = new NsdManager.ServiceInfoCallback() {
            public void onServiceInfoCallbackRegistrationFailed(int error) { dispatch(() -> { if (current(epoch)) { updates.remove(id); found.remove(id); } }); }
            public void onServiceInfoCallbackUnregistered() {}
            public void onServiceLost() { dispatch(() -> { if (current(epoch)) candidates.remove(id); }); }
            public void onServiceUpdated(NsdServiceInfo resolved) { dispatch(() -> { if (current(epoch) && found.containsKey(id)) accept(id, resolved); }); }
        };
        updates.put(id, callback);
        try { manager.registerServiceInfoCallback(service, main::post, callback); }
        catch (RuntimeException failed) { updates.remove(id); found.remove(id); }
    }
    @SuppressWarnings("deprecation")
    private void resolveNext(long epoch) {
        if (!current(epoch) || resolving) return;
        NsdServiceInfo service;
        do { service = queue.poll(); } while (service != null && !found.containsKey(key(service)));
        if (service == null) return;
        String id = key(service); resolving = true;
        try {
            manager.resolveService(service, new NsdManager.ResolveListener() {
                public void onResolveFailed(NsdServiceInfo service, int error) { dispatch(() -> {
                    if (!current(epoch)) return; resolving = false; found.remove(id); resolveNext(epoch);
                }); }
                public void onServiceResolved(NsdServiceInfo resolved) { dispatch(() -> {
                    if (!current(epoch)) return; resolving = false;
                    if (found.containsKey(id)) accept(id, resolved);
                    resolveNext(epoch);
                }); }
            });
        } catch (RuntimeException failed) { resolving = false; found.remove(id); resolveNext(epoch); }
    }
    private static String attribute(NsdServiceInfo service, String name) {
        byte[] bytes = service.getAttributes().get(name);
        return bytes == null || bytes.length > 128 ? "" : new String(bytes, StandardCharsets.UTF_8);
    }
    @SuppressWarnings("deprecation")
    private void accept(String id, NsdServiceInfo service) {
        String capability = attribute(service, "capability");
        boolean presence = service.getServiceType().startsWith("_extendpresence.");
        boolean pairing = service.getServiceType().startsWith("_extendpair.");
        if (!"1".equals(attribute(service, "version")) || !(presence ? "presence" : pairing ? "pairing" : "probe").equals(capability)) { candidates.remove(id); return; }
        String name = attribute(service, "name").trim();
        if (!presence && (name.isEmpty() || name.getBytes(StandardCharsets.UTF_8).length > 100 || name.codePoints().anyMatch(Character::isISOControl))) { candidates.remove(id); return; }
        List<InetAddress> hosts = Build.VERSION.SDK_INT >= 34 ? service.getHostAddresses() : java.util.Collections.singletonList(service.getHost());
        JSONArray addresses = new JSONArray();
        if (service.getPort() < 1 || service.getPort() > 65535) return;
        for (InetAddress host : hosts) {
            if (host == null || host.isAnyLocalAddress() || host.isLoopbackAddress() || host.isMulticastAddress()) continue;
            String address = host.getHostAddress();
            // Scoped IPv6 requires binding the socket to its Network. Do not offer an unusable endpoint.
            if (address == null || address.contains("%")) continue;
            addresses.put((address.contains(":") ? "[" + address + "]" : address) + ":" + service.getPort());
            if (addresses.length() >= 8) break;
        }
        if (addresses.length() == 0) { candidates.remove(id); return; }
        try { candidates.put(id, new JSONObject().put("id", id).put("name", name).put("addresses", addresses).put("pairing", pairing)); }
        catch (org.json.JSONException impossible) { throw new IllegalStateException(impossible); }
    }
    private void end() {
        running = false; ++generation; state = "idle";
        for (NsdManager.DiscoveryListener listener : browsers) {
            try { manager.stopServiceDiscovery(listener); } catch (RuntimeException ignored) {}
        }
        browsers.clear();
        if (Build.VERSION.SDK_INT >= 34) for (NsdManager.ServiceInfoCallback callback : updates.values()) {
            try { manager.unregisterServiceInfoCallback(callback); } catch (RuntimeException ignored) {}
        }
        updates.clear(); found.clear(); candidates.clear(); queue.clear(); resolving = false;
        if (multicast != null) { if (multicast.isHeld()) multicast.release(); multicast = null; }
    }
}
