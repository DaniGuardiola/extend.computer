package computer.extend.benchmarkcamera;

/** Pure acquisition policy; elapsed time is monotonic, dimensions are activity window pixels. */
final class OrientationGate {
    static final long STABLE_MS = 250;
    private int rotation = -1, width, height;
    private long since;
    boolean observe(int value, int w, int h, long now) {
        if (w <= h || h <= 0 || value < 0 || value > 3) { rotation = -1; return false; }
        if (value != rotation || w != width || h != height) {
            rotation = value; width = w; height = h; since = now; return false;
        }
        return now - since >= STABLE_MS;
    }
    static boolean supportsFocusRegions(int hint) { return hint == 0 || hint == 180; }
    static boolean matchesRequest(String request, int rotation) {
        return "reverse-landscape".equals(request) ? rotation == 3 : "sensor-landscape".equals(request);
    }
    static int hint(int sensor, int displayRotation) {
        return (sensor - displayRotation * 90 + 360) % 360;
    }
}
