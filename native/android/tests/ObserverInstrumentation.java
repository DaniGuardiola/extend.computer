package computer.extend.android.tests;

import android.app.Instrumentation;
import android.app.UiAutomation;
import android.accessibilityservice.AccessibilityService;
import android.graphics.Rect;
import android.os.SystemClock;
import android.view.InputDevice;
import android.view.MotionEvent;
import android.view.ViewConfiguration;
import android.os.Bundle;
import android.view.accessibility.AccessibilityNodeInfo;
import java.util.ArrayList;
import java.util.List;
import org.json.JSONArray;
import org.json.JSONObject;

/** Debug-only UI driver in the app-signed test APK, never in the shipped application. */
public final class ObserverInstrumentation extends Instrumentation {
    private Bundle arguments;
    @Override public void onCreate(Bundle arguments) { super.onCreate(arguments); this.arguments = arguments; start(); }
    private void collect(AccessibilityNodeInfo node, List<AccessibilityNodeInfo> nodes) {
        if (node == null) return;
        nodes.add(node);
        for (int i = 0; i < node.getChildCount(); i++) collect(node.getChild(i), nodes);
    }
    private float coordinate(String key) {
        float value = Float.parseFloat(arguments.getString(key, "NaN"));
        if (!Float.isFinite(value)) throw new IllegalArgumentException("Missing or invalid " + key);
        return value;
    }
    private long number(String key, long fallback) {
        return Long.parseLong(arguments.getString(key, Long.toString(fallback)));
    }
    private String foreground(UiAutomation automation) {
        AccessibilityNodeInfo root = automation.getRootInActiveWindow();
        return root == null || root.getPackageName() == null ? "" : root.getPackageName().toString();
    }
    private boolean inject(UiAutomation automation, GestureSequence.Step step, long downTime, JSONArray trace) throws Exception {
        MotionEvent.PointerProperties[] properties = new MotionEvent.PointerProperties[step.xs.length];
        MotionEvent.PointerCoords[] coordinates = new MotionEvent.PointerCoords[step.xs.length];
        for (int i=0;i<step.xs.length;i++) {
            properties[i]=new MotionEvent.PointerProperties(); properties[i].id=i==0?3:7; properties[i].toolType=MotionEvent.TOOL_TYPE_FINGER;
            coordinates[i]=new MotionEvent.PointerCoords(); coordinates[i].x=step.xs[i]; coordinates[i].y=step.ys[i];
            coordinates[i].pressure=1; coordinates[i].size=1;
        }
        long time=SystemClock.uptimeMillis();
        MotionEvent event=MotionEvent.obtain(downTime,time,step.action,step.xs.length,properties,coordinates,0,0,1,1,0,0,InputDevice.SOURCE_TOUCHSCREEN,0);
        boolean accepted;
        try { accepted=automation.injectInputEvent(event,true); } finally { event.recycle(); }
        trace.put(new JSONObject().put("action",step.action).put("pointers",step.xs.length).put("eventUptimeMs",time).put("downUptimeMs",downTime).put("accepted",accepted));
        return accepted;
    }
    private void gesture(Bundle result) throws Exception {
        UiAutomation automation=getUiAutomation(UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES);
        String pkg=arguments.getString("package", "");
        if(pkg.isEmpty()||!pkg.equals(foreground(automation)))throw new IllegalStateException("Requested package is not the foreground window");
        AccessibilityNodeInfo root=automation.getRootInActiveWindow(); Rect bounds=new Rect();root.getBoundsInScreen(bounds);
        String mode=arguments.getString("gesture", "");
        float x=coordinate("x"),y=coordinate("y");
        float endX=Float.parseFloat(arguments.getString("endX",Float.toString(x))),endY=Float.parseFloat(arguments.getString("endY",Float.toString(y)));
        float gap=Float.parseFloat(arguments.getString("gap","80"));
        List<GestureSequence.Step> steps=GestureSequence.make(mode,x,y,endX,endY,gap,
            number("holdMs",ViewConfiguration.getLongPressTimeout()+150L),number("durationMs",350));
        for(GestureSequence.Step step:steps) for(int i=0;i<step.xs.length;i++)
            if(!bounds.contains((int)step.xs[i],(int)step.ys[i]))throw new IllegalArgumentException("Gesture point is outside the observed app window");
        JSONArray trace=new JSONArray(); long start=SystemClock.uptimeMillis(),down=start;
        GestureSequence.Step held=null; boolean home=false;
        try {
            for(GestureSequence.Step step:steps){
                long wait=start+step.offsetMs-SystemClock.uptimeMillis();if(wait>0)SystemClock.sleep(wait);
                if(step.action==GestureSequence.HOME){
                    if(!automation.performGlobalAction(AccessibilityService.GLOBAL_ACTION_HOME))throw new IllegalStateException("Home action failed");
                    home=true;SystemClock.sleep(250);break;
                }
                if(!pkg.equals(foreground(automation)))throw new IllegalStateException("Foreground changed before gesture finished");
                int masked=step.action&255;
                if(masked==MotionEvent.ACTION_DOWN)down=SystemClock.uptimeMillis();
                if(!inject(automation,step,down,trace))throw new IllegalStateException("Touch injection was rejected");
                held=step;
                if(masked==MotionEvent.ACTION_POINTER_UP)held=new GestureSequence.Step(MotionEvent.ACTION_MOVE,0,new float[]{step.xs[0]},new float[]{step.ys[0]});
                if(masked==MotionEvent.ACTION_UP||masked==MotionEvent.ACTION_CANCEL)held=null;
            }
        } finally {
            if(held!=null){GestureSequence.Step cancel=new GestureSequence.Step(MotionEvent.ACTION_CANCEL,0,held.xs,held.ys);inject(automation,cancel,down,trace);}
            result.putString("gestureEvents",trace.toString());
        }
        if(home&&pkg.equals(foreground(automation)))throw new IllegalStateException("Home did not move the target out of the foreground");
        result.putString("gesture",mode);result.putString("foregroundAfter",foreground(automation));
        result.putString("home",Boolean.toString(home));result.putString("windowBounds",bounds.flattenToString());
        result.putString("status","passed");finish(-1,result);
    }
    @Override public void onStart() {
        Bundle result = new Bundle();
        try {
            getUiAutomation(android.app.UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES).waitForIdle(300, 3000);
            if ("gesture".equals(arguments.getString("action", "dump"))) { gesture(result); return; }
            List<AccessibilityNodeInfo> nodes = new ArrayList<>();
            collect(getUiAutomation(android.app.UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES).getRootInActiveWindow(), nodes);
            String action = arguments.getString("action", "dump");
            String text = arguments.getString("text", "");
            String pkg = arguments.getString("package", "");
            JSONArray details = new JSONArray();
            boolean success = false;
            for (AccessibilityNodeInfo node : nodes) {
                if (!pkg.isEmpty() && !pkg.contentEquals(node.getPackageName() == null ? "" : node.getPackageName())) continue;
                String label = node.getText() == null ? "" : node.getText().toString();
                String description = node.getContentDescription() == null ? "" : node.getContentDescription().toString();
                // Passwords are never included in debug output.
                if (node.isPassword()) continue;
                if ("dump".equals(action)) {
                    success = true;
                    android.graphics.Rect bounds=new android.graphics.Rect();node.getBoundsInScreen(bounds);
                    details.put(new JSONObject().put("text", label)
                        .put("description", description).put("checked", node.isChecked()).put("clickable", node.isClickable())
                        .put("resource", node.getViewIdResourceName()).put("focused",node.isFocused()).put("bounds",bounds.flattenToString()));
                } else if ("scroll".equals(action) && node.isScrollable()) {
                    success = node.performAction(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD);
                    if (success) break;
                } else if ((label.equals(text) || description.equals(text)) && !text.isEmpty()) {
                    if ("click".equals(action)) {
                        AccessibilityNodeInfo target = node;
                        while (target != null && !target.isClickable()) target = target.getParent();
                        if (target != null) success = target.performAction(AccessibilityNodeInfo.ACTION_CLICK);
                    } else if ("setText".equals(action) && node.isEditable()) {
                        Bundle input = new Bundle();
                        input.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, arguments.getString("value", ""));
                        success = node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, input);
                    }
                    if (success) break;
                }
            }
            result.putString("status", success ? "passed" : "failed");
            if (!success) result.putString("error", "Requested app window or UI action was not available");
            result.putString("nodes", details.toString());
            finish(success ? -1 : 0, result);
        } catch (Throwable error) { result.putString("status", "failed"); result.putString("error", error.toString()); finish(0, result); }
    }
}
