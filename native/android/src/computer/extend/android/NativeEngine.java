package computer.extend.android;

import android.content.Context;
import java.io.File;
import java.util.Arrays;
import org.json.JSONObject;
import org.json.JSONArray;

/** Native session engine facade. Only public identity metadata reaches JS. */
public final class NativeEngine {
    public static native String attachUsb(String peer);
    public static native String usbState();
    public static native String resolveUsbPresence(String peer, String id);
    public static native String probeUsbPeer(String peer, String id);
    public static native String stopUsb(String id);
    public static native int receiveUsb(String id, byte[] bytes, int offset, int length);
    public static native byte[] nextUsbFrame(String id);
    public static native String startDisplaySender(String peer, String address, String config);
    public static native String displaySenderState();
    public static native String configureDisplaySender(String id, String config);
    public static native String pushDisplayFrame(String id, byte[] bytes, long timestampNs, boolean keyframe);
    public static native String stopDisplaySender(String id);
    public static native String configureInputKeyboardAvailable(boolean enabled);
    public static native String configureTextReceiver(boolean enabled);
    public static native String configureInputReceiver(boolean enabled, double width, double height);
    public static native String pollInputReceiver();
    static { System.loadLibrary("extend_android_engine"); }
    private static JSONObject identity;
    private NativeEngine() {}

    private static native String initialize(byte[] seed, String root, Context context);
    public static native String accountProof(String serverKey, String challenge);
    public static native String replaceAccountPeers(String scope, String directory);
    public static native String clearAccountPeers();
    public static native String accountPeerNames();
    // Native account owner only: credentials never enter the Expo/React bridge.
    public static native String configureAccountRelay(String server, String scope, String device, String token, String fingerprint);
    public static native String resolveAccountPresence(String peer, boolean refresh);
    public static native String probeAccountPeer(String peer);
    public static native String updateLocalDeviceName(String name);
    public static native String pairedPeers();
    public static native String pairCode(String address, String code);
    public static native String startManualPairing(String id, String address, String code, String name);
    public static native String manualPairingState(String id);
    public static native String stopManualPairing(String id);
    public static native String startVisualPairing(String address, String name, String localName);
    public static native String visualPairingState(String id);
    public static native String respondVisualPairing(String id, String request, boolean allow);
    public static native String stopVisualPairing(String id);
    public static native String probePeer(String address, String peer);
    public static native String forgetPeer(String peer);
    public static native String notifyUnpair(String peer, String address, String operation);
    public static native String removedPeers();
    public static native String dismissRemovedPeer(String peer);
    public static native String startIncoming(String name);
    public static native String peerNames();
    public static native String peerHosts();
    public static native String startDisplayReceiver(String capabilities);
    public static native String stopDisplayReceiver();
    public static native String displayState();
    public static native String displayReceiverSettings(String id);
    public static native String requestDisplayReceiverSettings(String id, long revision, String settings);
    public static native String displayInputState(String id);
    public static native String setDisplayInputEnabled(String id, boolean enabled);
    public static native String displayInputCommand(String id, long revision, String command);
    public static native String attachDisplay(String id);
    public static native byte[] nextDisplayFrame(String id);
    public static native byte[] nextDisplayCursor(String id);
    public static native long createCursorCompositor(String id, android.view.Surface surface, long epoch, long revision,
        int rasterWidth, int rasterHeight, int logicalWidth, int logicalHeight,
        int holderWidth, int holderHeight, int fittedWidth, int fittedHeight, boolean trace);
    public static native int updateCursorCompositor(long handle, long epoch, long revision, android.graphics.Bitmap bitmap,
        long shapeRevision, double width, double height, double hotspotX, double hotspotY,
        long pointRevision, long sequence, double x, double y, boolean visible, long receivedMs, long nowMs);
    public static native void retireCursorCompositor(long handle);
    public static native void closeCursorCompositor(long handle);
    public static native String startControl(String peer, String address);
    public static native String controlState();
    public static native String controlEvent(String id, String event);
    public static native String stopControl(String id);
    public static native String displayConfigured(String id, long revision);
    public static native String displayRendered(String id, long revision, long presentationUs, long deviceNs, long callbackDeviceNs);
    public static native String cancelDisplay(String id);
    public static native String resolvePresence(String address, String peer);
    public static native String startReceiving(String name);
    public static native String receivingState();
    public static native String stopReceiving();
    public static native String startPresence(String name);
    public static native String stopPresence();
    public static native String incomingState();
    public static native String respondIncoming(String request, boolean allow);
    public static native String stopIncoming();
    public static String localName(Context context) {
        String saved = context.getSharedPreferences("engine-local", 0).getString("name", null);
        if (saved != null) return saved;
        return DeviceSettings.systemName(context);
    }
    public static void setLocalName(Context context, String name) {
        name = name.trim();
        if (name.isEmpty() || name.getBytes(java.nio.charset.StandardCharsets.UTF_8).length > 100 || name.codePoints().anyMatch(Character::isISOControl))
            throw new IllegalArgumentException("Use a shorter name without control characters");
        if (!context.getSharedPreferences("engine-local",0).edit().putString("name",name).commit()) throw new IllegalStateException("Couldn't save device name");
        updateLocalDeviceName(localName(context));
    }

