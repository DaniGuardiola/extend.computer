package computer.extend.android;

public final class CursorGeometryTest {
    private static void equal(double expected, double actual) {
        if (Math.abs(expected-actual)>0.000001) throw new AssertionError(expected + " != " + actual);
    }
    private static void rejects(Runnable action) {
        try { action.run(); } catch (IllegalArgumentException expected) { return; }
        throw new AssertionError("Invalid cursor geometry accepted");
    }
    public static void main(String[] args) {
        CursorGeometry.Frame frame=CursorGeometry.map(0,0,1000,800,1000,500,.25,.75,20,30,3,5);
        equal(247,frame.left); equal(520,frame.top); equal(20,frame.width); equal(30,frame.height);
        // Fit a Retina video raster using logical dimensions: pointer size stays
        // 20 points, not 10 points from incorrectly treating its source as 2x.
        frame=CursorGeometry.map(0,150,1000,500,1000,500,.25,.75,20,30,3,5);
        equal(247,frame.left); equal(520,frame.top); equal(20,frame.width);
        frame=CursorGeometry.map(100,20,500,250,1000,500,1,0,20,30,3,5);
        equal(598.5,frame.left); equal(17.5,frame.top); equal(10,frame.width); equal(15,frame.height);
        frame=CursorGeometry.map(0,0,500,1000,500,1000,0,1,20,30,3,5);
        equal(-3,frame.left); equal(995,frame.top);
        rejects(()->CursorGeometry.map(0,0,100,100,100,100,Double.NaN,0,20,20,0,0));
        rejects(()->CursorGeometry.map(0,0,100,100,100,100,1.01,0,20,20,0,0));
        rejects(()->CursorGeometry.map(0,0,100,100,100,100,0,0,20,20,21,0));
        rejects(()->CursorGeometry.map(0,0,100,100,100,100,0,0,513,20,0,0));
        rejects(()->CursorGeometry.map(0,0,0,100,100,100,0,0,20,20,0,0));
        if (!CursorGeometry.visible(2,2,true,100,2099)
                || CursorGeometry.visible(2,1,true,100,101)
                || CursorGeometry.visible(2,2,false,100,101)
                || CursorGeometry.visible(2,2,true,100,2100)
                || CursorGeometry.visible(2,2,true,100,99)) throw new AssertionError("Cursor revision/visibility/expiry failed");
        System.out.println("PASS: logical/Retina size, letterbox, hotspot, top-down coordinates, resize/portrait, invalid geometry, visibility/revision/expiry");
    }
}
