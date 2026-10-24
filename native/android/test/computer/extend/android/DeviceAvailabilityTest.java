package computer.extend.android;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

public final class DeviceAvailabilityTest {
    private static void equal(Object expected, Object actual) {
        if (!expected.equals(actual)) throw new AssertionError(expected + " != " + actual);
    }
    public static void main(String[] args) throws Exception {
        String usbPeer="a".repeat(64);
        org.json.JSONObject attached=new org.json.JSONObject().put("state","connected").put("peer",usbPeer).put("id","12").put("streams_ready",false);
        equal(true, DeviceAvailability.usbSelected(attached,usbPeer));
        equal(false, DeviceAvailability.usbSelected(attached,"b".repeat(64)));
        equal(false, DeviceAvailability.usbSelected(new org.json.JSONObject(attached.toString()).put("state","disconnected"),usbPeer));
        for(String bad:Arrays.asList("", "usb:12", "-1", "12x", "1".repeat(21))) {
            equal(false,DeviceAvailability.usbSelected(new org.json.JSONObject(attached.toString()).put("id",bad),usbPeer));
        }
        equal(Arrays.asList("192.168.1.22:48178", "192.168.1.10:48178"),
            DeviceAvailability.endpoints("192.168.1.22:54321", "192.168.1.10"));
        equal(Arrays.asList("192.168.1.22:48178"),
            DeviceAvailability.endpoints("192.168.1.22:48177", "192.168.1.22"));
        equal(Arrays.asList("[2001:db8::2]:48178", "[2001:db8::1]:48178"),
            DeviceAvailability.endpoints("[2001:DB8::2]:48177", "2001:db8::1"));
        equal(Arrays.asList("[2001:db8::2]:48178"),
            DeviceAvailability.endpoints("[2001:db8::2]:48177", "[2001:DB8::2]"));
        equal(Arrays.asList("192.168.1.22:48178"),
            DeviceAvailability.endpoints("", "192.168.1.22"));
        for (String malformed : Arrays.asList("not-an-endpoint", "[::1]", "[]:48177", "::1:48177", "192.168.1.1:0", "192.168.1.1:65536", "192.168.1.1:bad")) {
            equal(Arrays.asList("192.168.1.22:48178"),
                DeviceAvailability.endpoints(malformed, "192.168.1.22"));
        }
        equal(Arrays.asList(), DeviceAvailability.endpoints("", ""));

        List<String> calls = new ArrayList<>();
        List<String> hints = DeviceAvailability.endpoints("192.168.1.22:48177", "192.168.1.10");
        equal("pinned-current", DeviceAvailability.query(hints, endpoint -> {
            calls.add(endpoint);
            if (!endpoint.equals("192.168.1.22:48178")) throw new AssertionError("Old host outranked repaired route");
            return "pinned-current";
        }));
        equal(Arrays.asList("192.168.1.22:48178"), calls);
        calls.clear();
        equal("pinned-fallback", DeviceAvailability.query(hints, endpoint -> {
            calls.add(endpoint);
            if (endpoint.equals("192.168.1.22:48178")) throw new Exception("Could not reach this device's availability service");
            return "pinned-fallback";
        }));
        equal(hints, calls);
        try {
            DeviceAvailability.query(hints, endpoint -> {
                throw new Exception(endpoint.equals(hints.get(0)) ? "Pinned identity mismatch" : "Could not reach this device's availability service");
            });
            throw new AssertionError("Failed verification reported success");
        } catch (Exception expected) {
            equal("Pinned identity mismatch", expected.getMessage());
        }
        System.out.println("PASS: repaired route first, IPv4/IPv6, dedup, malformed hints, bounded pinned fallback, verification failure preservation");
    }
}
