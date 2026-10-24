package computer.extend.android;

import android.content.Context;
import android.app.Activity;
import org.json.JSONObject;

/** One foreground visual operation. JS receives only public verification data. */
public final class VisualPairing {
    private String operation;
    private String savedPeer;
    private java.lang.ref.WeakReference<Activity> owner;

    public synchronized String start(Context context, String address, String name, Activity activity) throws Exception {
        if (activity == null || !activity.hasWindowFocus())
            throw new IllegalStateException("Return to the app to pair a device");
        NativeEngine.identity(context);
        if (operation != null) {
            String state = new JSONObject(NativeEngine.visualPairingState(operation)).getString("state");
            if (state.equals("connecting") || state.equals("verify"))
                throw new IllegalStateException("Pairing already in progress");
            stop();
        }
        JSONObject result = new JSONObject(NativeEngine.startVisualPairing(address.trim(), name.trim(), NativeEngine.localName(context)));
        operation = result.getString("id");
        owner = new java.lang.ref.WeakReference<>(activity);
        savedPeer = null;
        if (!activity.hasWindowFocus()) {
            stop();
            throw new IllegalStateException("Return to the app to pair a device");
        }
        return result.toString();
    }

    public synchronized String state(Context context, String id) throws Exception {
        require(id);
        JSONObject result = new JSONObject(NativeEngine.visualPairingState(id));
        if (result.getString("state").equals("paired")) {
            String peer = result.getString("peer");
            if (!peer.equals(savedPeer)) {
                // Trust has already been committed by mutual visual approval.
                // Save the selected route as a hint; pinned presence verifies it later.
                JSONObject metadata = new JSONObject().put("address", result.getString("address"))
                    .put("name", result.getString("name"));
                if (!context.getSharedPreferences("engine-peers", 0).edit()
                    .putString(peer, metadata.toString()).commit()) {
                    result.put("error", "Paired device, but its address could not be saved");
                } else {
                    savedPeer = peer;
                }
            }
        }
        return result.toString();
    }

    public synchronized void respond(String id, String request, boolean allow) throws Exception {
        require(id);
        Activity activity = owner == null ? null : owner.get();
        if (allow && (activity == null || !activity.hasWindowFocus())) {
            stop();
            throw new IllegalStateException("Return to the app to confirm pairing");
        }
        NativeEngine.respondVisualPairing(id, request, allow);
    }
    public synchronized void stop(String id) throws Exception { require(id); stop(); }
    public synchronized void stop() {
        if (operation != null) {
            try { NativeEngine.stopVisualPairing(operation); }
            finally { operation = null; savedPeer = null; owner = null; }
        }
    }
    private void require(String id) {
        if (operation == null || !operation.equals(id))
            throw new IllegalStateException("Pairing operation changed");
    }
}
