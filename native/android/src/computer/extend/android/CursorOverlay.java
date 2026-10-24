package computer.extend.android;

import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.RectF;
import android.os.Handler;
import android.os.Build;
import android.os.Looper;
import android.os.SystemClock;
import android.util.Base64;
import android.util.Log;
import android.view.View;
import android.view.Surface;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import org.json.JSONObject;

/** Native cursor state; never waits for MediaCodec or sends image bytes to React. */
public final class CursorOverlay extends View {
    private static final int MAX_PACKET = 128 * 1024, MAX_PNG = 90 * 1024;
    private final Handler handler = new Handler(Looper.getMainLooper());
    private final AtomicInteger generation = new AtomicInteger();
    private final Object mailbox = new Object();
    private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG | Paint.FILTER_BITMAP_FLAG);
    private final RectF viewport = new RectF();
    private Thread worker;
    private Shape shape, pendingShape;
    private Position position, pendingPosition;
    private boolean scheduled, active;
    private double logicalWidth, logicalHeight;
    private long visibleSequence = -1, visibleRevision = -1;
    private long draws;
    private CursorTrace trace;
    private final AtomicLong compositorHandle = new AtomicLong();
    private final AtomicBoolean compositorSelected = new AtomicBoolean();
    private volatile CompositorTarget compositorTarget;
    private long compositorEpoch;

    private static final class CompositorTarget {
        final Surface surface;
        final long surfaceEpoch, epoch, revision;
        final int rasterWidth, rasterHeight, logicalWidth, logicalHeight;
        final int holderWidth, holderHeight, fittedWidth, fittedHeight;
        final boolean ready;
        CompositorTarget(Surface surface, long surfaceEpoch, long epoch, long revision, int rw, int rh, int lw, int lh,
                int hw, int hh, int vw, int vh, boolean ready) {
            this.surface=surface; this.surfaceEpoch=surfaceEpoch; this.epoch=epoch; this.revision=revision;
            rasterWidth=rw; rasterHeight=rh; logicalWidth=lw; logicalHeight=lh;
            holderWidth=hw; holderHeight=hh; fittedWidth=vw; fittedHeight=vh; this.ready=ready;
        }
        boolean same(CompositorTarget other) {
            return other != null && surface==other.surface && surfaceEpoch==other.surfaceEpoch && revision==other.revision
                && rasterWidth==other.rasterWidth && rasterHeight==other.rasterHeight
                && logicalWidth==other.logicalWidth && logicalHeight==other.logicalHeight
                && holderWidth==other.holderWidth && holderHeight==other.holderHeight
                && fittedWidth==other.fittedWidth && fittedHeight==other.fittedHeight && ready==other.ready;
        }
        boolean sameProfile(CompositorTarget other) {
            return other != null && rasterWidth==other.rasterWidth && rasterHeight==other.rasterHeight
                && logicalWidth==other.logicalWidth && logicalHeight==other.logicalHeight;
        }
    }

    /** Main-thread surface descriptor. Native-window extent is checked again by native code. */
    public void compositorSurface(Surface surface, long surfaceEpoch, long revision, int rw, int rh, int lw, int lh,
            int hw, int hh, int vw, int vh, boolean ready) {
        CompositorTarget next = new CompositorTarget(surface,surfaceEpoch,0,revision,rw,rh,lw,lh,hw,hh,vw,vh,ready);
        synchronized (mailbox) {
            if (next.same(compositorTarget)) return;
            compositorTarget = new CompositorTarget(surface,surfaceEpoch,++compositorEpoch,revision,rw,rh,lw,lh,hw,hh,vw,vh,ready);
            retireCompositor();
        }
    }
    public void compositorSurfaceLost() {
        synchronized (mailbox) { compositorTarget=null; ++compositorEpoch; retireCompositor(); }
    }
    private void retireCompositor() {
        long handle = compositorHandle.getAndSet(0);
        if (handle != 0) NativeEngine.retireCursorCompositor(handle);
    }

    private static final class Shape {
        final long revision;
        final Bitmap bitmap;
        final double width, height, hotspotX, hotspotY;
        CursorTrace.Sample trace;
        Shape(long revision, Bitmap bitmap, double width, double height, double hotspotX, double hotspotY) {
            this.revision = revision; this.bitmap = bitmap; this.width = width; this.height = height;
            this.hotspotX = hotspotX; this.hotspotY = hotspotY;
        }
    }
    private static final class Position {
        final long revision, sequence, receivedMs;
        final double x, y;
        final boolean visible;
        CursorTrace.Sample trace;
        Position(JSONObject value) throws Exception {
            revision = unsigned(value,"revision"); sequence = unsigned(value,"sequence");
            x = value.getDouble("x"); y = value.getDouble("y"); visible = value.getBoolean("visible");
            if (!Double.isFinite(x) || !Double.isFinite(y) || x < 0 || x > 1 || y < 0 || y > 1)
                throw new IllegalArgumentException("Invalid cursor position");
            receivedMs = SystemClock.uptimeMillis();
        }
    }
    private static long unsigned(JSONObject value, String key) throws Exception {
        long result = value.getLong(key);
        if (result < 0) throw new IllegalArgumentException("Invalid cursor sequence");
        return result;
    }
    public CursorOverlay(Context context) {
        super(context);
        setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
        setFocusable(false); setClickable(false); setWillNotDraw(false);
    }
    /** Called on the UI thread; bounds are the same fitted rect as SurfaceView. */
    public void geometry(float left, float top, float width, float height, int sourceWidth, int sourceHeight) {
        viewport.set(left,top,left+width,top+height);
        logicalWidth = sourceWidth; logicalHeight = sourceHeight;
        invalidate();
    }
    public void start(String id) {
        stop();
        active = true;
        int epoch = generation.get();
        // Enable before starting a session: adb shell setprop log.tag.ExtendCursorTrace DEBUG.
        // Trace timestamps share only Android's elapsedRealtimeNanos clock.
        trace = Log.isLoggable("ExtendCursorTrace",Log.DEBUG) ? new CursorTrace(epoch) : null;
        CursorTrace currentTrace = trace;
        // API31+ uses native composition after the independent surface/session
        // gates admit it. Set log.tag.ExtendCursorCanvas=DEBUG before reconnecting
        // to force the existing Canvas backend; this does not enable tracing.
        boolean canvasRequested = Log.isLoggable("ExtendCursorCanvas",Log.DEBUG);
        boolean direct = useCompositor(Build.VERSION.SDK_INT,canvasRequested);
        if (!direct) Log.i("ExtendCursor","Cursor backend: canvas; "
            +(canvasRequested ? "debug override" : "native position/scale requires API31"));
        compositorSelected.set(direct);
        handler.postDelayed(expiry,500);
        worker = new Thread(() -> receive(id,epoch,currentTrace,direct),"extend-display-cursor");
        worker.start();
    }
    static boolean useCompositor(int sdk, boolean canvasRequested) {
        return sdk >= 31 && !canvasRequested;
    }
    public void stop() {
        generation.incrementAndGet(); active = false;
        handler.removeCallbacks(expiry);
        synchronized (mailbox) {
            retireCompositor(); compositorSelected.set(false);
            finish(pendingShape,"stopped"); finish(pendingPosition,"stopped");
            pendingShape = null; pendingPosition = null; scheduled = false;
        }
        finish(shape,"stopped"); finish(position,"stopped");
        if (trace != null) { trace.close(); trace = null; }
        shape = null; position = null;
        visibleSequence = -1; visibleRevision = -1;
        draws = 0;
        Thread previous = worker; worker = null;
        if (previous != null) previous.interrupt();
        invalidate();
    }
    private final Runnable expiry = new Runnable() {
        @Override public void run() {
            if (!active) return;
            invalidate(); handler.postDelayed(this,500);
        }
    };
    private void receive(String id, int epoch, CursorTrace currentTrace, boolean direct) {
        long lastSequence = -1, lastRevision = -1;
        String lastPng = null;
        NativeCursor nativeCursor = direct ? new NativeCursor(id,epoch,currentTrace != null) : null;
        try {
            if (currentTrace != null) CursorTraceLog.start();
            while (generation.get() == epoch && !Thread.currentThread().isInterrupted()) {
                byte[] bytes = NativeEngine.nextDisplayCursor(id);
                if (bytes == null) { if (nativeCursor != null) nativeCursor.refresh(); continue; }
                long returnedNs = currentTrace == null ? 0 : SystemClock.elapsedRealtimeNanos();
                if (bytes.length > MAX_PACKET) throw new IllegalArgumentException("Cursor packet too large");
                JSONObject value = new JSONObject(new String(bytes,StandardCharsets.UTF_8));
                long jsonNs = currentTrace == null ? 0 : SystemClock.elapsedRealtimeNanos();
                Shape nextShape = null; Position nextPosition = null;
                if (value.has("png")) {
                    long revision = unsigned(value,"revision");
                    if (revision < lastRevision) continue;
                    String png = value.getString("png");
                    if (png.length() > 120 * 1024) throw new IllegalArgumentException("Cursor image too large");
                    if (revision == lastRevision && png.equals(lastPng)) continue;
                    nextShape = decode(value,revision,png);
                    if (currentTrace != null && currentTrace.sampleShape(returnedNs))
                        nextShape.trace = currentTrace.sample("shape",revision,-1,returnedNs,jsonNs,SystemClock.elapsedRealtimeNanos());
                    lastRevision = revision; lastPng = png;
                } else {
                    nextPosition = new Position(value);
                    if (nextPosition.sequence <= lastSequence) continue;
                    lastSequence = nextPosition.sequence;
                    if (currentTrace != null && currentTrace.samplePosition())
                        nextPosition.trace = currentTrace.sample("position",nextPosition.revision,nextPosition.sequence,
                            returnedNs,jsonNs,SystemClock.elapsedRealtimeNanos());
                }
                if (nativeCursor == null) publish(epoch,nextShape,nextPosition);
                else nativeCursor.update(nextShape,nextPosition);
            }
        } catch (Exception error) {
            if (generation.get() == epoch) {
                android.util.Log.i("ExtendCursor","Cursor ended: " + error.getMessage());
                handler.post(() -> { if (generation.get() == epoch) stop(); });
            }
        } finally {
            if (nativeCursor != null) nativeCursor.close();
        }
    }
    /** Runs only on this generation's existing receive thread: no per-move FIFO. */
    private final class NativeCursor {
        final String id;
        final int generationEpoch;
        final boolean measuring;
        long handle;
        CompositorTarget target, initialProfile, suspendedTarget;
        Shape cachedShape;
        Position cachedPosition;
        boolean fallback;
        NativeCursor(String id, int epoch, boolean measuring) { this.id=id; generationEpoch=epoch; this.measuring=measuring; }
        void update(Shape nextShape, Position nextPosition) {
            boolean changedImage = nextShape != null && cachedShape != null && nextShape.revision == cachedShape.revision
                && nextShape.bitmap != cachedShape.bitmap;
            if (nextShape != null) { finish(cachedShape,"replaced-before-native-submit"); cachedShape=nextShape; }
            if (nextPosition != null) { finish(cachedPosition,"replaced-before-native-submit"); cachedPosition=nextPosition; }
            if (changedImage && !fallback) { useCanvas("Cursor image changed without a new revision"); return; }
            if (fallback) publish(generationEpoch,nextShape,nextPosition); else refresh();
        }
        void refresh() {
            if (fallback || generation.get()!=generationEpoch) return;
            CompositorTarget next = compositorTarget;
            if (next != target) {
                closeHandle(); target=next;
            }
            if (next==null || !next.ready || next.surface==null || !next.surface.isValid()
                    || cachedShape==null || cachedPosition==null || next==suspendedTarget) return;
            // Cursor packets have no media-config revision. An old shape cannot
            // certify a changed logical/raster profile; use the existing Canvas
            // fallback after retiring the direct layer instead of guessing.
            if (initialProfile != null && !next.sameProfile(initialProfile)) { useCanvas("Display profile changed"); return; }
            if (handle==0) {
                handle=NativeEngine.createCursorCompositor(id,next.surface,next.epoch,next.revision,
                    next.rasterWidth,next.rasterHeight,next.logicalWidth,next.logicalHeight,
                    next.holderWidth,next.holderHeight,next.fittedWidth,next.fittedHeight,measuring);
                if (handle==0) { useCanvas("Native parent geometry or composition unavailable"); return; }
                boolean stale;
                synchronized (mailbox) {
                    stale = generation.get()!=generationEpoch || compositorTarget!=next;
                    if (!stale) compositorHandle.set(handle);
                }
                if (stale) { closeHandle(); return; }
                initialProfile=next;
                Log.i("ExtendCursor","Cursor backend: compositor API31; epoch="+next.epoch+" revision="+next.revision
                    +" raster="+next.rasterWidth+"x"+next.rasterHeight+" holder="+next.holderWidth+"x"+next.holderHeight
                    +" fitted="+next.fittedWidth+"x"+next.fittedHeight
                    +" window_holder_extent_verified=true coordinates=decoded-raster parent_scaling=inherited physical_geometry_verified=false");
            }
            int result=NativeEngine.updateCursorCompositor(handle,next.epoch,next.revision,cachedShape.bitmap,
                cachedShape.revision,cachedShape.width,cachedShape.height,cachedShape.hotspotX,cachedShape.hotspotY,
                cachedPosition.revision,cachedPosition.sequence,cachedPosition.x,cachedPosition.y,cachedPosition.visible,
                cachedPosition.receivedMs,SystemClock.uptimeMillis());
            if (result==1) { closeHandle(); suspendedTarget=next; return; }
            if (result!=0) { useCanvas("Native cursor update failed"); return; }
            if (CursorGeometry.visible(cachedShape.revision,cachedPosition.revision,cachedPosition.visible,
                    cachedPosition.receivedMs,SystemClock.uptimeMillis())) {
                finish(cachedShape,"native-submit"); finish(cachedPosition,"native-submit");
            }
        }
        void useCanvas(String reason) {
            closeHandle(); fallback=true;
            synchronized (mailbox) {
                if (generation.get()!=generationEpoch) return;
                compositorSelected.set(false);
                publish(generationEpoch,cachedShape,cachedPosition);
            }
            Log.i("ExtendCursor","Cursor backend: canvas fallback; "+reason);
        }
        void closeHandle() {
            long previous=handle; handle=0;
            if (previous!=0) {
                compositorHandle.compareAndSet(previous,0);
                NativeEngine.retireCursorCompositor(previous);
                NativeEngine.closeCursorCompositor(previous);
            }
        }
        void close() {
            closeHandle(); finish(cachedShape,"stopped"); finish(cachedPosition,"stopped");
        }
    }
    private static Shape decode(JSONObject value, long revision, String png) throws Exception {
        double width=value.getDouble("width"),height=value.getDouble("height");
        double hotspotX=value.getDouble("hotspot_x"),hotspotY=value.getDouble("hotspot_y");
        // Geometry validation also enforces finite logical size and hotspot bounds.
        CursorGeometry.map(0,0,1,1,1,1,0,0,width,height,hotspotX,hotspotY);
        byte[] data = Base64.decode(png,Base64.DEFAULT);
        if (data.length > MAX_PNG || data.length < 8 || data[0] != (byte)137
                || data[1] != 80 || data[2] != 78 || data[3] != 71
                || data[4] != 13 || data[5] != 10 || data[6] != 26 || data[7] != 10)
            throw new IllegalArgumentException("Invalid cursor PNG");
        BitmapFactory.Options bounds = new BitmapFactory.Options(); bounds.inJustDecodeBounds = true;
        BitmapFactory.decodeByteArray(data,0,data.length,bounds);
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0 || bounds.outWidth > 512 || bounds.outHeight > 512)
            throw new IllegalArgumentException("Invalid cursor PNG dimensions");
        BitmapFactory.Options options = new BitmapFactory.Options(); options.inScaled = false;
        Bitmap bitmap = BitmapFactory.decodeByteArray(data,0,data.length,options);
        if (bitmap == null) throw new IllegalArgumentException("Invalid cursor PNG");
        return new Shape(revision,bitmap,width,height,hotspotX,hotspotY);
    }
    private void publish(int epoch, Shape nextShape, Position nextPosition) {
        synchronized (mailbox) {
            if (generation.get() != epoch) {
                finish(nextShape,"stale-generation"); finish(nextPosition,"stale-generation"); return;
            }
            if (nextShape != null) { finish(pendingShape,"superseded-before-ui"); pendingShape = nextShape; published(nextShape.trace); }
            if (nextPosition != null) { finish(pendingPosition,"superseded-before-ui"); pendingPosition = nextPosition; published(nextPosition.trace); }
            if (scheduled) return;
            scheduled = true;
        }
        handler.post(() -> {
            synchronized (mailbox) {
                if (generation.get() != epoch) return;
                if (pendingShape != null) { finish(shape,"replaced-before-draw"); shape = pendingShape; applied(shape.trace); }
                if (pendingPosition != null) { finish(position,"replaced-before-draw"); position = pendingPosition; applied(position.trace); }
                pendingShape = null; pendingPosition = null; scheduled = false;
            }
            invalidate();
        });
    }
    @Override protected void onDraw(Canvas canvas) {
        super.onDraw(canvas);
        Shape current = shape; Position point = position;
        if (!active || compositorSelected.get() || current == null || point == null
                || !CursorGeometry.visible(current.revision,point.revision,point.visible,point.receivedMs,SystemClock.uptimeMillis())
                || viewport.width() <= 0 || viewport.height() <= 0 || logicalWidth <= 0 || logicalHeight <= 0) return;
        CursorGeometry.Frame frame = CursorGeometry.map(viewport.left,viewport.top,viewport.width(),viewport.height(),
            logicalWidth,logicalHeight,point.x,point.y,current.width,current.height,current.hotspotX,current.hotspotY);
        int saved = canvas.save(); canvas.clipRect(viewport);
        canvas.drawBitmap(current.bitmap,null,new RectF((float)frame.left,(float)frame.top,
            (float)(frame.left+frame.width),(float)(frame.top+frame.height)),paint);
        canvas.restoreToCount(saved);
        finish(current,"draw"); finish(point,"draw");
        if (point.sequence != visibleSequence || current.revision != visibleRevision) {
            draws++;
            if (current.revision != visibleRevision || draws % 120 == 0)
                android.util.Log.i("ExtendCursor","Cursor drawn: revision=" + current.revision + " sequence=" + point.sequence
                    + " updates=" + draws + " uptime_ms=" + SystemClock.uptimeMillis());
            visibleSequence = point.sequence; visibleRevision = current.revision;
        }
    }
    @Override protected void onDetachedFromWindow() { stop(); super.onDetachedFromWindow(); }
    @Override protected void onWindowVisibilityChanged(int visibility) {
        super.onWindowVisibilityChanged(visibility);
        if (visibility != View.VISIBLE) {
            // Keep the compositor worker's cached shape while its parent is hidden;
            // restarting on the same session would need an unsent fresh PNG.
            // Actual detach/session/surface destruction still stops the worker.
            if (compositorSelected.get()) compositorSurfaceLost(); else stop();
        }
    }
    private static void published(CursorTrace.Sample sample) {
        if (sample != null) sample.publishedNs = SystemClock.elapsedRealtimeNanos();
    }
    private static void applied(CursorTrace.Sample sample) {
        if (sample != null) sample.appliedNs = SystemClock.elapsedRealtimeNanos();
    }
    private static void finish(Shape value, String outcome) {
        if (value != null && value.trace != null) value.trace.finish(outcome,SystemClock.elapsedRealtimeNanos());
    }
    private static void finish(Position value, String outcome) {
        if (value != null && value.trace != null) value.trace.finish(outcome,SystemClock.elapsedRealtimeNanos());
    }
    /** Trace logging and formatting run off the UI thread. Queue admission never waits for I/O. */
    private static final class CursorTraceLog {
        private static final CursorTraceQueue QUEUE = new CursorTraceQueue(64);
        static {
            Thread worker = new Thread(() -> {
                while (true) {
                    try { Log.d("ExtendCursorTrace",QUEUE.take().format()); }
                    catch (InterruptedException ignored) { return; }
                }
            },"extend-cursor-trace");
            worker.setDaemon(true); worker.start();
        }
        static void start() { /* Initializes the background logger on the cursor worker. */ }
        static void offer(CursorTrace.Sample sample) { QUEUE.offer(sample); }
    }
    static final class CursorTraceQueue {
        private final ArrayBlockingQueue<CursorTrace.Sample> queue;
        CursorTraceQueue(int limit) { queue = new ArrayBlockingQueue<>(limit); }
        void offer(CursorTrace.Sample sample) {
            if (queue.offer(sample)) return;
            // Preserve the final drop-accounting summary even if sampling filled the queue.
            if ("summary".equals(sample.kind)) {
                CursorTrace.Sample evicted = queue.poll();
                if (evicted != null) evicted.owner.queueDropped.incrementAndGet();
                if (queue.offer(sample)) return;
            }
            sample.owner.queueDropped.incrementAndGet();
        }
        CursorTrace.Sample take() throws InterruptedException { return queue.take(); }
        CursorTrace.Sample poll() { return queue.poll(); }
    }
    /** Package-visible pure accounting allows host tests without Android clocks or logging. */
    static final class CursorTrace {
        private static final int MAX_SAMPLES = 600;
        final int generation;
        final AtomicInteger sampled = new AtomicInteger(), queueDropped = new AtomicInteger(), capDropped = new AtomicInteger();
        final AtomicBoolean closed = new AtomicBoolean();
        private long positions, shapes, lastShapeNs;
        CursorTrace(int generation) { this.generation = generation; }
        boolean samplePosition() { long count = ++positions; return count <= 8 || count % 12 == 0; }
        boolean sampleShape(long nowNs) {
            long count = ++shapes;
            if (count > 8 && nowNs - lastShapeNs < 250_000_000L) return false;
            lastShapeNs = nowNs; return true;
        }
        Sample sample(String kind, long revision, long sequence, long returnedNs, long jsonNs, long decodedNs) {
            if (closed.get()) return null;
            if (sampled.incrementAndGet() > MAX_SAMPLES) { capDropped.incrementAndGet(); return null; }
            return new Sample(this,kind,revision,sequence,returnedNs,jsonNs,decodedNs);
        }
        void close() {
            if (!closed.compareAndSet(false,true)) return;
            Sample summary = new Sample(this,"summary",-1,-1,0,0,0);
            summary.finish("stopped",SystemClock.elapsedRealtimeNanos());
        }
        static final class Sample {
            final CursorTrace owner;
            final String kind;
            final long revision, sequence, returnedNs, jsonNs, decodedNs;
            final AtomicBoolean complete = new AtomicBoolean();
            long publishedNs, appliedNs, finishedNs;
            String outcome;
            Sample(CursorTrace owner, String kind, long revision, long sequence, long returnedNs, long jsonNs, long decodedNs) {
                this.owner=owner; this.kind=kind; this.revision=revision; this.sequence=sequence;
                this.returnedNs=returnedNs; this.jsonNs=jsonNs; this.decodedNs=decodedNs;
            }
            void finish(String outcome, long nowNs) {
                if (complete(outcome,nowNs)) CursorTraceLog.offer(this);
            }
            boolean complete(String outcome, long nowNs) {
                if (!complete.compareAndSet(false,true)) return false;
                this.outcome=outcome; finishedNs=nowNs; return true;
            }
            String format() {
                return "clock=android-elapsed-realtime-ns generation=" + owner.generation + " kind=" + kind
                    + " revision=" + revision + " sequence=" + sequence + " outcome=" + outcome
                    + " jni_return_ns=" + returnedNs + " json_ns=" + jsonNs + " decoded_ns=" + decodedNs
                    + " publish_ns=" + publishedNs + " ui_apply_ns=" + appliedNs
                    // onDraw submits a Canvas operation; it does not prove compositor/physical presentation.
                    + " draw_stage=" + ("native-submit".equals(outcome) ? "native-transaction-submit" : "canvas-submit")
                    + " on_draw_ns=" + ("draw".equals(outcome) ? finishedNs : 0)
                    + " native_submit_ns=" + ("native-submit".equals(outcome) ? finishedNs : 0) + " finished_ns=" + finishedNs
                    + " json_us=" + durationUs(returnedNs,jsonNs) + " decode_us=" + durationUs(jsonNs,decodedNs)
                    + " publish_us=" + durationUs(decodedNs,publishedNs) + " ui_queue_us=" + durationUs(publishedNs,appliedNs)
                    + " draw_wait_us=" + durationUs(appliedNs,"draw".equals(outcome) ? finishedNs : 0)
                    + " age_us=" + durationUs(returnedNs,"draw".equals(outcome) ? finishedNs : 0)
                    + " sampled_total=" + owner.sampled.get() + " queue_dropped_total=" + owner.queueDropped.get()
                    + " cap_dropped_total=" + owner.capDropped.get()
                    + " dropped_total=" + ((long)owner.queueDropped.get()+owner.capDropped.get());
            }
            private static long durationUs(long from, long to) { return from > 0 && to >= from ? (to-from)/1000 : -1; }
        }
    }
}
