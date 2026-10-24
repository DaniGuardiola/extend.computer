package computer.extend.android;

import android.content.Context;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.concurrent.atomic.AtomicBoolean;
import org.json.JSONArray;
import org.json.JSONObject;

/** Bounded pinned status checks. Never pairs, sends input, or changes saved routes. */
public final class DeviceAvailability {
    private static final AtomicBoolean checking = new AtomicBoolean();
    private DeviceAvailability() {}

    public static JSONObject read(Context context, String peer) throws Exception {
        if (!checking.compareAndSet(false, true)) return status("checking", "");
        try {
            NativeEngine.identity(context);
            if (!known(peer)) return status("unavailable", "This device is no longer paired.");
            JSONObject usb = new JSONObject(NativeEngine.usbState());
            if (usbSelected(usb, peer)) {
                // No LAN fallback after selecting this carrier, even while its
                // Hello is pending. Raw attachment alone never means Online.
                try {
                    JSONObject presence = new JSONObject(NativeEngine.resolveUsbPresence(peer, usb.getString("id")));
                    if (!known(peer)) return status("unavailable", "This device is no longer paired.");
                    return status(presence.getBoolean("receiving") ? "online" : "receiving_off", "").put("route", "usb");
                } catch (Exception unavailable) {
                    return status("unavailable", "Couldn't verify the USB connection. Check connection to retry.").put("route", "usb");
                }
            }
            String saved = context.getSharedPreferences("engine-peers", 0).getString(peer, null);
            JSONObject metadata = saved == null ? new JSONObject() : new JSONObject(saved);
            String address = metadata.optString("address", "");
            String host = new JSONObject(NativeEngine.peerHosts()).optString(peer, "");
            List<String> endpoints = endpoints(address, host);
            try {
                JSONObject presence = new JSONObject(query(endpoints, endpoint -> NativeEngine.resolvePresence(endpoint, peer)));
                if (!known(peer)) return status("unavailable", "This device is no longer paired.");
                return status(presence.getBoolean("receiving") ? "online" : "receiving_off", "");
            } catch (Exception unreachable) {
                try {
                    JSONObject presence = new JSONObject(NativeEngine.resolveAccountPresence(peer, false));
                    if (!known(peer)) return status("unavailable", "This device is no longer paired.");
                    return status(presence.getBoolean("receiving") ? "online" : "receiving_off", "").put("route", "internet");
                } catch (Exception internet) { /* Preserve local verification failure semantics. */ }
                // Presence is separate from active control/display sessions. A generic
                // probe can be rejected while a peer is busy, so never poll that path.
                boolean offline = offline(unreachable);
                return status(offline ? "offline" : "unavailable", offline
                    ? "Couldn't reach this device on the local network."
                    : "Couldn't verify availability. Check connection to retry.");
            }
        } finally { checking.set(false); }
    }
    static boolean usbSelected(JSONObject snapshot, String peer) {
        return "connected".equals(snapshot.optString("state")) && peer.equals(snapshot.optString("peer"))
            && snapshot.optString("id").matches("[0-9]{1,20}");
    }

    // A successful Check connection saves its newly authenticated route here.
    // The engine host may still refer to the original visual-pairing address.
    // Hints select at most two endpoints; only a pinned reply establishes status.
    static List<String> endpoints(String address, String engineHost) {
        LinkedHashSet<String> result = new LinkedHashSet<>();
        if (address != null) {
            address = address.trim();
            int colon = address.lastIndexOf(':');
            if (colon > 0) {
                String port = address.substring(colon + 1);
                boolean validPort = port.matches("[0-9]{1,5}")
                    && Integer.parseInt(port) > 0 && Integer.parseInt(port) <= 65535;
                String savedHost = address.substring(0, colon);
                boolean bracketed = savedHost.startsWith("[") && savedHost.endsWith("]");
                if (validPort && (bracketed || savedHost.indexOf(':') < 0)) addHost(result, savedHost);
            }
        }
        addHost(result, engineHost);
        return new ArrayList<>(result);
    }
    private static void addHost(LinkedHashSet<String> endpoints, String host) {
        if (host == null) return;
        host = host.trim();
        if (host.startsWith("[") && host.endsWith("]")) host = host.substring(1, host.length() - 1);
        if (host.isEmpty() || host.contains("[") || host.contains("]")) return;
        host = host.toLowerCase(Locale.ROOT);
        endpoints.add((host.contains(":") ? "[" + host + "]" : host) + ":48178");
    }
    interface PresenceQuery { String read(String endpoint) throws Exception; }
    static String query(List<String> endpoints, PresenceQuery reader) throws Exception {
        Exception failed = null;
        for (String endpoint : endpoints) {
            try { return reader.read(endpoint); }
            catch (Exception error) {
                // A verification failure must not become an Offline claim just
                // because another stale hint subsequently refused a connection.
                if (failed == null || offline(failed)) failed = error;
            }
        }
        if (failed != null) throw failed;
        throw new IllegalStateException("Device address unavailable");
    }
    private static boolean offline(Exception error) {
        String message = error.getMessage();
        return message != null && message.contains("Could not reach this device's availability service");
    }

    private static boolean known(String peer) throws Exception {
        JSONArray peers = new JSONArray(NativeEngine.pairedPeers());
        for (int i = 0; i < peers.length(); i++) if (peer.equals(peers.getString(i))) return true;
        return false;
    }
    private static JSONObject status(String state, String reason) throws Exception {
        return new JSONObject().put("state", state).put("reason", reason);
    }
}
