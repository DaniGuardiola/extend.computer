package computer.extend.android;

import android.content.Context;
import android.media.MediaCodec;
import android.media.MediaCodecInfo;
import android.media.MediaFormat;
import android.os.Build;
import android.os.Handler;
import android.os.HandlerThread;
import android.view.Surface;
import java.nio.ByteBuffer;
import java.io.IOException;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;
import org.json.JSONObject;
import org.json.JSONArray;

/** Native shared-engine receiver. React receives status only, never media bytes. */
public final class MediaSession {
    private static final int MAX_PACKET = 8 * 1024 * 1024;
    public static JSONObject capabilities(Context context) throws Exception {
        android.util.DisplayMetrics metrics = context.getResources().getDisplayMetrics();
        int pixelsWidth = metrics.widthPixels, pixelsHeight = metrics.heightPixels;
        android.view.WindowManager windows = (android.view.WindowManager) context.getSystemService(Context.WINDOW_SERVICE);
        if (windows != null) {
            if (Build.VERSION.SDK_INT >= 30) {
                android.graphics.Rect bounds = windows.getMaximumWindowMetrics().getBounds();
                pixelsWidth = bounds.width(); pixelsHeight = bounds.height();
            } else {
                android.util.DisplayMetrics full = new android.util.DisplayMetrics();
                windows.getDefaultDisplay().getRealMetrics(full);
                pixelsWidth = full.widthPixels; pixelsHeight = full.heightPixels;
            }
        }
        int screenWidth = Math.max(pixelsWidth, pixelsHeight);
        int screenHeight = Math.min(pixelsWidth, pixelsHeight);
        float density = metrics.density;
        if (!Float.isFinite(density) || density <= 0 || screenWidth <= 0 || screenHeight <= 0)
            throw new IOException("Invalid fullscreen display metrics");
        // Android density is the user's logical UI scale. fontScale applies to
        // sp text, not the desktop's logical display geometry.
        double logicalWidth = screenWidth / (double) density;
        double logicalHeight = screenHeight / (double) density;
        double minimum = Math.max(1., Math.max(640. / logicalWidth, 480. / logicalHeight));
        int preferredWidth = (int)Math.round(logicalWidth * minimum) / 2 * 2;
        int preferredHeight = (int)Math.round(logicalHeight * minimum) / 2 * 2;
        JSONObject result = new JSONObject();
        result.put("preferred_logical", new JSONObject().put("width",preferredWidth).put("height",preferredHeight)
            .put("density",density).put("physical_width",screenWidth).put("physical_height",screenHeight));
        for (String codec : new String[]{"h264","hevc"}) {
            JSONArray sizes = new JSONArray();
            String mime = codec.equals("hevc") ? "video/hevc" : "video/avc";
            for (int fps : new int[]{120,90,60,30}) {
                for (int width : new int[]{preferredWidth,preferredWidth*2,screenWidth,1920,1600,1280,960,800,640}) {
                    if (width > screenWidth || width > 4096) continue;
                    int height = width == preferredWidth ? preferredHeight : width == preferredWidth*2 ? preferredHeight*2
                        : ((int)Math.round((double)width * screenHeight / screenWidth) / 2) * 2;
                    if (height < 480 || height > 4096) continue;
                    try { DisplaySession.decoder(mime,width,height,fps);
                        sizes.put(new JSONObject().put("width",width).put("height",height).put("fps",fps));
                    } catch (IOException unsupported) {}
                }
            }
            result.put(codec,sizes);
        }
        return result;
    }
    public static void receive(String id, JSONObject config, Surface surface, DisplaySession.Listener listener) throws Exception {
        int scale = config.getBoolean("retina") ? 2 : 1;
        int width = config.getInt("width") * scale, height = config.getInt("height") * scale;
        int fps = config.getInt("fps");
        String mime = config.getString("codec").equals("hevc") ? "video/hevc" : "video/avc";
        MediaCodecInfo selected = DisplaySession.decoder(mime,width,height,fps);
        listener.configured(width,height,config.getInt("width"),config.getInt("height"),0);
        HandlerThread events = new HandlerThread("extend-media-render-events");events.start();
        AtomicBoolean draining = new AtomicBoolean();
        AtomicReference<Exception> outputError = new AtomicReference<>();
        Thread outputWorker = null;
        MediaCodec codec = null;
        DecodedFrameGate outputGate = null;
        long previous = -1;
        long revision = 0;
        java.util.concurrent.atomic.AtomicLong currentRevision = new java.util.concurrent.atomic.AtomicLong();
        try {
            NativeEngine.attachDisplay(id);
            while (true) {
                Exception failed = outputError.get();if (failed != null) throw failed;
                JSONObject state = new JSONObject(NativeEngine.displayState());
                if (!id.equals(state.optString("id"))) throw new IOException("Display session replaced");
                long nextRevision = state.optLong("revision");
                if (nextRevision != revision) {
                    currentRevision.set(nextRevision);
                    draining.set(false);
                    if (outputGate != null) outputGate.close();
                    if (outputWorker != null) {
                        outputWorker.join(2000);
                        if (outputWorker.isAlive()) throw new IOException("Hardware decoder output stalled");
                        outputWorker = null;
                    }
                    if (codec != null) { codec.release(); codec = null; }
                    outputError.set(null);
                    JSONObject next = state.getJSONObject("config");
                    String nextMime = next.getString("codec").equals("hevc") ? "video/hevc" : "video/avc";
                    if (!mime.equals(nextMime)) throw new IOException("Live codec changes require reconnect");
                    scale = next.getBoolean("retina") ? 2 : 1;
                    width = next.getInt("width") * scale; height = next.getInt("height") * scale;
                    fps = next.getInt("fps");
                    selected = DisplaySession.decoder(mime,width,height,fps);
                    listener.configured(width,height,next.getInt("width"),next.getInt("height"),nextRevision);
                    revision = nextRevision;
                    NativeEngine.displayConfigured(id,revision);
                }
                byte[] payload = NativeEngine.nextDisplayFrame(id);
                if (payload == null) continue;
                if (payload.length <= 17 || payload.length > MAX_PACKET + 17) throw new IOException("Invalid video frame");
                ByteBuffer metadata = ByteBuffer.wrap(payload);
                long timestamp = metadata.getLong();int keyframe = metadata.get() & 255;
                long frameRevision = metadata.getLong();
                if (frameRevision != revision) continue;
                if (timestamp < 0 || timestamp <= previous || keyframe > 1) throw new IOException("Invalid video timestamp");
                previous = timestamp;
                if (codec == null) {
                    if (keyframe == 0) continue;
                    MediaFormat format = MediaFormat.createVideoFormat(mime,width,height);
                    format.setInteger(MediaFormat.KEY_MAX_INPUT_SIZE,MAX_PACKET);
                    byte[][] parameters = AnnexB.parameters(payload,17,mime.equals("video/hevc"));
                    if (mime.equals("video/hevc")) {
                        java.io.ByteArrayOutputStream joined = new java.io.ByteArrayOutputStream();
                        for (byte[] parameter : parameters) joined.write(parameter);
                        format.setByteBuffer("csd-0",ByteBuffer.wrap(joined.toByteArray()));
                    } else {
                        format.setByteBuffer("csd-0",ByteBuffer.wrap(parameters[0]));
                        format.setByteBuffer("csd-1",ByteBuffer.wrap(parameters[1]));
                    }
                    boolean lowLatency = Build.VERSION.SDK_INT >= 30 && selected.getCapabilitiesForType(mime).isFeatureSupported("low-latency");
                    if (lowLatency) format.setInteger("low-latency",1);
                    codec = MediaCodec.createByCodecName(selected.getName());
                    codec.configure(format,surface,null,0);
                    final DecodedFrameGate decoderGate = new DecodedFrameGate(width,height);
                    outputGate = decoderGate;
                    AtomicBoolean first = new AtomicBoolean(true);
                    java.util.concurrent.atomic.AtomicLong rendered = new java.util.concurrent.atomic.AtomicLong();
                    final long decoderRevision = revision;
                    codec.setOnFrameRenderedListener((active,pts,now)-> {
                        if (currentRevision.get() != decoderRevision || !decoderGate.rendered(pts)) return;
                        try {
                            NativeEngine.displayRendered(id,decoderRevision,pts,now,System.nanoTime());
                            if (first.getAndSet(false)) listener.rendered(decoderRevision);
                            long count = rendered.incrementAndGet();
                            if (count == 1 || count % 120 == 0) android.util.Log.i("ExtendMedia", "Stream stats: " + NativeEngine.displayState());
                        }
                        catch (Exception ended) { outputError.compareAndSet(null,ended); }
                    },new Handler(events.getLooper()));
                    codec.start();
                    final MediaCodec active = codec;draining.set(true);
                    outputWorker = new Thread(()-> {
                        MediaCodec.BufferInfo info = new MediaCodec.BufferInfo();
                        try { while (draining.get()) { DisplaySession.drain(active,info,decoderGate);Thread.sleep(1); } }
                        catch (Exception error) { if (draining.get()) outputError.compareAndSet(null,error); }
                    },"extend-media-decoder-output");outputWorker.start();
                    android.util.Log.i("ExtendMedia","Hardware decoder: " + selected.getName() + ", low latency=" + lowLatency + ", " + width + "x" + height + "@" + fps);
                }
                int slot;long deadline = System.nanoTime()+1_000_000_000L;
                while ((slot=codec.dequeueInputBuffer(2000))<0) {
                    if (System.nanoTime()>deadline) throw new IOException("Hardware decoder stalled");
                    if (outputError.get()!=null) throw outputError.get();
                }
                ByteBuffer buffer = codec.getInputBuffer(slot);
                if (buffer==null || buffer.capacity()<payload.length-17) throw new IOException("Encoded frame exceeds decoder buffer");
                buffer.clear();buffer.put(payload,17,payload.length-17);
                codec.queueInputBuffer(slot,0,payload.length-17,timestamp/1000,0);
            }
        } finally {
            NativeEngine.cancelDisplay(id);draining.set(false);
            currentRevision.set(-1);
            if (outputGate != null) outputGate.close();
            if (outputWorker!=null) outputWorker.join(2000);
            if (codec!=null) codec.release();
            events.quitSafely();events.join(2000);
        }
    }
}
