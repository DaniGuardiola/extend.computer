package computer.extend.android;

import java.net.URI;
import java.net.URLDecoder;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.SecureRandom;
import java.util.Base64;
import java.util.HashMap;
import java.util.Locale;
import java.util.Map;
import org.json.JSONArray;
import org.json.JSONObject;

/** Pure account wire validation. Discovery and heartbeat are never peer authentication. */
final class AccountProtocol {
    static String origin(String raw) throws Exception {
        URI uri = new URI(raw.trim());
        String host = uri.getHost(), scheme = uri.getScheme();
        if (host == null || scheme == null || uri.getRawUserInfo() != null || uri.getRawQuery() != null || uri.getRawFragment() != null
            || !(uri.getRawPath().isEmpty() || uri.getRawPath().equals("/")) || uri.getPort() == 0 || uri.getPort() > 65535)
            throw new IllegalArgumentException("Use the account server origin without a path, query or credentials.");
        scheme = scheme.toLowerCase(Locale.ROOT); host = host.toLowerCase(Locale.ROOT);
        if (!scheme.equals("https") && !(scheme.equals("http") && (host.equals("localhost") || host.equals("127.0.0.1") || host.equals("[::1]") || host.equals("::1"))))
            throw new IllegalArgumentException("Account servers require HTTPS. Localhost HTTP is allowed for development.");
        int port = uri.getPort();
        if (port == (scheme.equals("https") ? 443 : 80)) port = -1;
        return new URI(scheme, null, host, port, null, null, null).toASCIIString();
    }
    static boolean hex(String value) { return value != null && value.matches("[a-f0-9]{64}"); }
    static String text(JSONObject value, String key, int max) throws Exception {
        Object field = value.opt(key);
        if (!(field instanceof String)) throw new IllegalArgumentException("Invalid account response.");
        String text = (String) field;
        if (text.isEmpty() || text.getBytes(StandardCharsets.UTF_8).length > max || text.codePoints().anyMatch(Character::isISOControl))
            throw new IllegalArgumentException("Invalid account response.");
        return text;
    }
    static String id(JSONObject value, String key) throws Exception {
        String id = text(value, key, 64);
        if (!hex(id)) throw new IllegalArgumentException("Invalid account device identifier.");
        return id;
    }
    static String randomHex() { byte[] bytes = new byte[32]; new SecureRandom().nextBytes(bytes); return hexBytes(bytes); }
    static String hexBytes(byte[] bytes) { StringBuilder result = new StringBuilder(); for (byte b : bytes) result.append(String.format(Locale.ROOT, "%02x", b & 255)); return result.toString(); }
    static String verifier() { byte[] bytes = new byte[32]; new SecureRandom().nextBytes(bytes); return Base64.getUrlEncoder().withoutPadding().encodeToString(bytes); }
    static String challenge(String verifier) throws Exception { return Base64.getUrlEncoder().withoutPadding().encodeToString(MessageDigest.getInstance("SHA-256").digest(verifier.getBytes(StandardCharsets.US_ASCII))); }
    static String callback(String requestLine, String state) {
        try {
            if (!requestLine.startsWith("GET ") || !requestLine.endsWith(" HTTP/1.1")) return null;
            String target = requestLine.substring(4, requestLine.length() - 9);
            URI uri = new URI(target);
            if (uri.isAbsolute() || uri.getRawAuthority() != null || !uri.getRawPath().equals("/callback") || uri.getRawFragment() != null || uri.getRawQuery() == null) return null;
            Map<String,String> query = new HashMap<>();
            for (String pair : uri.getRawQuery().split("&", -1)) {
                String[] field = pair.split("=", -1);
                if (field.length != 2) return null;
                String key = URLDecoder.decode(field[0], "UTF-8"), value = URLDecoder.decode(field[1], "UTF-8");
                if (query.put(key, value) != null) return null;
            }
            String receivedState = query.get("state"), code = query.get("code");
            if (query.size() != 2 || !hex(receivedState) || !hex(code) || !MessageDigest.isEqual(state.getBytes(StandardCharsets.US_ASCII), receivedState.getBytes(StandardCharsets.US_ASCII))) return null;
            return code;
        } catch (Exception malformed) { return null; }
    }
    static JSONArray directory(JSONObject response) throws Exception {
        JSONArray rows = response.getJSONArray("devices"), clean = new JSONArray();
        if (rows.length() > 100) throw new IllegalArgumentException("Account device list is too large.");
        java.util.Set<String> ids = new java.util.HashSet<>(), fingerprints = new java.util.HashSet<>();
        for (int i = 0; i < rows.length(); i++) {
            JSONObject row = rows.getJSONObject(i);
            String id = id(row, "id"), fingerprint = id(row, "fingerprint");
            if (!ids.add(id) || !fingerprints.add(fingerprint)) throw new IllegalArgumentException("Duplicate account device.");
            Object verified = row.opt("key_verified");
            boolean validKey = Boolean.TRUE.equals(verified) || Integer.valueOf(1).equals(verified) || Long.valueOf(1).equals(verified);
            clean.put(new JSONObject().put("id", id).put("fingerprint", fingerprint).put("name", text(row,"name",100))
                .put("platform", text(row,"platform",32)).put("key_verified", validKey).put("online", row.optBoolean("online",false)));
        }
        return clean;
    }
}
