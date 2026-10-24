package computer.extend.android;

import android.accessibilityservice.AccessibilityService;
import android.accessibilityservice.AccessibilityServiceInfo;
import android.accessibilityservice.GestureDescription;
import android.accessibilityservice.InputMethod;
import android.app.KeyguardManager;
import android.content.Context;
import android.content.Intent;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.PixelFormat;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.provider.Settings;
import android.util.DisplayMetrics;
import android.view.InputDevice;
import android.view.KeyCharacterMap;
import android.view.KeyEvent;
import android.view.View;
import android.view.WindowManager;
import android.view.accessibility.AccessibilityEvent;
import java.util.ArrayDeque;
import java.util.HashMap;
import java.util.Map;
import org.json.JSONArray;
import org.json.JSONObject;

/** Native, opt-in input adapter. No accessibility content or typed text is logged. */
public final class InputReceivingService extends AccessibilityService {
    private static volatile InputReceivingService active;
    private final Handler handler = new Handler(Looper.getMainLooper());
    private final ArrayDeque<JSONObject> commands = new ArrayDeque<>();
    private final Map<Integer, InputMethod.AccessibilityInputConnection> held = new HashMap<>();
    private volatile long session;
    private long gestureEpoch, gestureStarted;
    private int modifiers, gestureCount, completedGestures;
    private boolean down, gesturePending, stopping;
    private float x, y, strokeX, strokeY;
    private GestureDescription.StrokeDescription stroke;
    private WindowManager windows;
    private View pointer;
    private WindowManager.LayoutParams pointerLayout;
    private volatile String error = "";
    public static boolean enabled(Context c) { return c.getSharedPreferences("engine-local",0).getBoolean("input-receiving",false); }
    public static synchronized void enable(Context c, boolean value) {
        if (!c.getSharedPreferences("engine-local",0).edit().putBoolean("input-receiving",value).commit()) throw new IllegalStateException("Couldn't save input preference");
        InputReceivingService service=active;
        if(value) { if(service!=null)service.error=""; }
        else {
            try { revokeNativeInput(); }
            finally { if(service!=null)service.handler.post(service::reset); }
        }
    }
    public static JSONObject state(Context c) throws Exception {
        InputReceivingService service=active;
        return new JSONObject().put("enabled",enabled(c)).put("permission",service!=null)
            .put("active",service!=null && service.session!=0).put("error",service==null?"":service.error);
    }
    public static void openSettings(Context c) {
        Intent intent=new Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS);
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK); c.startActivity(intent);
    }
    @Override protected void onServiceConnected() {
        super.onServiceConnected();
        if (Build.VERSION.SDK_INT>=33) {
            AccessibilityServiceInfo info=getServiceInfo(); info.flags |= AccessibilityServiceInfo.FLAG_INPUT_METHOD_EDITOR; setServiceInfo(info);
        }
        windows=(WindowManager)getSystemService(WINDOW_SERVICE);
        synchronized (InputReceivingService.class) { revokeNativeInput(); active=this; stopping=false; }
        handler.post(poll);
    }
    @Override public void onAccessibilityEvent(AccessibilityEvent event) { /* Never retain or transmit accessibility content. */ }
    @Override public void onInterrupt() {
        synchronized (InputReceivingService.class) {
            if(stopping || active!=this)return;
            android.util.Log.i("ExtendInput","Accessibility feedback interrupted"); enable(this,false); reset();
        }
    }
    private static void revokeNativeInput() {
        try { NativeEngine.configureInputKeyboardAvailable(false); }
        finally { revokeNativeTextAndInput(); }
    }
    private static void revokeNativeTextAndInput() {
        try { NativeEngine.configureTextReceiver(false); }
        // No display is exposed while disabled; still honor the protocol's valid minimum.
        finally { NativeEngine.configureInputReceiver(false,16,16); }
    }
    private void stopInput() {
        stopping=true; handler.removeCallbacksAndMessages(null);
        try {
            synchronized (InputReceivingService.class) {
                if(active==this) {
                    try { revokeNativeInput(); }
                    catch(RuntimeException failure) { android.util.Log.w("ExtendInput","Native input teardown failed"); }
                    finally { active=null; }
                }
            }
        } finally {
            reset();
        }
    }
    @Override public boolean onUnbind(Intent intent) {
        boolean rebind;
        try { stopInput(); }
        finally { rebind=super.onUnbind(intent); }
        return rebind;
    }
    @Override public void onDestroy() {
        try { stopInput(); }
        finally { super.onDestroy(); }
    }
    private final Runnable poll=new Runnable() {
        @Override public void run() {
            if(stopping || active!=InputReceivingService.this)return;
            try {
                DisplayMetrics metrics=new DisplayMetrics(); windows.getDefaultDisplay().getRealMetrics(metrics);
                KeyguardManager lock=(KeyguardManager)getSystemService(KEYGUARD_SERVICE);
                boolean allowed;
                synchronized (InputReceivingService.class) {
                    if(stopping || active!=InputReceivingService.this)return;
                    allowed=enabled(InputReceivingService.this) && ReceivingService.enabled(InputReceivingService.this)
                        && "available".equals(ReceivingService.state(InputReceivingService.this).getString("state")) && !lock.isKeyguardLocked();
                    NativeEngine.configureInputKeyboardAvailable(allowed && Build.VERSION.SDK_INT>=33 && getInputMethod()!=null);
                    NativeEngine.configureTextReceiver(allowed && Build.VERSION.SDK_INT>=33);
                    NativeEngine.configureInputReceiver(allowed,metrics.widthPixels,metrics.heightPixels);
                }
                if(!allowed)reset();
                // Keep platform liveness current even while a gesture callback is pending.
                JSONArray events=new JSONArray(NativeEngine.pollInputReceiver());
                if(commands.size()+events.length()>256)throw new IllegalStateException("Input receiver queue full");
                for(int i=0;i<events.length();i++) {
                    JSONObject event=events.getJSONObject(i);
                    if("release".equals(event.optString("kind")) && event.optBoolean("end",false) && event.optLong("id")==session) reset();
                    else commands.addLast(event);
                }
                if(gesturePending && SystemClock.uptimeMillis()-gestureStarted>750)throw new IllegalStateException("Android gesture callback timed out");
                while(!gesturePending && !commands.isEmpty()) {
                    synchronized (InputReceivingService.class) {
                        if(stopping || active!=InputReceivingService.this) { reset(); return; }
                        if(!enabled(InputReceivingService.this) || !ReceivingService.enabled(InputReceivingService.this)) { reset(); break; }
                        apply(commands.removeFirst(),metrics);
                    }
                }
                // Continued stationary strokes produce no events and Android rejects them.
                // The contact remains held until an actual move or final release.
            }catch(Exception failure) {
                synchronized (InputReceivingService.class) {
                    if(stopping || active!=InputReceivingService.this) { reset(); return; }
                    error=failure.getMessage()==null?"Input failed":failure.getMessage(); enable(InputReceivingService.this,false); reset();
                }
                android.util.Log.w("ExtendInput","Input receiver stopped");
            }
            handler.postDelayed(this,16);
        }
    };
    private void apply(JSONObject event,DisplayMetrics m) throws Exception {
        String kind=event.getString("kind"); long id=event.getLong("id");
        if(kind.equals("start")) {
            ArrayDeque<JSONObject> remaining=new ArrayDeque<>(commands); reset(); commands.addAll(remaining); session=id; gestureCount=0;completedGestures=0;x=m.widthPixels/2f; y=m.heightPixels/2f; error=""; showPointer(); return;
        }
        if(id!=session)return;
        switch(kind) {
            case "move":
                x=(float)Math.max(0,Math.min(m.widthPixels-1,event.getDouble("x")*m.widthPixels));
                y=(float)Math.max(0,Math.min(m.heightPixels-1,event.getDouble("y")*m.heightPixels));
                showPointer(); if(down && (x!=strokeX || y!=strokeY))gesture(x,y,true,16); break;
            case "button":
                int button=event.getInt("button"); boolean pressed=event.getBoolean("down");
                if(button==0 && down!=pressed) { down=pressed; gesture(x,y,down,16); }
                else if(button==1 && pressed) { gesture(x,y,false,500); }
                else if(button==2 && pressed) { performGlobalAction(GLOBAL_ACTION_HOME); }
                break;
            case "scroll":
                float endX=Math.max(1,Math.min(m.widthPixels-2,x-event.getInt("dx")));
                float endY=Math.max(1,Math.min(m.heightPixels-2,y-event.getInt("dy")));
                gesture(endX,endY,false,80); break;
            case "modifiers": modifiers=event.getInt("mask"); break;
            case "mac_key": key(event.getInt("code"),event.getBoolean("down"),event.optBoolean("repeat")); break;
            case "text": text(event.getString("value")); break;
            case "release":
                long keep=event.optBoolean("end",false)?0:session;
                ArrayDeque<JSONObject> remaining=new ArrayDeque<>(commands);reset();
                if(keep!=0) { session=keep;commands.addAll(remaining);showPointer(); } break;
            default: throw new IllegalStateException("Unknown input command");
        }
    }
    private void gesture(float endX,float endY,boolean continues,long duration) throws Exception {
        Path path=new Path(); path.moveTo(stroke==null?x:strokeX,stroke==null?y:strokeY); path.lineTo(endX,endY);
        GestureDescription.StrokeDescription next=stroke==null?new GestureDescription.StrokeDescription(path,0,duration,continues):stroke.continueStroke(path,0,duration,continues);
        gestureCount++;long epoch=gestureEpoch; gestureStarted=SystemClock.uptimeMillis(); gesturePending=true; strokeX=endX; strokeY=endY;
        if(!dispatchGesture(new GestureDescription.Builder().addStroke(next).build(),new GestureResultCallback() {
            @Override public void onCompleted(GestureDescription gesture) {
                if(epoch!=gestureEpoch) { if(continues && !stopping && active==InputReceivingService.this)endStroke(next,endX,endY); return; }
                completedGestures++;gesturePending=false; stroke=continues?next:null;
            }
            @Override public void onCancelled(GestureDescription gesture) {
                synchronized (InputReceivingService.class) {
                    if(epoch!=gestureEpoch || stopping || active!=InputReceivingService.this)return;
                    android.util.Log.w("ExtendInput","Gesture canceled: dispatched="+gestureCount+", completed="+completedGestures+", continuing="+continues+", previous="+(stroke!=null)); gesturePending=false; stroke=continues?next:stroke; down=false;
                    if(session!=0) { error="Gesture canceled"; enable(InputReceivingService.this,false); reset(); }
                }
            }
        },handler)) { gesturePending=false; throw new IllegalStateException("Android rejected gesture"); }
    }
    private void reset() {
        gestureEpoch++; commands.clear(); session=0; down=false; modifiers=0;
        GestureDescription.StrokeDescription previous=stroke;
        boolean pending=gesturePending;
        stroke=null; gesturePending=false;
        Map<Integer,InputMethod.AccessibilityInputConnection> pressed=new HashMap<>(held);
        held.clear(); View previousPointer=pointer; pointer=null;
        // Finish a continued touch rather than leaving a pressed pointer after disconnect.
        if(previous!=null && !pending)endStroke(previous,strokeX,strokeY);
        if(Build.VERSION.SDK_INT>=33) {
            for(Map.Entry<Integer,InputMethod.AccessibilityInputConnection> entry:pressed.entrySet()) {
                try {
                    long now=SystemClock.uptimeMillis(); entry.getValue().sendKeyEvent(new KeyEvent(now,now,KeyEvent.ACTION_UP,entry.getKey(),0,0,KeyCharacterMap.VIRTUAL_KEYBOARD,0,KeyEvent.FLAG_SOFT_KEYBOARD,InputDevice.SOURCE_KEYBOARD));
                } catch(RuntimeException unavailable) { android.util.Log.w("ExtendInput","Held key release unavailable"); }
            }
        }
        if(previousPointer!=null) {
            try { windows.removeView(previousPointer); }
            catch(RuntimeException unavailable) { android.util.Log.w("ExtendInput","Pointer teardown unavailable"); }
        }
    }
    private void endStroke(GestureDescription.StrokeDescription previous,float endX,float endY) {
        try { Path end=new Path(); end.moveTo(endX,endY);
            dispatchGesture(new GestureDescription.Builder().addStroke(previous.continueStroke(end,0,1,false)).build(),null,handler);
        }catch(RuntimeException ignored){}
    }
    private void text(String value) throws Exception {
        if(Build.VERSION.SDK_INT<33)throw new IllegalStateException("Native text receiving needs Android 13 or newer");
        if(!held.isEmpty() || modifiers!=0)throw new IllegalStateException("Release physical keys before committing text");
        InputMethod ime=getInputMethod();
        InputMethod.AccessibilityInputConnection connection=ime==null?null:ime.getCurrentInputConnection();
        if(connection==null)throw new IllegalStateException("Focus an editable field to receive text");
        connection.commitText(value,1,null);
    }
    private void key(int mac,boolean pressed,boolean repeat) throws Exception {
        if(mac==53) { if(pressed)performGlobalAction(GLOBAL_ACTION_BACK); return; }
        int code=androidKey(mac); if(code==KeyEvent.KEYCODE_UNKNOWN)throw new IllegalStateException("Key not supported on Android yet");
        if(Build.VERSION.SDK_INT<33)throw new IllegalStateException("Native keyboard receiving needs Android 13 or newer");
        InputMethod ime=getInputMethod(); InputMethod.AccessibilityInputConnection connection=pressed?(ime==null?null:ime.getCurrentInputConnection()):held.get(code);
        if(connection==null) { if(!pressed)return; throw new IllegalStateException("Focus an editable field to receive keyboard input"); }
        int meta=((modifiers&1)!=0?KeyEvent.META_SHIFT_ON:0)|((modifiers&2)!=0?KeyEvent.META_CTRL_ON:0)|((modifiers&4)!=0?KeyEvent.META_ALT_ON:0)|((modifiers&8)!=0?KeyEvent.META_META_ON:0);
        long now=SystemClock.uptimeMillis(); connection.sendKeyEvent(new KeyEvent(now,now,pressed?KeyEvent.ACTION_DOWN:KeyEvent.ACTION_UP,code,repeat?1:0,meta,KeyCharacterMap.VIRTUAL_KEYBOARD,0,KeyEvent.FLAG_SOFT_KEYBOARD,InputDevice.SOURCE_KEYBOARD));
        if(pressed)held.put(code,connection);else held.remove(code);
    }
    private static int androidKey(int mac) {
        int[] letters={0,11,8,2,14,3,5,4,34,38,40,37,46,45,31,35,12,15,1,17,32,9,13,7,16,6};
        for(int i=0;i<letters.length;i++)if(letters[i]==mac)return KeyEvent.KEYCODE_A+i;
        int[] numbers={29,18,19,20,21,23,22,26,28,25};for(int i=0;i<numbers.length;i++)if(numbers[i]==mac)return KeyEvent.KEYCODE_0+i;
        int[] macs={36,51,117,48,49,126,125,123,124,115,119,116,121,27,24,33,30,42,41,39,43,47,44,50};
        int[] keys={66,67,112,61,62,19,20,21,22,122,123,92,93,69,70,71,72,73,74,75,55,56,76,68};
        for(int i=0;i<macs.length;i++)if(macs[i]==mac)return keys[i];return KeyEvent.KEYCODE_UNKNOWN;
    }
    private void showPointer() {
        if(pointer==null) {
            pointer=new View(this) { final Paint paint=new Paint(Paint.ANTI_ALIAS_FLAG);
                @Override protected void onDraw(Canvas c) { paint.setColor(Color.WHITE);paint.setStyle(Paint.Style.STROKE);paint.setStrokeWidth(3);c.drawCircle(12,12,8,paint);paint.setColor(Color.BLACK);paint.setStrokeWidth(1);c.drawCircle(12,12,10,paint); }
            };
            pointerLayout=new WindowManager.LayoutParams(24,24,WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
                WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE|WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE|WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN,PixelFormat.TRANSLUCENT);
            pointerLayout.gravity=android.view.Gravity.TOP|android.view.Gravity.LEFT; pointerLayout.setTitle("Extend remote cursor");
            windows.addView(pointer,pointerLayout);
        }
        pointerLayout.x=Math.round(x)-12; pointerLayout.y=Math.round(y)-12; windows.updateViewLayout(pointer,pointerLayout);
    }
}
