package computer.extend.android.tests;

import android.app.Instrumentation;
import android.content.Context;
import android.os.Bundle;
import android.util.AtomicFile;
import java.io.File;
import java.io.FileOutputStream;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.security.KeyStore;
import java.security.MessageDigest;
import java.util.Arrays;
import org.json.JSONArray;
import org.json.JSONObject;

/** Exercises the installed app's JNI and Keystore in its actual target UID. */
public final class EngineInstrumentation extends Instrumentation {
    private Bundle arguments;
    @Override public void onCreate(Bundle arguments) { super.onCreate(arguments); this.arguments = arguments; start(); }
    private static void require(boolean value, String message) {
        if (!value) throw new AssertionError(message);
    }
    private static String hex(byte[] bytes) {
        StringBuilder result = new StringBuilder();
        for (byte value : bytes) result.append(String.format("%02x", value & 255));
        return result.toString();
    }
    private static byte[] unhex(String value) {
        byte[] bytes = new byte[value.length() / 2];
        for (int i = 0; i < bytes.length; i++) bytes[i] = (byte) Integer.parseInt(value.substring(i*2, i*2+2), 16);
        return bytes;
    }
    private static void write(AtomicFile file, byte[] bytes) throws Exception {
        FileOutputStream output = file.startWrite();
        try { output.write(bytes); file.finishWrite(output); }
        catch (Exception error) { file.failWrite(output); throw error; }
    }
    @Override public void onStart() {
        Bundle result = new Bundle();
        try {
            Context context = getTargetContext();
            Class<?> engine = context.getClassLoader().loadClass("computer.extend.android.NativeEngine");
            Method identityMethod = engine.getMethod("identity", Context.class);
            JSONObject identity = (JSONObject) identityMethod.invoke(null, context);
            require(identity.toString().equals(identityMethod.invoke(null, context).toString()), "Identity changed in-process");
            String publicKey = identity.getString("public_key"), fingerprint = identity.getString("fingerprint");
            require(publicKey.matches("[0-9a-f]{64}"), "Invalid public key");
            require(fingerprint.equals(hex(MessageDigest.getInstance("SHA-256").digest(unhex(publicKey)))), "Shared engine fingerprint mismatch");
            KeyStore store = KeyStore.getInstance("AndroidKeyStore"); store.load(null);
            require(store.getKey("computer.extend.identity.wrap.v1", null).getEncoded() == null, "Wrapping key is exportable");
            AtomicFile file = new AtomicFile(new File(context.getNoBackupFilesDir(), "engine/identity.v1"));
            byte[] original = file.readFully();
            require(original.length == 61, "Unexpected encrypted identity record");
            Class<?> storage = context.getClassLoader().loadClass("computer.extend.android.DeviceIdentity");
            Method load = storage.getDeclaredMethod("load", Context.class); load.setAccessible(true);
            byte[] corrupt = original.clone(); corrupt[corrupt.length-1] ^= 1;
            try {
                write(file, corrupt);
                try {
                    byte[] unexpected = (byte[]) load.invoke(null, context);
                    Arrays.fill(unexpected, (byte) 0);
                    throw new AssertionError("Tampered identity was accepted");
                } catch (InvocationTargetException rejected) {
                    require(rejected.getCause() instanceof java.security.GeneralSecurityException, "Tamper failed for unrelated reason");
                }
                require(Arrays.equals(corrupt, file.readFully()), "Corrupt identity was silently replaced");
            } finally { write(file, original); }
            Method proof = engine.getMethod("accountProof", String.class, String.class);
            String zeros = new String(new char[64]).replace('\0', '0');
            String valid = (String) proof.invoke(null, "09" + zeros.substring(2), zeros);
            require(valid.matches("[0-9a-f]{64}"), "Native account proof unavailable");
            try { proof.invoke(null, zeros, zeros); throw new AssertionError("Non-contributory server key was accepted"); }
            catch (InvocationTargetException rejected) { require(rejected.getCause() instanceof IllegalStateException, "Unexpected native exception"); }
            new JSONArray((String) engine.getMethod("pairedPeers").invoke(null));
            if (arguments != null && arguments.containsKey("pairAddress")) {
                String expected = arguments.getString("pairPeer");
                if (arguments.containsKey("discoveryName")) {
                    android.content.Intent launch = context.getPackageManager().getLaunchIntentForPackage(context.getPackageName());
                    require(launch != null, "App launch unavailable");
                    context.startActivity(launch.addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK | android.content.Intent.FLAG_ACTIVITY_CLEAR_TASK));
                    Thread.sleep(1000);
                    Class<?> discoveryClass = context.getClassLoader().loadClass("computer.extend.android.NearbyDiscovery");
                    Object discovery = discoveryClass.getConstructor(Context.class).newInstance(context);
                    Method snapshot = discoveryClass.getMethod("snapshot");
                    discoveryClass.getMethod("start").invoke(discovery);
                    String discoveredAddress = null;
                    try {
                        long end = android.os.SystemClock.elapsedRealtime() + 20000;
                        while (android.os.SystemClock.elapsedRealtime() < end && discoveredAddress == null) {
                            JSONArray nearby = ((JSONObject) snapshot.invoke(discovery)).getJSONArray("candidates");
                            for (int i = 0; i < nearby.length(); i++) {
                                JSONObject item = nearby.getJSONObject(i);
                                if (arguments.getString("discoveryName").equals(item.getString("name"))) {
                                    require(item.getBoolean("pairing"), "Wrong advertised capability");
                                    discoveredAddress = item.getJSONArray("addresses").getString(0);
                                }
                            }
                            if (discoveredAddress == null) Thread.sleep(100);
                        }
                        require(discoveredAddress != null, "Native NSD did not resolve expected Mac peer");
                        arguments.putString("pairAddress", discoveredAddress);
                    } finally {
                        discoveryClass.getMethod("stop").invoke(discovery);
                        long end = android.os.SystemClock.elapsedRealtime() + 2000;
                        while (android.os.SystemClock.elapsedRealtime() < end && !"idle".equals(((JSONObject) snapshot.invoke(discovery)).getString("state"))) Thread.sleep(20);
                        JSONObject stopped = (JSONObject) snapshot.invoke(discovery);
                        require("idle".equals(stopped.getString("state")) && stopped.getJSONArray("candidates").length() == 0, "Discovery retained resources after stop");
                    }
                    result.putString("discovery", "passed");
                }
                if ("first".equals(arguments.getString("pairPhase"))) {
                    JSONObject paired = (JSONObject) engine.getMethod("pair", Context.class, String.class, String.class, String.class)
                        .invoke(null, context, arguments.getString("pairAddress"), arguments.getString("pairCode"), "Test Mac");
                    require(expected.equals(paired.getString("peer")), "Paired wrong identity");
                }
                JSONArray devices = (JSONArray) engine.getMethod("devices", Context.class).invoke(null, context);
                boolean found = false;
                for (int i = 0; i < devices.length(); i++) {
                    JSONObject device = devices.getJSONObject(i);
                    if (expected.equals(device.getString("peer"))) {
                        found = true;
                        require("Test Mac".equals(device.getString("name")), "Peer name did not persist");
                        if (!arguments.containsKey("discoveryName")) require(arguments.getString("pairAddress").equals(device.getString("address")), "Peer address did not persist");
                        require(!device.getString("address").isEmpty(), "Peer address unavailable after restart");
                    }
                }
                require(found, "Paired peer missing after initialization");
                JSONObject checked = (JSONObject) engine.getMethod("check", Context.class, String.class).invoke(null, context, expected);
                require(expected.equals(checked.getString("peer")), "Probe identity mismatch");
                require(checked.getDouble("rtt_ms") >= 0, "Invalid authenticated probe timing");
                if ("second".equals(arguments.getString("pairPhase"))) {
                    engine.getMethod("forget", Context.class, String.class).invoke(null, context, expected);
                    JSONArray remaining = new JSONArray((String) engine.getMethod("pairedPeers").invoke(null));
                    for (int i = 0; i < remaining.length(); i++) require(!expected.equals(remaining.getString(i)), "Forgotten peer still trusted");
                    try {
                        engine.getMethod("check", Context.class, String.class).invoke(null, context, expected);
                        throw new AssertionError("Forgotten peer reconnected");
                    } catch (InvocationTargetException rejected) { require(rejected.getCause() instanceof IllegalStateException, "Unexpected forgotten peer failure"); }
                }
                result.putString("pairing", "passed-" + arguments.getString("pairPhase"));
            }
            result.putString("status", "passed");
            result.putString("fingerprint", fingerprint);
            result.putString("checks", "native-load,fingerprint,non-exportable-wrapper,tamper-rejection,no-silent-replacement,account-proof,invalid-server-key,trust-store");
            finish(-1, result);
        } catch (Throwable error) {
            result.putString("status", "failed");
            result.putString("error", error.toString());
            finish(0, result);
        }
    }
}
