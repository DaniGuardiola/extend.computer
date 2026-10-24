package computer.extend.android;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.hardware.display.DisplayManager;
import android.hardware.display.VirtualDisplay;
import android.media.MediaCodec;
import android.media.MediaFormat;
import android.media.projection.MediaProjection;
import android.media.projection.MediaProjectionManager;
import android.os.Handler;
import android.os.HandlerThread;
import android.os.IBinder;
import android.os.SystemClock;
import android.view.Surface;
import java.io.ByteArrayOutputStream;
import java.nio.ByteBuffer;
import org.json.JSONObject;

/** Projection -> hardware encoder Surface -> JNI encrypted transport. No video in JS. */
public final class ScreenSharingService extends Service {
    private static final String STOP="computer.extend.android.STOP_SCREEN_SHARING";
    private static volatile ScreenSharingService active;
    private static volatile String phase="idle", failure="", target="";
    private HandlerThread thread;
    private Handler worker;
    private MediaProjection projection;
    private VirtualDisplay virtual;
    private MediaCodec codec;
    private Surface surface;
    private String id;
    private JSONObject preferences, prepared, pendingPreferences;
    private java.util.concurrent.CompletableFuture<JSONObject> settingsResult;
    private int capturedWidth, capturedHeight;
    private volatile int resizes;
    private boolean hevc;
    private byte[] parameters=new byte[0];
    private long started, lastKeyframe, lastTimestamp, lastStats;
    private volatile long encoded;
    private volatile boolean closing;
    public static synchronized void request(Context context,String peer,String address) throws Exception {
        JSONObject config=ScreenSharingOptions.config(context,ScreenSharingOptions.read(context,peer));
        ScreenSharingOptions.encoder(config);
        if(active!=null || phase.equals("consent"))throw new IllegalStateException("Screen sharing is already active");
        phase="consent";failure="";target=peer;
        try { context.startActivity(new Intent(context,ScreenSharingActivity.class).putExtra("peer",peer).putExtra("address",address).putExtra("config",config.toString()).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)); }
        catch(RuntimeException error){phase="failed";failure="Couldn't open screen sharing consent";throw error;}
    }
    public static synchronized void consentCanceled(){if(active==null){phase="closed";failure="Screen sharing wasn't started";}}
    public static JSONObject update(Context context,String peer,String value) throws Exception {
        JSONObject requested=ScreenSharingOptions.validate(value);
        ScreenSharingService service=active;
        if(service==null || !peer.equals(target))throw new IllegalStateException("No active sharing session for this device");
        java.util.concurrent.CompletableFuture<JSONObject> result=new java.util.concurrent.CompletableFuture<>();
        if(!service.worker.post(()->service.updateSettings(requested,result)))throw new IllegalStateException("Screen sharing ended");
        try {return result.get(20,java.util.concurrent.TimeUnit.SECONDS);}
        catch(java.util.concurrent.ExecutionException error){throw new IllegalStateException(error.getCause().getMessage(),error.getCause());}
    }
    private void updateSettings(JSONObject requested,java.util.concurrent.CompletableFuture<JSONObject> result) {
        boolean detached=false;
        try {
            if(closing || !phase.equals("streaming") || settingsResult!=null)throw new IllegalStateException("Wait for screen sharing to connect before changing settings");
            if(!requested.getString("codec").equals(preferences.getString("codec")))throw new IllegalStateException("Stop sharing to change the video codec");
            JSONObject next=capturedWidth>0 && capturedHeight>0?ScreenSharingOptions.config(requested,capturedWidth,capturedHeight):ScreenSharingOptions.config(this,requested);
            ScreenSharingOptions.encoder(next);
            if(next.toString().equals(prepared.toString())) {
                ScreenSharingOptions.save(this,target,requested.toString());preferences=requested;result.complete(requested);return;
            }
            virtual.setSurface(null);detached=true;releaseEncoder();
            NativeEngine.configureDisplaySender(id,next.toString());
            pendingPreferences=requested;settingsResult=result;phase="connecting";started=SystemClock.elapsedRealtime();
        }catch(Exception error){result.completeExceptionally(error);if(detached)fail(error);}
    }
    public static JSONObject state() throws Exception {
        ScreenSharingService service=active;
        JSONObject result=new JSONObject().put("state",phase).put("error",failure).put("peer",target);
        if(service!=null){result.put("encoded",service.encoded).put("resizes",service.resizes);JSONObject nativeState=new JSONObject(NativeEngine.displaySenderState());result.put("transport",nativeState);}
        return result;
    }
    public static void stop(Context context){ScreenSharingActivity.cancelPending();context.stopService(new Intent(context,ScreenSharingService.class));}
    @Override public void onCreate(){super.onCreate();thread=new HandlerThread("extend-screen-capture");thread.start();worker=new Handler(thread.getLooper());}
    @Override public int onStartCommand(Intent intent,int flags,int startId) {
        if(intent==null || STOP.equals(intent.getAction())){stopSelf();return START_NOT_STICKY;}
        if(active!=null){return START_NOT_STICKY;}
        try {
            Intent consent=intent.getParcelableExtra("consent");
            if(consent==null)throw new IllegalStateException("Screen sharing consent missing");
            NotificationManager manager=getSystemService(NotificationManager.class);
            manager.createNotificationChannel(new NotificationChannel("extend-screen-sharing","Screen sharing",NotificationManager.IMPORTANCE_LOW));
            PendingIntent open=PendingIntent.getActivity(this,42,getPackageManager().getLaunchIntentForPackage(getPackageName()),PendingIntent.FLAG_IMMUTABLE|PendingIntent.FLAG_UPDATE_CURRENT);
            PendingIntent stop=PendingIntent.getService(this,43,new Intent(this,ScreenSharingService.class).setAction(STOP),PendingIntent.FLAG_IMMUTABLE|PendingIntent.FLAG_UPDATE_CURRENT);
            startForeground(3402,new Notification.Builder(this,"extend-screen-sharing").setSmallIcon(android.R.drawable.ic_menu_view).setContentTitle("Sharing your screen")
                .setContentText("extend.computer is sending this screen to a paired device").setContentIntent(open).setOngoing(true)
                .addAction(new Notification.Action.Builder(null,"Stop sharing",stop).build()).build(),ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION);
            active=this;phase="connecting";failure="";target=intent.getStringExtra("peer");
            projection=getSystemService(MediaProjectionManager.class).getMediaProjection(intent.getIntExtra("result",0),consent);
            projection.registerCallback(new MediaProjection.Callback(){
                @Override public void onStop(){stopSelf();}
                @Override public void onCapturedContentResize(int width,int height){capturedWidth=width;capturedHeight=height;}
            },worker);
            String address=intent.getStringExtra("address"),config=intent.getStringExtra("config");started=SystemClock.elapsedRealtime();
            worker.post(()->start(address,config));
        }catch(Exception error){fail(error);}
        return START_NOT_STICKY;
    }
    private void start(String address,String settings) {
        if(closing)return;
        try {
            NativeEngine.identity(this);
            preferences=ScreenSharingOptions.read(this,target);
            JSONObject config=new JSONObject(settings);
            ScreenSharingOptions.encoder(config);
            JSONObject start=new JSONObject(NativeEngine.startDisplaySender(target,address,config.toString()));id=start.getString("id");
            worker.post(pump);
        }catch(Exception error){fail(error);}
    }
    private void prepare(JSONObject config) throws Exception {
        int width=config.getInt("width"),height=config.getInt("height"),fps=config.getInt("fps");
        hevc=config.getString("codec").equals("hevc");
        String name=ScreenSharingOptions.encoder(config);
        codec=MediaCodec.createByCodecName(name);codec.configure(ScreenSharingOptions.format(config),null,null,MediaCodec.CONFIGURE_FLAG_ENCODE);
        surface=codec.createInputSurface();codec.start();
        if(virtual==null)virtual=projection.createVirtualDisplay("extend.computer screen",width,height,getResources().getConfiguration().densityDpi,DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,surface,null,worker);
        else {virtual.resize(width,height,getResources().getConfiguration().densityDpi);virtual.setSurface(surface);}
        if(virtual==null)throw new IllegalStateException("Screen capture unavailable");
        prepared=new JSONObject(config.toString());
        android.util.Log.i("ExtendCapture","Hardware encoder: "+name+", "+width+"x"+height+"@"+fps);
    }
    private final Runnable pump=new Runnable(){@Override public void run(){
        if(closing)return;
        try {
            JSONObject transport=new JSONObject(NativeEngine.displaySenderState());
            if(!id.equals(transport.optString("id")) || transport.optString("state").equals("failed") || transport.optString("state").equals("closed"))throw new IllegalStateException(transport.optString("error","Screen connection ended"));
            if(codec==null) {
                if(transport.getString("state").equals("streaming")){
                    prepare(transport.getJSONObject("config"));phase="streaming";
                    if(settingsResult!=null){
                        ScreenSharingOptions.save(ScreenSharingService.this,target,pendingPreferences.toString());
                        preferences=pendingPreferences;pendingPreferences=null;
                        java.util.concurrent.CompletableFuture<JSONObject> completed=settingsResult;settingsResult=null;completed.complete(preferences);
                    }
                }
                else if(SystemClock.elapsedRealtime()-started>15000)throw new IllegalStateException("Screen connection timed out");
            } else {
                if(capturedWidth>0 && capturedHeight>0) {
                    JSONObject next=ScreenSharingOptions.config(preferences,capturedWidth,capturedHeight);
                    next.put("fps",prepared.getInt("fps")).put("bitrate",prepared.getInt("bitrate"));
                    if(next.getInt("width")!=prepared.getInt("width") || next.getInt("height")!=prepared.getInt("height")) {
                        ScreenSharingOptions.encoder(next);
                        virtual.setSurface(null);releaseEncoder();
                        NativeEngine.configureDisplaySender(id,next.toString());
                        resizes++;phase="connecting";started=SystemClock.elapsedRealtime();
                        worker.postDelayed(this,5);return;
                    }
                }
                if(transport.optBoolean("keyframe") && SystemClock.elapsedRealtime()-lastKeyframe>300){android.os.Bundle request=new android.os.Bundle();request.putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME,0);codec.setParameters(request);lastKeyframe=SystemClock.elapsedRealtime();}
                MediaCodec.BufferInfo info=new MediaCodec.BufferInfo();
                for(int count=0;count<4;count++) {
                    int index=codec.dequeueOutputBuffer(info,0);
                    if(index==MediaCodec.INFO_OUTPUT_FORMAT_CHANGED){MediaFormat format=codec.getOutputFormat();ByteArrayOutputStream sets=new ByteArrayOutputStream();for(String key:new String[]{"csd-0","csd-1"})if(format.containsKey(key))sets.write(annex(copy(format.getByteBuffer(key))));parameters=sets.toByteArray();continue;}
                    if(index<0)break;
                    try {
                        if(info.size>0){ByteBuffer buffer=codec.getOutputBuffer(index);if(buffer==null || info.size>8*1024*1024)throw new IllegalStateException("Invalid encoder output");buffer.position(info.offset);buffer.limit(info.offset+info.size);byte[] bytes=annex(copy(buffer));
                            if((info.flags&MediaCodec.BUFFER_FLAG_CODEC_CONFIG)!=0)parameters=bytes;
                            else {boolean key=(info.flags&MediaCodec.BUFFER_FLAG_KEY_FRAME)!=0;
                                if(key){ByteArrayOutputStream full=new ByteArrayOutputStream();full.write(parameters);full.write(bytes);bytes=full.toByteArray();AnnexB.parameters(bytes,0,hevc);}
                                long timestamp=info.presentationTimeUs*1000;
                                if(timestamp>lastTimestamp && timestamp>0){NativeEngine.pushDisplayFrame(id,bytes,timestamp,key);lastTimestamp=timestamp;encoded++;}
                            }
                        }
                    }finally{codec.releaseOutputBuffer(index,false);}
                    if((info.flags&MediaCodec.BUFFER_FLAG_END_OF_STREAM)!=0)throw new IllegalStateException("Screen encoder ended");
                }
            }
            if(SystemClock.elapsedRealtime()-lastStats>=1000){android.util.Log.i("ExtendCapture","Capture stats: "+state());lastStats=SystemClock.elapsedRealtime();}
            worker.postDelayed(this,5);
        }catch(Exception error){fail(error);}
    }};
    @Override public void onConfigurationChanged(android.content.res.Configuration config) {
        super.onConfigurationChanged(config);
        if(android.os.Build.VERSION.SDK_INT<34 && worker!=null)worker.post(()->{
            if(closing || preferences==null)return;
            try {JSONObject dimensions=ScreenSharingOptions.config(this,preferences);capturedWidth=dimensions.getInt("width");capturedHeight=dimensions.getInt("height");}
            catch(Exception error){fail(error);}
        });
    }
    private void releaseEncoder() {
        if(codec!=null){try{codec.stop();}catch(RuntimeException ignored){}codec.release();codec=null;}
        if(surface!=null){surface.release();surface=null;}
        parameters=new byte[0];lastKeyframe=0;
    }
    private static byte[] copy(ByteBuffer value) throws Exception {if(value==null || value.remaining()>8*1024*1024)throw new IllegalStateException("Invalid encoder bytes");byte[] bytes=new byte[value.remaining()];value.duplicate().get(bytes);return bytes;}
    static byte[] annex(byte[] bytes) throws Exception {
        if(bytes.length>=4 && bytes[0]==0 && bytes[1]==0 && (bytes[2]==1 || (bytes[2]==0 && bytes[3]==1)))return bytes;
        ByteArrayOutputStream output=new ByteArrayOutputStream();int offset=0;
        while(offset<bytes.length){if(bytes.length-offset<4)throw new IllegalStateException("Malformed encoder frame");int length=ByteBuffer.wrap(bytes,offset,4).getInt();offset+=4;if(length<=0 || length>bytes.length-offset)throw new IllegalStateException("Malformed encoder frame");output.write(new byte[]{0,0,0,1});output.write(bytes,offset,length);offset+=length;}
        return output.toByteArray();
    }
    private void fail(Exception error){failure=error.getMessage()==null?"Screen sharing failed":error.getMessage();phase="failed";stopSelf();}
    @Override public void onDestroy(){closing=true;if(active==this){active=null;if(!phase.equals("failed"))phase="closed";}
        if(worker!=null){worker.removeCallbacksAndMessages(null);worker.post(()->{if(settingsResult!=null){settingsResult.completeExceptionally(new IllegalStateException(failure.isEmpty()?"Screen sharing stopped":failure));settingsResult=null;}if(id!=null)NativeEngine.stopDisplaySender(id);if(virtual!=null)virtual.release();releaseEncoder();if(projection!=null)projection.stop();thread.quitSafely();});}
        super.onDestroy();}
    @Override public IBinder onBind(Intent intent){return null;}
}