    public static JSONObject pair(Context context, String address, String code, String name) throws Exception {
        identity(context);
        if (name.trim().isEmpty() || name.length() > 128) throw new IllegalArgumentException("Enter a device name");
        String peer = pairCode(address.trim(), code);
        JSONObject metadata = new JSONObject().put("address", address.trim()).put("name", name.trim());
        if (!context.getSharedPreferences("engine-peers", 0).edit().putString(peer, metadata.toString()).commit())
            throw new IllegalStateException("Paired device, but its address could not be saved");
        return metadata.put("peer", peer);
    }

    public static JSONArray devices(Context context) throws Exception {
        identity(context);
        JSONArray peers = new JSONArray(pairedPeers()), result = new JSONArray();
        JSONObject names = new JSONObject(peerNames());
        JSONObject account = new JSONObject(accountPeerNames());
        for (int i = 0; i < peers.length(); i++) {
            String peer = peers.getString(i);
            String saved = context.getSharedPreferences("engine-peers", 0).getString(peer, null);
            JSONObject item = PeerAddress.metadata(saved, account.optString(peer,names.optString(peer, "Device " + peer.substring(0, 8))));
            String authenticated = names.optString(peer, account.optString(peer, ""));
            result.put(DeviceSettings.decorate(item.put("peer", peer).put("trustSource",account.has(peer)?"account":"manual"), authenticated, DeviceSettings.label(context, peer)));
        }
        return result;
    }

