package computer.extend.android.tests;

import java.util.ArrayList;
import java.util.List;

/** Pure test-only touchscreen sequence; pointer identifiers intentionally differ from indices. */
public final class GestureSequence {
    public static final int DOWN = 0, UP = 1, MOVE = 2, CANCEL = 3, POINTER_DOWN = 5, POINTER_UP = 6, HOME = -1;
    public static final class Step {
        public final int action;
        public final long offsetMs;
        public final float[] xs, ys;
        Step(int action, long time, float[] xs, float[] ys) { this.action=action; this.offsetMs=time; this.xs=xs; this.ys=ys; }
    }
    private static void add(List<Step> steps,int action,long time,float x,float y,float gap,boolean two) {
        steps.add(new Step(action,time,two?new float[]{x,x+gap}:new float[]{x},two?new float[]{y,y}:new float[]{y}));
    }
    public static List<Step> make(String mode,float x,float y,float endX,float endY,float gap,long hold,long duration) {
        if (!Float.isFinite(x)||!Float.isFinite(y)||!Float.isFinite(endX)||!Float.isFinite(endY)||!Float.isFinite(gap)) throw new IllegalArgumentException("Coordinates must be finite");
        if (x<0||y<0||endX<0||endY<0||gap<=0||gap>4096||hold<0||hold>5000||duration<16||duration>5000) throw new IllegalArgumentException("Gesture arguments out of range");
        boolean two=mode.equals("two-finger-scroll")||mode.equals("two-finger-tap");
        boolean drag=mode.equals("hold-drag")||mode.equals("cancel-drag")||mode.equals("home-drag");
        if (!two&&!drag&&!mode.equals("tap")&&!mode.equals("double-tap")) throw new IllegalArgumentException("Unknown gesture mode");
        List<Step> steps=new ArrayList<>();
        add(steps,DOWN,0,x,y,gap,false);
        if (mode.equals("tap")||mode.equals("double-tap")) {
            add(steps,UP,40,x,y,gap,false);
            if(mode.equals("double-tap")){add(steps,DOWN,120,x,y,gap,false);add(steps,UP,160,x,y,gap,false);}
            return steps;
        }
        long begin=drag?hold:20;
        if(two)add(steps,POINTER_DOWN|(1<<8),20,x,y,gap,true);
        long finish;
        if(mode.equals("two-finger-tap")){finish=60;endX=x;endY=y;}
        else {
            int count=(int)Math.ceil(duration/16.0);
            for(int i=1;i<=count;i++){
                float ratio=(float)i/count;
                add(steps,MOVE,begin+(duration*i/count),x+(endX-x)*ratio,y+(endY-y)*ratio,gap,two);
            }
            finish=begin+duration+20;
        }
        if(mode.equals("cancel-drag"))add(steps,CANCEL,finish,endX,endY,gap,false);
        else if(mode.equals("home-drag"))add(steps,HOME,finish,endX,endY,gap,false);
        else {
            if(two)add(steps,POINTER_UP|(1<<8),finish,endX,endY,gap,true);
            add(steps,UP,finish+(two?20:0),endX,endY,gap,false);
        }
        return steps;
    }
}
