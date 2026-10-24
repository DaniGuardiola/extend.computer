package computer.extend.android;

import java.net.URI;
import java.net.URISyntaxException;
import java.util.Locale;
import org.json.JSONObject;

/** Numeric route hints only. Saving an address never authenticates or pairs a device. */
final class PeerAddress {
    private PeerAddress() {}
    private static IllegalArgumentException invalid() {
        return new IllegalArgumentException("Enter a numeric IPv4:port or [IPv6]:port address.");
    }
    static String normalize(String raw) {
        if (raw == null || raw.length() > 128 || raw.codePoints().anyMatch(Character::isISOControl)) throw invalid();
        String address = raw.trim();
        String host, port;
        if (address.startsWith("[")) {
            int end = address.indexOf(']');
            if (end < 2 || end + 1 >= address.length() || address.charAt(end + 1) != ':') throw invalid();
            host = address.substring(1, end);
            port = address.substring(end + 2);
            ipv6(host);
            host = "[" + host.toLowerCase(Locale.ROOT) + "]";
        } else {
            int colon = address.indexOf(':');
            if (colon < 1 || colon != address.lastIndexOf(':')) throw invalid();
            host = address.substring(0, colon);
            port = address.substring(colon + 1);
            int[] bytes = ipv4(host);
            if ((bytes[0] | bytes[1] | bytes[2] | bytes[3]) == 0 || bytes[0] >= 224 && bytes[0] <= 239) throw invalid();
        }
        if (!port.matches("[0-9]{1,5}")) throw invalid();
        int number = Integer.parseInt(port);
        if (number == 0 || number > 65535) throw invalid();
        return host + ":" + number;
    }
    private static int[] ipv4(String host) {
        String[] parts = host.split("\\.", -1);
        if (parts.length != 4) throw invalid();
        int[] bytes = new int[4];
        for (int i = 0; i < 4; i++) {
            if (!parts[i].matches("0|[1-9][0-9]{0,2}")) throw invalid();
            bytes[i] = Integer.parseInt(parts[i]);
            if (bytes[i] > 255) throw invalid();
        }
        return bytes;
    }
    private static void ipv6(String host) {
        if (!host.contains(":") || !host.matches("[0-9a-fA-F:.]+(?:%[0-9]{1,10})?")) throw invalid();
        int scope = host.indexOf('%');
        String literal = scope < 0 ? host : host.substring(0, scope);
        if (scope >= 0 && Long.parseLong(host.substring(scope + 1)) > 0xffffffffL) throw invalid();
        // URI validates IPv6 syntax locally; no hostname resolver is called.
        try {
            if (new URI("tcp://[" + host + "]:1").getHost() == null) throw invalid();
        } catch (URISyntaxException malformed) { throw invalid(); }
        if (literal.contains(".")) ipv4(literal.substring(literal.lastIndexOf(':') + 1));
        if (literal.replace(":", "").replace(".", "").matches("0*")) throw invalid();
        int firstColon = literal.indexOf(':');
        int first = firstColon == 0 ? 0 : Integer.parseInt(literal.substring(0, firstColon), 16);
        if (first >= 0xff00) throw invalid();
    }
    interface Store {
        boolean paired(String peer) throws Exception;
        String read(String peer) throws Exception;
        boolean commit(String peer, String metadata) throws Exception;
    }
    interface Probe { String read(String address, String peer) throws Exception; }
    static JSONObject metadata(String saved, String fallbackName) throws Exception {
        JSONObject metadata = saved == null ? new JSONObject() : new JSONObject(saved);
        if (!metadata.has("name")) metadata.put("name", fallbackName);
        if (!metadata.has("address")) metadata.put("address", "");
        return metadata;
    }
    static JSONObject verifyAndSave(String peer, String address, Store store, Probe probe) throws Exception {
        synchronized (NativeEngine.class) {
            String endpoint = normalize(address);
            JSONObject result = new JSONObject(probe.read(endpoint, peer));
            // Neither an unverified discovery hint nor a presence reply alone
            // can replace the route. The exact pinned control probe comes first.
            save(peer, endpoint, store);
            return result;
        }
    }
    static void save(String peer, String raw, Store store) throws Exception {
        if (!AccountProtocol.hex(peer)) throw new IllegalArgumentException("Invalid device identity.");
        String address = normalize(raw);
        // Use the same monitor as Check connection and Unpair. A stale Save must
        // not recreate metadata after a completed local unpair.
        synchronized (NativeEngine.class) {
            if (!store.paired(peer)) throw new IllegalStateException("This device is no longer paired.");
            String saved = store.read(peer);
            JSONObject metadata = saved == null ? new JSONObject() : new JSONObject(saved);
            metadata.put("address", address);
            if (!store.paired(peer)) throw new IllegalStateException("This device is no longer paired.");
            if (!store.commit(peer, metadata.toString())) throw new IllegalStateException("Couldn't save device address.");
        }
    }
}
