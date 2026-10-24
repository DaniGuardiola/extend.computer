package computer.extend.android;

import android.app.Activity;
import android.content.Intent;
import android.media.projection.MediaProjectionManager;
import android.os.Bundle;

/** One system consent token per capture; it never crosses the React bridge. */
public final class ScreenSharingActivity extends Activity {
    private static volatile java.lang.ref.WeakReference<ScreenSharingActivity> pending = new java.lang.ref.WeakReference<>(null);
    private boolean awaitingResult;
    public static void cancelPending() {
        ScreenSharingActivity activity=pending.get();
        if(activity!=null)activity.runOnUiThread(()->{activity.finishActivity(41);ScreenSharingService.consentCanceled();activity.finish();});
    }
    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        pending=new java.lang.ref.WeakReference<>(this);awaitingResult=true;
        if(state!=null) { ScreenSharingService.consentCanceled(); finish(); return; }
        try { startActivityForResult(((MediaProjectionManager)getSystemService(MEDIA_PROJECTION_SERVICE)).createScreenCaptureIntent(),41); }
        catch(RuntimeException error) { ScreenSharingService.consentCanceled(); finish(); }
    }
    @Override protected void onDestroy() {
        if(pending.get()==this)pending.clear();
        if(awaitingResult)ScreenSharingService.consentCanceled();
        super.onDestroy();
    }
    @Override protected void onActivityResult(int request,int result,Intent data) {
        super.onActivityResult(request,result,data);
        if(request!=41)return;
        awaitingResult=false;
        if(result!=RESULT_OK || data==null)ScreenSharingService.consentCanceled();
        else try {
            Intent start=new Intent(this,ScreenSharingService.class).putExtra("consent",data).putExtra("result",result)
                .putExtra("peer",getIntent().getStringExtra("peer")).putExtra("address",getIntent().getStringExtra("address")).putExtra("config",getIntent().getStringExtra("config"));
            startForegroundService(start);
        }catch(RuntimeException error){ScreenSharingService.consentCanceled();}
        finish();
    }
}
