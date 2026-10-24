package computer.extend.android;

/** Source cursor sizes are logical points, independent of encoded Retina pixels. */
final class CursorGeometry {
    static boolean visible(long shapeRevision, long positionRevision, boolean visible, long receivedMs, long nowMs) {
        return visible && shapeRevision == positionRevision && nowMs >= receivedMs && nowMs-receivedMs < 2000;
    }
    static final class Frame {
        final double left, top, width, height;
        Frame(double left, double top, double width, double height) {
            this.left = left; this.top = top; this.width = width; this.height = height;
        }
    }
    static Frame map(double viewportLeft, double viewportTop, double viewportWidth, double viewportHeight,
            double sourceWidth, double sourceHeight, double x, double y,
            double width, double height, double hotspotX, double hotspotY) {
        double[] values = {viewportLeft, viewportTop, viewportWidth, viewportHeight,
            sourceWidth, sourceHeight, x, y, width, height, hotspotX, hotspotY};
        for (double value : values) if (!Double.isFinite(value)) throw new IllegalArgumentException("Invalid cursor geometry");
        if (viewportWidth <= 0 || viewportHeight <= 0 || sourceWidth <= 0 || sourceHeight <= 0
                || x < 0 || x > 1 || y < 0 || y > 1 || width <= 0 || width > 512 || height <= 0 || height > 512
                || hotspotX < 0 || hotspotX > width || hotspotY < 0 || hotspotY > height)
            throw new IllegalArgumentException("Invalid cursor geometry");
        double scale = Math.min(viewportWidth / sourceWidth, viewportHeight / sourceHeight);
        double fittedWidth = sourceWidth * scale, fittedHeight = sourceHeight * scale;
        return new Frame(viewportLeft + (viewportWidth - fittedWidth) / 2 + x * fittedWidth - hotspotX * scale,
            viewportTop + (viewportHeight - fittedHeight) / 2 + y * fittedHeight - hotspotY * scale,
            width * scale, height * scale);
    }
}
