package computer.extend.android;

import android.content.Context;
import android.content.pm.PackageInfo;
import java.nio.charset.StandardCharsets;
import org.json.JSONArray;
import org.json.JSONObject;

/** Local presentation preferences. Labels never change a peer identity or its authenticated name. */
public final class DeviceSettings {
    private DeviceSettings() {}

    static String normalize(String raw) {
        String name = raw.trim();
        if (name.getBytes(StandardCharsets.UTF_8).length > 100 || name.codePoints().anyMatch(Character::isISOControl))
            throw new IllegalArgumentException("Use a name up to 100 bytes without control characters.");
        return name;
    }
    static String systemName(Context context) {
        String name = android.provider.Settings.Global.getString(context.getContentResolver(), "device_name");
        return name == null || name.trim().isEmpty() ? android.os.Build.MANUFACTURER + " " + android.os.Build.MODEL : name.trim();
    }
    public static JSONObject localInfo(Context context) throws Exception {
        JSONObject identity = NativeEngine.identity(context);
        PackageInfo installed = context.getPackageManager().getPackageInfo(context.getPackageName(), 0);
        return new JSONObject().put("name", NativeEngine.localName(context)).put("defaultName", systemName(context))
            .put("customName", context.getSharedPreferences("engine-local", 0).contains("name"))
            .put("fingerprint", identity.getString("fingerprint"))
            .put("version", installed.versionName == null ? "" : installed.versionName)
            .put("build", Long.toString(installed.getLongVersionCode()));
    }
    public static JSONObject setLocal(Context context, String raw) throws Exception {
        NativeEngine.identity(context);
        String name = normalize(raw);
        if (name.isEmpty()) {
            if (!context.getSharedPreferences("engine-local", 0).edit().remove("name").commit())
                throw new IllegalStateException("Couldn't reset device name.");
            NativeEngine.updateLocalDeviceName(NativeEngine.localName(context));
        } else NativeEngine.setLocalName(context, name);
        // A native account refresh registers the new name using its existing identity proof.
        // Name persistence must still succeed if an unrelated account store needs recovery.
        try { Accounts.get(context).resume(); } catch (Exception ignored) { }
        return localInfo(context);
    }
    static JSONObject decorate(JSONObject metadata, String authenticatedName, String label) throws Exception {
        JSONObject result = new JSONObject(metadata.toString());
        String defaultName = metadata.getString("name");
        return result.put("defaultName", defaultName)
            .put("authenticatedName", authenticatedName.isEmpty() ? JSONObject.NULL : authenticatedName)
            .put("label", label == null ? JSONObject.NULL : label)
            .put("name", label == null ? defaultName : label);
    }
    static String label(Context context, String peer) {
        return context.getSharedPreferences("engine-labels", 0).getString(peer, null);
    }
    static void clearLabel(Context context, String peer) {
        if (!context.getSharedPreferences("engine-labels", 0).edit().remove(peer).commit())
            throw new IllegalStateException("Couldn't remove device label.");
    }
    public static JSONObject peerInfo(Context context, String peer) throws Exception {
        if (!AccountProtocol.hex(peer)) throw new IllegalArgumentException("Invalid device identity.");
        JSONArray devices = NativeEngine.devices(context);
        for (int i = 0; i < devices.length(); i++) {
            JSONObject item = devices.getJSONObject(i);
            if (peer.equals(item.getString("peer"))) return item;
        }
        throw new IllegalStateException("This device is no longer paired.");
    }
    public static synchronized JSONObject setPeerLabel(Context context, String peer, String raw) throws Exception {
        peerInfo(context, peer); // Do not create preferences for arbitrary untrusted fingerprints.
        String name = normalize(raw);
        android.content.SharedPreferences.Editor editor = context.getSharedPreferences("engine-labels", 0).edit();
        if (name.isEmpty()) editor.remove(peer); else editor.putString(peer, name);
        if (!editor.commit()) throw new IllegalStateException("Couldn't save device label.");
        return peerInfo(context, peer);
    }
}
