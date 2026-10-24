package computer.extend.android;

import android.app.Activity;
import android.content.Context;
import java.lang.ref.WeakReference;
import java.util.concurrent.atomic.AtomicLong;
import org.json.JSONObject;

/** Reserve before async initialization so leaving the UI also cancels queued startup. */
public final class ManualPairing {
    private static final AtomicLong IDS=new AtomicLong();
    private String operation, savedPeer;
    private boolean started;
    private WeakReference<Activity> owner;

    public synchronized String reserve(Activity activity) {
        if(activity==null || !activity.hasWindowFocus())throw new IllegalStateException("Return to the app to pair a device");
        if(operation!=null)throw new IllegalStateException("Pairing already in progress");
        operation=Long.toString(IDS.incrementAndGet());
        owner=new WeakReference<>(activity);started=false;savedPeer=null;
        return operation;
    }
    public String start(Context context,String id,String address,String code,String name)throws Exception {
        // Identity initialization may wait for another native operation. Cancel
        // does not take that lock and invalidates this reservation immediately.
        NativeEngine.identity(context);
        synchronized(this){
            require(id);
            Activity activity=owner==null?null:owner.get();
            if(activity==null || !activity.hasWindowFocus()){stop(id);throw new IllegalStateException("Return to the app to pair a device");}
            if(started)throw new IllegalStateException("Pairing already in progress");
            String result=NativeEngine.startManualPairing(id,address.trim(),code,name.trim());
            started=true;
            return result;
        }
    }
    public synchronized String state(Context context,String id)throws Exception {
        require(id);
        if(!started)return new JSONObject().put("id",id).put("state","starting").put("error","").toString();
        JSONObject result=new JSONObject(NativeEngine.manualPairingState(id));
        if(result.getString("state").equals("paired")){
            String peer=result.getString("peer");
            if(!peer.equals(savedPeer)){
                JSONObject metadata=new JSONObject().put("address",result.getString("address")).put("name",result.getString("name"));
                if(!context.getSharedPreferences("engine-peers",0).edit().putString(peer,metadata.toString()).commit())
                    result.put("error","Paired device, but its address could not be saved");
                else savedPeer=peer;
            }
        }
        return result.toString();
    }
    public synchronized void stop(String id) {
        // Stale cleanup must never stop a replacement reservation.
        if(operation==null || !operation.equals(id))return;
        try{if(started)NativeEngine.stopManualPairing(id);}
        finally{operation=null;owner=null;started=false;savedPeer=null;}
    }
    public synchronized void stop(){if(operation!=null)stop(operation);}
    private void require(String id){if(operation==null || !operation.equals(id))throw new IllegalStateException("Pairing operation changed");}
}
