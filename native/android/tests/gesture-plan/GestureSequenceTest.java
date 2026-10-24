package computer.extend.android.tests;
import java.util.List;

public final class GestureSequenceTest {
    private static void require(boolean ok, String reason) { if(!ok)throw new AssertionError(reason); }
    public static void main(String[] args) {
        String[] modes={"tap","double-tap","hold-drag","two-finger-scroll","two-finger-tap","cancel-drag","home-drag"};
        for(String mode:modes){
            List<GestureSequence.Step> plan=GestureSequence.make(mode,100,200,300,400,80,650,350);
            long last=-1;int contacts=0;int downs=0;
            for(GestureSequence.Step step:plan){
                require(step.offsetMs>=last,"Time went backwards");last=step.offsetMs;
                int action=step.action&255;
                if(step.action==GestureSequence.HOME){require(contacts==1,"Home must occur during held contact");contacts=0;continue;}
                if(action==GestureSequence.DOWN){require(contacts==0&&step.xs.length==1,"Invalid initial down");contacts=1;downs++;}
                else if(action==GestureSequence.POINTER_DOWN){require(contacts==1&&step.xs.length==2&&(step.action>>8)==1,"Invalid second pointer index/count");contacts=2;}
                else if(action==GestureSequence.POINTER_UP){require(contacts==2&&step.xs.length==2&&(step.action>>8)==1,"Invalid pointer up");contacts=1;}
                else {require(step.xs.length==contacts,"Pointer count changed without transition");if(action==GestureSequence.UP||action==GestureSequence.CANCEL)contacts=0;}
            }
            require(contacts==0,"Held contact leaked");require(downs==(mode.equals("double-tap")?2:1),"Wrong tap count");
            require(last<=11000,"Gesture duration unbounded");
            if(mode.endsWith("drag"))require(plan.get(1).offsetMs>=650,"Drag began before platform hold timeout");
        }
        try { GestureSequence.make("hold-drag",Float.NaN,1,2,3,80,650,350);throw new AssertionError("NaN accepted"); }catch(IllegalArgumentException expected){}
        try { GestureSequence.make("two-finger-scroll",1,2,3,4,80,650,100000);throw new AssertionError("Unbounded duration accepted"); }catch(IllegalArgumentException expected){}
        System.out.println("PASS: seven gesture modes, pointer transitions, hold ordering, bounded durations and invalid coordinates");
    }
}
