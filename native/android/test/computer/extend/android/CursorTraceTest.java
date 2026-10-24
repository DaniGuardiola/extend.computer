package computer.extend.android;

/** Production sampling and queue accounting without Android clocks, UI, or logcat. */
public final class CursorTraceTest {
    private static void check(boolean value,String message) { if (!value) throw new AssertionError(message); }
    public static void main(String[] args) throws Exception {
        check(!CursorOverlay.useCompositor(29,false) && !CursorOverlay.useCompositor(30,false),"Older Android enabled API31 composition");
        check(CursorOverlay.useCompositor(31,false) && CursorOverlay.useCompositor(36,false),"Eligible Android did not default to composition");
        check(!CursorOverlay.useCompositor(31,true) && !CursorOverlay.useCompositor(36,true),"Canvas debug override was ignored");
        CursorOverlay.CursorTrace sampler = new CursorOverlay.CursorTrace(7);
        int selected=0;
        for (int i=1;i<=120;i++) {
            boolean expected=i<=8 || i%12==0;
            check(sampler.samplePosition()==expected,"Position sampling changed at " + i);
            if (expected) selected++;
        }
        check(selected==18,"Position sample rate unbounded");
        for (int i=0;i<8;i++) check(sampler.sampleShape(1_000_000_000L+i),"Initial shape omitted");
        check(!sampler.sampleShape(1_249_999_999L),"Shape sampling wasn't bounded");
        check(sampler.sampleShape(1_250_000_007L),"Shape interval not admitted");
        CursorOverlay.CursorTrace.Sample sample=sampler.sample("position",3,10,1_000_000,1_050_000,1_070_000);
        sample.publishedNs=1_100_000; sample.appliedNs=2_100_000;
        check(sample.complete("draw",3_100_000),"First completion lost");
        check(!sample.complete("draw",9_100_000),"Duplicate completion logged");
        String line=sample.format();
        check(line.contains("clock=android-elapsed-realtime-ns generation=7"),"Clock/generation ambiguous");
        check(line.contains("revision=3 sequence=10 outcome=draw"),"Cursor identity missing");
        check(line.contains("json_us=50 decode_us=20 publish_us=30 ui_queue_us=1000 draw_wait_us=1000 age_us=2100"),"Within-clock durations wrong");
        CursorOverlay.CursorTrace.Sample coalesced=sampler.sample("position",3,11,1_000_000,1_050_000,1_070_000);
        coalesced.publishedNs=1_100_000; coalesced.complete("superseded-before-ui",1_200_000);
        check(coalesced.format().contains("on_draw_ns=0") && coalesced.format().contains("age_us=-1"),"Undrawn cursor claimed visible latency");
        CursorOverlay.CursorTrace.Sample nativeSubmit=sampler.sample("position",3,12,1_000_000,1_050_000,1_070_000);
        nativeSubmit.complete("native-submit",1_200_000);
        String nativeLine=nativeSubmit.format();
        check(nativeLine.contains("draw_stage=native-transaction-submit on_draw_ns=0 native_submit_ns=1200000"),"Native transaction claimed Canvas drawing");
        check(nativeLine.contains("draw_wait_us=-1 age_us=-1"),"Native submit claimed physical visibility");
        CursorOverlay.CursorTrace cap=new CursorOverlay.CursorTrace(8);
        for (int i=0;i<600;i++) check(cap.sample("position",1,i,1,2,3)!=null,"Cap dropped too early");
        check(cap.sample("position",1,600,1,2,3)==null && cap.capDropped.get()==1,"Report cap exceeded");
        cap.closed.set(true);
        check(cap.sample("position",1,601,1,2,3)==null,"Closed generation continued sampling");
        CursorOverlay.CursorTraceQueue queue=new CursorOverlay.CursorTraceQueue(64);
        for (int i=0;i<65;i++) queue.offer(new CursorOverlay.CursorTrace.Sample(cap,"position",1,i,1,2,3));
        check(cap.queueDropped.get()==1,"Queue overflow not counted");
        CursorOverlay.CursorTrace.Sample summary=new CursorOverlay.CursorTrace.Sample(cap,"summary",-1,-1,0,0,0);
        summary.complete("stopped",10); queue.offer(summary);
        check(cap.queueDropped.get()==2,"Summary eviction not counted");
        int count=0; boolean found=false;
        for (CursorOverlay.CursorTrace.Sample value;(value=queue.poll())!=null;) { count++; found|=value==summary; }
        check(count==64 && found,"Queue bound or final summary failed");
        check(summary.format().contains("queue_dropped_total=2 cap_dropped_total=1 dropped_total=3"),"Aggregate drops missing");
        System.out.println("Cursor trace passed: bounded sampling/cap/queue, summary drops, generation identity, exact Android-clock durations, coalesced and duplicate completion.");
    }
}
