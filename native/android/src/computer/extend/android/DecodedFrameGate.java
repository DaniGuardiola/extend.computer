package computer.extend.android;

import java.io.IOException;
import java.util.LinkedHashSet;
import java.util.concurrent.TimeUnit;

/** One decoder's actual output geometry and bounded permits for frames submitted to its Surface. */
final class DecodedFrameGate {
    static final int MAX_PERMITS = 64;
    static final long MISMATCH_TIMEOUT_NS = TimeUnit.SECONDS.toNanos(3);
    private final int expectedWidth, expectedHeight;
    private final LinkedHashSet<Long> permits = new LinkedHashSet<>();
    private long highestPermitted = -1, highestRendered = -1, mismatches;
    private Long mismatchStarted;
    private int lastWidth, lastHeight;
    private boolean closed;

    DecodedFrameGate(int width, int height) {
        if (width <= 0 || height <= 0 || width > 4096 || height > 4096)
            throw new IllegalArgumentException("Invalid expected decoder raster");
        expectedWidth = width; expectedHeight = height;
    }
    private static int visible(int coded, Integer first, Integer last) throws IOException {
        if (coded <= 0 || (first == null) != (last == null)) throw new IOException("Malformed decoder output geometry");
        if (first == null) return coded;
        if (first < 0 || last < first || last >= coded) throw new IOException("Malformed decoder output crop");
        return (int)((long)last - first + 1);
    }
    synchronized boolean accepts(int codedWidth, int codedHeight, Integer left, Integer right, Integer top, Integer bottom, long now) throws IOException {
        if (closed) throw new IOException("Decoder output gate closed");
        int width = visible(codedWidth, left, right), height = visible(codedHeight, top, bottom);
        if (width == expectedWidth && height == expectedHeight) { mismatchStarted = null; return true; }
        lastWidth = width; lastHeight = height; mismatches++;
        if (mismatchStarted == null) mismatchStarted = now;
        checkDeadline(now);
        return false;
    }
    synchronized void checkDeadline(long now) throws IOException {
        if (mismatchStarted != null && now - mismatchStarted >= MISMATCH_TIMEOUT_NS)
            throw new IOException("Decoder output size didn't match this display (expected=" + expectedWidth + "x" + expectedHeight
                + ", actual=" + lastWidth + "x" + lastHeight + ", rejected_outputs=" + mismatches + "). Reconnect to retry.");
    }
    synchronized boolean permit(long pts) {
        if (closed || pts < 0 || pts <= highestPermitted) return false;
        highestPermitted = pts;
        if (permits.size() == MAX_PERMITS) permits.remove(permits.iterator().next());
        permits.add(pts);
        return true;
    }
    synchronized void withdraw(long pts) { permits.remove(pts); }
    synchronized boolean rendered(long pts) {
        if (closed || pts < 0 || pts <= highestRendered || !permits.remove(pts)) return false;
        highestRendered = pts;
        permits.removeIf(older -> older < pts);
        return true;
    }
    synchronized void close() { closed = true; permits.clear(); }
}
