package computer.extend.android;

import android.media.MediaCodec;
import android.media.MediaCodecInfo;
import android.media.MediaCodecList;
import android.media.MediaFormat;
import android.os.Build;
import android.os.Handler;
import android.os.HandlerThread;
import android.view.Surface;
import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.net.Socket;
import java.nio.ByteBuffer;
import org.json.JSONObject;

final class DisplaySession {
    interface Listener {
        void configured(int width, int height);
        default void configured(int width, int height, int logicalWidth, int logicalHeight) {
            configured(width, height);
        }
        default void configured(int width, int height, int logicalWidth, int logicalHeight, long revision) {
            configured(width, height, logicalWidth, logicalHeight);
        }
        void rendered();
        default void rendered(long revision) { rendered(); }
    }
    private static final int MAX_PACKET = 8 * 1024 * 1024;

    static void send(DataOutputStream output, JSONObject value) throws Exception {
        byte[] bytes = value.toString().getBytes("UTF-8");
        synchronized (output) {
            output.writeByte(2); output.writeInt(bytes.length); output.write(bytes); output.flush();
        }
    }

    static MediaCodecInfo decoder(String mime, int width, int height, int fps) throws IOException {
        for (MediaCodecInfo info : new MediaCodecList(MediaCodecList.REGULAR_CODECS).getCodecInfos()) {
            if (info.isEncoder() || !info.isHardwareAccelerated()) continue;
            for (String type : info.getSupportedTypes()) {
                if (!type.equalsIgnoreCase(mime)) continue;
                MediaCodecInfo.VideoCapabilities caps = info.getCapabilitiesForType(type).getVideoCapabilities();
                if (caps != null && caps.areSizeAndRateSupported(width, height, fps)) return info;
            }
        }
        throw new IOException("No hardware decoder supports this codec, display size and frame rate.");
    }

