package computer.extend.android;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import org.json.JSONObject;

/** TLS uses platform trust; bearer credentials never follow a redirect. */
final class AccountHttp {
    private static final java.util.concurrent.ScheduledExecutorService DEADLINES=java.util.concurrent.Executors.newSingleThreadScheduledExecutor(task->{Thread thread=new Thread(task,"extend-account-http-deadline");thread.setDaemon(true);return thread;});
    static final class Failure extends Exception {
        final int status;
        Failure(int status) { super(status == 401 ? "Your login or code expired. Sign in again." : status == 429 ? "Too many requests. Wait and try again." : "Account request failed. Try again."); this.status = status; }
    }
    JSONObject request(String origin, String path, String method, JSONObject body, String token) throws Exception {
        if (!origin.equals(AccountProtocol.origin(origin)) || !path.startsWith("/") || path.contains("..") || (token != null && !AccountProtocol.hex(token)))
            throw new IllegalArgumentException("Invalid account request.");
        HttpURLConnection connection = (HttpURLConnection) new URL(origin + "/v1" + path).openConnection();
        connection.setConnectTimeout(12000); connection.setReadTimeout(12000);
        connection.setInstanceFollowRedirects(false); connection.setUseCaches(false);
        connection.setRequestMethod(method); connection.setRequestProperty("Origin",origin);
        connection.setRequestProperty("X-Extend-Client","desktop"); connection.setRequestProperty("Accept","application/json");
        if (token != null) connection.setRequestProperty("Authorization","Bearer " + token);
        java.util.concurrent.ScheduledFuture<?> timeout=DEADLINES.schedule(connection::disconnect,12,java.util.concurrent.TimeUnit.SECONDS);
        try {
            if (body != null) {
                byte[] bytes = body.toString().getBytes(StandardCharsets.UTF_8);
                connection.setRequestProperty("Content-Type","application/json"); connection.setDoOutput(true); connection.setFixedLengthStreamingMode(bytes.length);
                try (java.io.OutputStream output = connection.getOutputStream()) { output.write(bytes); }
            }
            int status = connection.getResponseCode();
            if (status < 200 || status >= 300) throw new Failure(status);
            ByteArrayOutputStream output = new ByteArrayOutputStream();
            long deadline = System.nanoTime() + 12000000000L;
            try (InputStream input = connection.getInputStream()) {
                byte[] block = new byte[4096]; int count;
                while ((count = input.read(block)) != -1) {
                    if (output.size() + count > 256 * 1024 || System.nanoTime() > deadline) throw new IllegalStateException("Account response exceeded its limit.");
                    output.write(block,0,count);
                }
            }
            return output.size() == 0 ? new JSONObject() : new JSONObject(output.toString("UTF-8"));
        } catch (Failure expected) { throw expected; }
        catch (Exception failed) { throw new IllegalStateException("Couldn't reach the account server. Check your connection and try again."); }
        finally { timeout.cancel(false);connection.disconnect(); }
    }
}
