package computer.extend.android;

import org.json.JSONObject;

/** Local display names must not rewrite authenticated names, pinned routes, or device identities. */
public final class DeviceSettingsTest {
    static void check(boolean value, String message) { if (!value) throw new AssertionError(message); }
    static void equal(Object expected, Object actual) { check(expected.equals(actual), expected + " != " + actual); }
    static void rejected(String value) {
        try { DeviceSettings.normalize(value); } catch (IllegalArgumentException expected) { return; }
        throw new AssertionError("Invalid device name accepted");
    }
    public static void main(String[] args) throws Exception {
        equal("", DeviceSettings.normalize("  "));
        equal("Tablet", DeviceSettings.normalize("  Tablet  "));
        equal("🖥".repeat(25), DeviceSettings.normalize("🖥".repeat(25)));
        rejected("🖥".repeat(26));
        rejected("x".repeat(101));
        rejected("Tablet\nname");
        rejected("Tablet\u0085name");
        String peer = "1".repeat(64);
        JSONObject original = new JSONObject().put("peer", peer).put("name", "MacBook")
            .put("address", "[fe80::abcd%wlan0]:48177").put("trustSource", "account")
            .put("public_key", "2".repeat(64));
        String saved = original.toString();
        JSONObject labeled = DeviceSettings.decorate(original, "MacBook Pro", "Work computer");
        equal("Work computer", labeled.getString("name"));
        equal("MacBook", labeled.getString("defaultName"));
        equal("MacBook Pro", labeled.getString("authenticatedName"));
        equal("Work computer", labeled.getString("label"));
        for (String key : new String[] { "peer", "address", "trustSource", "public_key" })
            equal(original.get(key), labeled.get(key));
        equal(saved, original.toString());
        JSONObject reset = DeviceSettings.decorate(original, "MacBook Pro", null);
        equal("MacBook", reset.getString("name"));
        equal(peer, reset.getString("peer"));
        check(reset.isNull("label"), "Reset retained local label");
        equal("MacBook Pro", reset.getString("authenticatedName"));
        check(DeviceSettings.decorate(original, "", null).isNull("authenticatedName"), "Invented an authenticated name");
        System.out.println("Device settings contracts passed: name bounds, reset, immutable identity, route and authenticated name.");
    }
}