    public static synchronized JSONObject check(Context context, String peer) throws Exception {
        identity(context);
        // The cable selects an epoch, not an authenticated connection. Its
        // presence and probe must prove the pinned peer before reporting USB.
        JSONObject usb = new JSONObject(usbState());
        if (DeviceAvailability.usbSelected(usb, peer)) {
            JSONObject remote = new JSONObject(resolveUsbPresence(peer, usb.getString("id")));
            if (!remote.getBoolean("receiving")) throw new IllegalStateException("This device isn't accepting connections");
            JSONObject verified = new JSONObject(probeUsbPeer(peer, remote.getString("carrier_id")));
            DeviceSettings.peerInfo(context, peer);
            return verified;
        }
        DeviceSettings.peerInfo(context, peer);
        String saved = context.getSharedPreferences("engine-peers", 0).getString(peer, null);
        JSONObject metadata = saved == null ? new JSONObject() : new JSONObject(saved);
        String address = metadata.optString("address", "");
        String host = new JSONObject(peerHosts()).optString(peer, "");
        JSONObject presence = null;
        Exception presenceError = null;
        try {
            presence = new JSONObject(DeviceAvailability.query(DeviceAvailability.endpoints(address, host),
                candidate -> resolvePresence(candidate, peer)));
        } catch (Exception error) { presenceError = error; }
        if (presence == null) {
            NearbyDiscovery discovery = new NearbyDiscovery(context, true);
            discovery.start();
            try {
                // Bounded browse; each candidate still has to prove the exact paired identity.
                Thread.sleep(2000);
                JSONArray candidates = discovery.snapshot().getJSONArray("candidates");
                java.util.LinkedHashSet<String> endpoints = new java.util.LinkedHashSet<>();
                for (int pass = 0; pass < 2; pass++) for (int i = 0; i < candidates.length(); i++) {
                    JSONObject candidate = candidates.getJSONObject(i);
                    boolean hintMatches = candidate.getString("id").contains(peer.substring(0, 12));
                    if (hintMatches != (pass == 0)) continue;
                    JSONArray addresses = candidate.getJSONArray("addresses");
                    for (int j = 0; j < addresses.length() && endpoints.size() < 8; j++) endpoints.add(addresses.getString(j));
                }
                for (String candidate : endpoints) {
                    try { presence = new JSONObject(resolvePresence(candidate, peer)); break; }
                    catch (Exception error) { presenceError = error; }
                }
            } finally { discovery.stop(); }
        }
        if (presence != null) {
            if (!presence.getBoolean("receiving")) throw new IllegalStateException("This device isn't accepting connections");
            address = presence.getString("address");
            return PeerAddress.verifyAndSave(peer, address, addressStore(context), NativeEngine::probePeer);
        }
        // Internet membership is a route hint; both presence and probe still
        // authenticate the exact locally permitted engine identity.
        try {
            JSONObject remote = new JSONObject(resolveAccountPresence(peer, true));
            if (!remote.getBoolean("receiving")) throw new IllegalStateException("This device isn't accepting connections");
            JSONObject verified = new JSONObject(probeAccountPeer(peer));
            DeviceSettings.peerInfo(context, peer);
            return verified;
        } catch (Exception internet) {
            if ("This device isn't accepting connections".equals(internet.getMessage())) throw internet;
        }
        if (!address.isEmpty()) {
            JSONObject result = new JSONObject(probePeer(address, peer));
            DeviceSettings.peerInfo(context, peer);
            return result;
        }
        if (presenceError != null) throw presenceError;
        throw new IllegalStateException("Device address unavailable. Make sure both devices are on the same network.");
    }

    public static synchronized JSONObject setPeerAddress(Context context, String peer, String address) throws Exception {
        identity(context);
        PeerAddress.save(peer, address, addressStore(context));
        return DeviceSettings.peerInfo(context, peer);
    }
    private static PeerAddress.Store addressStore(Context context) {
        return new PeerAddress.Store() {
            @Override public boolean paired(String id) throws Exception {
                JSONArray peers = new JSONArray(pairedPeers());
                for (int i = 0; i < peers.length(); i++) if (id.equals(peers.getString(i))) return true;
                return false;
            }
            @Override public String read(String id) {
                return context.getSharedPreferences("engine-peers", 0).getString(id, null);
            }
            @Override public boolean commit(String id, String metadata) {
                return context.getSharedPreferences("engine-peers", 0).edit().putString(id, metadata).commit();
            }
        };
    }

    public static synchronized void forget(Context context, String peer) throws Exception {
        identity(context);
        String saved = context.getSharedPreferences("engine-peers", 0).getString(peer, null);
        String operation = new JSONObject(forgetPeer(peer)).getString("operation");
        // A hint is optional and uses only the previously saved literal endpoint.
        if (saved != null) {
            try { notifyUnpair(peer, new JSONObject(saved).getString("address"), operation); }
            catch (Exception ignored) { /* Local revocation already completed. */ }
        }
        if (!context.getSharedPreferences("engine-peers", 0).edit().remove(peer).commit())
            throw new IllegalStateException("Device unpaired, but its saved address could not be removed");
        DeviceSettings.clearLabel(context, peer);
    }

    public static synchronized JSONObject identity(Context context) throws Exception {
        if (identity == null) {
            byte[] seed = DeviceIdentity.load(context);
            try {
                Context application = context.getApplicationContext();
                JSONObject initialized = new JSONObject(initialize(seed,
                    new File(context.getNoBackupFilesDir(), "engine").getAbsolutePath(),
                    application != null ? application : context));
                updateLocalDeviceName(localName(context));
                identity = initialized;
            } finally { Arrays.fill(seed, (byte) 0); }
        }
        return new JSONObject(identity.toString());
    }
}
