package computer.extend.android;

import java.io.IOException;
import java.util.concurrent.atomic.AtomicInteger;

/** Output metadata may advance before older queued render callbacks arrive. */
public final class DecodedFrameGateTest {
    static void check(boolean value, String message) { if (!value) throw new AssertionError(message); }
    interface Throwing { void run() throws Exception; }
    static void rejected(Throwing operation) throws Exception {
        try { operation.run(); } catch (IOException expected) { return; }
        throw new AssertionError("Malformed or expired output accepted");
    }
    static boolean raster(DecodedFrameGate gate, int width, int height, long now) throws IOException {
        return gate.accepts(width,height,null,null,null,null,now);
    }
    public static void main(String[] args) throws Exception {
        DecodedFrameGate crop = new DecodedFrameGate(1920,1200);
        check(crop.accepts(1920,1216,0,1919,0,1199,0), "Inclusive crop didn't remove codec padding");
        check(!crop.accepts(1920,1216,0,1919,0,1179,0), "Wrong visible height accepted");
        rejected(() -> crop.accepts(1920,1216,0,null,null,null,0));
        rejected(() -> crop.accepts(1920,1216,-1,1919,null,null,0));
        rejected(() -> crop.accepts(1920,1216,10,9,null,null,0));
        rejected(() -> crop.accepts(1920,1216,0,1920,null,null,0));
        rejected(() -> crop.accepts(0,1200,null,null,null,null,0));
        DecodedFrameGate startup = new DecodedFrameGate(2560,1600);
        check(!raster(startup,1280,800,0), "Old 1x raster unlocked new 2x decoder");
        startup.checkDeadline(DecodedFrameGate.MISMATCH_TIMEOUT_NS - 1);
        rejected(() -> startup.checkDeadline(DecodedFrameGate.MISMATCH_TIMEOUT_NS));
        DecodedFrameGate recovered = new DecodedFrameGate(1280,800);
        check(!raster(recovered,1920,1200,0), "Old profile accepted");
        check(raster(recovered,1280,800,DecodedFrameGate.MISMATCH_TIMEOUT_NS - 1), "Fresh decoded geometry rejected");
        recovered.checkDeadline(10 * DecodedFrameGate.MISMATCH_TIMEOUT_NS);
        check(!recovered.rendered(1), "Format metadata alone acknowledged rendering");
        check(recovered.permit(2), "Validated presentation not permitted");
        check(!recovered.rendered(1), "Older callback used a newer matching format");
        check(!recovered.permit(-1) && !recovered.permit(2), "Invalid or duplicate presentation allowed");
        check(recovered.rendered(2) && !recovered.rendered(2), "Duplicate render callback acknowledged");
        check(recovered.permit(3), "Next presentation rejected");
        recovered.withdraw(3);
        check(!recovered.rendered(3), "A failed release-render retained its permit");
        check(recovered.permit(4) && recovered.permit(5), "Ordered output rejected");
        check(!recovered.rendered(999), "Unknown callback poisoned the sequence");
        check(recovered.rendered(5) && !recovered.rendered(4), "Late callback acknowledged stale presentation");
        DecodedFrameGate bounded = new DecodedFrameGate(1280,800);
        for (int i=0;i<DecodedFrameGate.MAX_PERMITS+2;i++) check(bounded.permit(i), "Permit insertion failed");
        check(!bounded.rendered(0) && !bounded.rendered(1), "Permit mailbox exceeded its bound");
        check(bounded.rendered(DecodedFrameGate.MAX_PERMITS+1), "Newest permit lost");
        bounded.permit(100); bounded.close();
        check(!bounded.rendered(100) && !bounded.permit(101), "Retired decoder retained render permits");
        check(!new DecodedFrameGate(1280,800).rendered(100), "Replacement decoder accepted another decoder's callback");
        AtomicInteger legacy = new AtomicInteger();
        DisplaySession.Listener oldListener = new DisplaySession.Listener() {
            public void configured(int width,int height) { }
            public void rendered() { legacy.incrementAndGet(); }
        };
        oldListener.rendered(9);
        check(legacy.get()==1, "Additive revision callback broke legacy listener dispatch");
        System.out.println("Decoded frame gate passed: crop/2x geometry, stale/duplicate callbacks, bounded permits, release rollback, mismatch deadline and legacy listener compatibility.");
    }
}