    static void receive(Socket socket, String token, Surface surface, Listener listener) throws Exception {
        DataInputStream input = new DataInputStream(socket.getInputStream());
        DataOutputStream output = new DataOutputStream(socket.getOutputStream());
        send(output, new JSONObject().put("type", "hello").put("version", 1).put("token", token));
        MediaCodec codec = null;
        HandlerThread events = new HandlerThread("extend-display-render-events");
        events.start();
        int width = 0, height = 0, fps = 0;
        String mime = "video/avc";
        long received = 0, submitted = 0;
        long lastTimestamp = -1;
        final java.util.concurrent.atomic.AtomicLong rendered = new java.util.concurrent.atomic.AtomicLong();
        final java.util.concurrent.atomic.AtomicBoolean draining = new java.util.concurrent.atomic.AtomicBoolean();
        Thread outputWorker = null;
        try {
            while (!socket.isClosed()) {
                int kind = input.readUnsignedByte();
                int length = input.readInt();
                if (length < 0 || length > MAX_PACKET) throw new IOException("Invalid display packet size.");
                byte[] payload = new byte[length];
                input.readFully(payload);
                if (kind == 0) {
                    if (width != 0 || length > 4096) throw new IOException("Unexpected display configuration.");
                    JSONObject config = new JSONObject(new String(payload, "UTF-8"));
                    width = config.getInt("width"); height = config.getInt("height"); fps = config.getInt("fps");
                    String selectedCodec = config.getString("codec");
                    if (!(selectedCodec.equals("h264") || selectedCodec.equals("hevc")) || width < 640 || height < 480
                            || width > 4096 || height > 4096 || fps < 30 || fps > 120
                            || width % 2 != 0 || height % 2 != 0) throw new IOException("Unsupported display configuration.");
                    mime = selectedCodec.equals("hevc") ? "video/hevc" : "video/avc";
                    MediaCodecInfo selected = decoder(mime, width, height, fps);
                    send(output, new JSONObject().put("type", "ready").put("decoder", selected.getName()));
                    listener.configured(width, height);
                } else if (kind == 1) {
                    if (width == 0 || length <= 9) throw new IOException("Video arrived before configuration.");
                    ByteBuffer frame = ByteBuffer.wrap(payload);
                    long timestamp = frame.getLong();
                    int keyframe = frame.get() & 255;
                    if (timestamp < 0 || timestamp <= lastTimestamp || keyframe > 1)
                        throw new IOException("Invalid frame metadata.");
                    lastTimestamp = timestamp;
                    received++;
                    if (codec == null) {
                        if (keyframe == 0) continue;
                        MediaCodecInfo selected = decoder(mime, width, height, fps);
                        MediaFormat format = MediaFormat.createVideoFormat(mime, width, height);
                        format.setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, MAX_PACKET);
                        boolean hevc = mime.equals("video/hevc");
                        byte[][] parameters = AnnexB.parameters(payload, 9, hevc);
                        if (hevc) {
                            java.io.ByteArrayOutputStream joined = new java.io.ByteArrayOutputStream();
                            for (byte[] parameter : parameters) joined.write(parameter);
                            format.setByteBuffer("csd-0", ByteBuffer.wrap(joined.toByteArray()));
                        } else {
                            format.setByteBuffer("csd-0", ByteBuffer.wrap(parameters[0]));
                            format.setByteBuffer("csd-1", ByteBuffer.wrap(parameters[1]));
                        }
                        boolean lowLatency = Build.VERSION.SDK_INT >= 30
                            && selected.getCapabilitiesForType(mime).isFeatureSupported("low-latency");
                        if (lowLatency) format.setInteger("low-latency", 1);
                        codec = MediaCodec.createByCodecName(selected.getName());
                        codec.configure(format, surface, null, 0);
                        codec.setOnFrameRenderedListener((active, presentationTimeUs, nanoTime) -> {
                            long count = rendered.incrementAndGet();
                            if (count == 1) listener.rendered();
                            if (count == 1 || count % 60 == 0) try {
                                send(output, new JSONObject().put("type", "rendered").put("frames", count)
                                    .put("timestamp_us", presentationTimeUs).put("device_ns", nanoTime));
                            } catch (Exception error) { try { socket.close(); } catch (IOException ignored) {} }
                        }, new Handler(events.getLooper()));
                        codec.start();
                        final MediaCodec active = codec;
                        draining.set(true);
                        outputWorker = new Thread(() -> {
                            MediaCodec.BufferInfo info = new MediaCodec.BufferInfo();
                            try {
                                while (draining.get()) {
                                    drain(active, info);
                                    Thread.sleep(1);
                                }
                            } catch (Exception error) {
                                if (draining.get()) {
                                    android.util.Log.e("ExtendDisplay", "Decoder output failed", error);
                                    try { socket.close(); } catch (IOException ignored) {}
                                }
                            }
                        }, "extend-display-output");
                        outputWorker.start();
                        send(output, new JSONObject().put("type", "decoder").put("name", selected.getName())
                            .put("hardware", true).put("low_latency", lowLatency));
                    }
                    int slot;
                    long deadline = System.nanoTime() + 1_000_000_000L;
                    while ((slot = codec.dequeueInputBuffer(2000)) < 0) {
                        if (System.nanoTime() > deadline) throw new IOException("Hardware decoder stalled.");
                    }
                    ByteBuffer buffer = codec.getInputBuffer(slot);
                    if (buffer == null || buffer.capacity() < length - 9) throw new IOException("Encoded frame exceeds decoder buffer.");
                    buffer.clear(); buffer.put(payload, 9, length - 9);
                    codec.queueInputBuffer(slot, 0, length - 9, timestamp / 1000, 0);
                    submitted++;
                } else if (kind == 2) {
                    if (length > 4096) throw new IOException("Oversized control message.");
                    JSONObject control = new JSONObject(new String(payload, "UTF-8"));
                    if (control.optString("type").equals("ping")) {
                        send(output, new JSONObject().put("type", "stats").put("received", received)
                            .put("submitted", submitted).put("rendered", rendered.get()));
                    } else if (control.optString("type").equals("close")) return;
                    else throw new IOException("Unknown display control message.");
                } else throw new IOException("Unknown display packet type.");
            }
        } finally {
            draining.set(false);
            if (outputWorker != null) outputWorker.join(2000);
            if (codec != null) codec.release();
            events.quitSafely(); events.join(2000);
        }
    }

    // Discard stale decoded output only; never discard encoded reference frames.
    static void drain(MediaCodec codec, MediaCodec.BufferInfo info) {
        int latest = -1;
        while (true) {
            int slot = codec.dequeueOutputBuffer(info, 0);
            if (slot >= 0) {
                if (latest >= 0) codec.releaseOutputBuffer(latest, false);
                latest = slot;
            } else if (slot == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                android.util.Log.i("ExtendDisplay", "Decoder output: " + codec.getOutputFormat());
            } else break;
        }
        // Source PTS belongs to another clock. Submit immediately using the
        // receiver clock instead of forwarding media time to SurfaceView.
        if (latest >= 0) codec.releaseOutputBuffer(latest, System.nanoTime());
    }

    // Trusted shared-engine sessions certify the output buffer's visible raster
    // before presentation. This protects a resize from an in-flight older IDR;
    // equal-raster metadata changes still need a future media revision boundary.
    static void drain(MediaCodec codec, MediaCodec.BufferInfo info, DecodedFrameGate gate) throws IOException {
        int latest = -1;
        long latestPts = -1;
        try {
            while (true) {
                int slot = codec.dequeueOutputBuffer(info, 0);
                if (slot >= 0) {
                    boolean accepted;
                    try { accepted = actualRaster(codec.getOutputFormat(slot), gate, System.nanoTime()); }
                    catch (IOException | RuntimeException failed) { codec.releaseOutputBuffer(slot, false); throw failed; }
                    if (!accepted) { codec.releaseOutputBuffer(slot, false); continue; }
                    if (latest >= 0) codec.releaseOutputBuffer(latest, false);
                    latest = slot; latestPts = info.presentationTimeUs;
                } else if (slot == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                    android.util.Log.i("ExtendDisplay", "Decoder output: " + codec.getOutputFormat());
                } else break;
            }
            gate.checkDeadline(System.nanoTime());
            if (latest >= 0 && gate.permit(latestPts)) {
                int rendered = latest; latest = -1;
                try { codec.releaseOutputBuffer(rendered, System.nanoTime()); }
                catch (RuntimeException failed) { gate.withdraw(latestPts); throw failed; }
            }
        } finally {
            if (latest >= 0) codec.releaseOutputBuffer(latest, false);
        }
    }
    private static boolean actualRaster(MediaFormat format, DecodedFrameGate gate, long now) throws IOException {
        if (format == null || !format.containsKey(MediaFormat.KEY_WIDTH) || !format.containsKey(MediaFormat.KEY_HEIGHT))
            throw new IOException("Missing decoder output geometry");
        try {
            return gate.accepts(format.getInteger(MediaFormat.KEY_WIDTH), format.getInteger(MediaFormat.KEY_HEIGHT),
                format.containsKey("crop-left") ? format.getInteger("crop-left") : null,
                format.containsKey("crop-right") ? format.getInteger("crop-right") : null,
                format.containsKey("crop-top") ? format.getInteger("crop-top") : null,
                format.containsKey("crop-bottom") ? format.getInteger("crop-bottom") : null, now);
        } catch (RuntimeException malformed) { throw new IOException("Malformed decoder output geometry", malformed); }
    }
}
