package computer.extend.android;

import android.content.Context;
import android.graphics.Rect;
import android.media.MediaCodecInfo;
import android.media.MediaCodecList;
import android.media.MediaFormat;
import android.view.WindowManager;
import org.json.JSONObject;

/** Per-peer source preferences and hardware validation before capture consent. */
public final class ScreenSharingOptions {
    private ScreenSharingOptions() {}
    public static final class Unsupported extends IllegalStateException { public Unsupported(String message){super(message);} }
    public static JSONObject defaults() throws Exception {
        return new JSONObject().put("resolution","1280").put("fps",60).put("quality","balanced").put("codec","h264");
    }
    public static JSONObject validate(String input) throws Exception {
        if(input==null || input.length()>1024)throw new IllegalArgumentException("Invalid screen sharing settings");
        JSONObject value=new JSONObject(input);
        String resolution=value.getString("resolution"),quality=value.getString("quality"),codec=value.getString("codec");int fps=value.getInt("fps");
        if(!resolution.equals("fit") && !resolution.equals("1280") && !resolution.equals("1920"))throw new IllegalArgumentException("Choose a supported screen resolution");
        if(fps!=30 && fps!=60 && fps!=120)throw new IllegalArgumentException("Choose 30, 60 or 120 FPS");
        if(!quality.equals("responsive") && !quality.equals("balanced") && !quality.equals("detail"))throw new IllegalArgumentException("Choose a supported video quality");
        if(!codec.equals("h264") && !codec.equals("hevc"))throw new IllegalArgumentException("Choose H.264 or HEVC");
        return new JSONObject().put("resolution",resolution).put("fps",fps).put("quality",quality).put("codec",codec);
    }
    public static JSONObject read(Context context,String peer) throws Exception {
        try {String value=context.getSharedPreferences("engine-screen-sharing",0).getString(peer,null);if(value!=null)return validate(value);}
        catch(Exception invalid){}
        return defaults();
    }
    public static void save(Context context,String peer,String value) throws Exception {
        if(peer==null || !peer.matches("[0-9a-f]{64}"))throw new IllegalArgumentException("Invalid paired device");
        String normalized=validate(value).toString();
        if(!context.getSharedPreferences("engine-screen-sharing",0).edit().putString(peer,normalized).commit())throw new IllegalStateException("Couldn't save screen sharing settings");
    }
    public static JSONObject config(Context context,JSONObject options) throws Exception {
        options=validate(options.toString());WindowManager windows=context.getSystemService(WindowManager.class);Rect bounds;
        if(android.os.Build.VERSION.SDK_INT>=30)bounds=windows.getMaximumWindowMetrics().getBounds();
        else {android.util.DisplayMetrics metrics=new android.util.DisplayMetrics();windows.getDefaultDisplay().getRealMetrics(metrics);bounds=new Rect(0,0,metrics.widthPixels,metrics.heightPixels);}
        return config(options,bounds.width(),bounds.height());
    }
    public static JSONObject config(JSONObject options,int screenWidth,int screenHeight) throws Exception {
        options=validate(options.toString());
        if(screenWidth<=0 || screenHeight<=0 || screenWidth>16384 || screenHeight>16384)throw new IllegalArgumentException("Invalid capture region");
        int edge=options.getString("resolution").equals("fit")?4096:Integer.parseInt(options.getString("resolution"));
        double scale=Math.min(1.,edge/(double)Math.max(screenWidth,screenHeight));
        scale=Math.max(scale,Math.max(640./screenWidth,480./screenHeight));
        int width=(int)(screenWidth*scale)/2*2,height=(int)(screenHeight*scale)/2*2;
        if(width>4096 || height>4096)throw new IllegalStateException("This screen shape needs a different sharing resolution");
        String quality=options.getString("quality");int bitrate=quality.equals("responsive")?8000000:quality.equals("detail")?40000000:20000000;
        return new JSONObject().put("mode","mirror").put("width",width).put("height",height).put("fps",options.getInt("fps")).put("bitrate",bitrate)
            .put("retina",false).put("auto_size",false).put("codec",options.getString("codec")).put("separate_cursor",false);
    }
    public static MediaFormat format(JSONObject config) throws Exception {
        String mime=config.getString("codec").equals("hevc")?"video/hevc":"video/avc";
        MediaFormat format=MediaFormat.createVideoFormat(mime,config.getInt("width"),config.getInt("height"));
        format.setInteger(MediaFormat.KEY_COLOR_FORMAT,MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface);
        format.setInteger(MediaFormat.KEY_BIT_RATE,config.getInt("bitrate"));format.setInteger(MediaFormat.KEY_FRAME_RATE,config.getInt("fps"));
        format.setInteger(MediaFormat.KEY_I_FRAME_INTERVAL,1);format.setInteger(MediaFormat.KEY_PRIORITY,0);
        format.setInteger(MediaFormat.KEY_MAX_B_FRAMES,0);format.setFloat(MediaFormat.KEY_MAX_FPS_TO_ENCODER,(float)config.getInt("fps"));
        return format;
    }
    public static String encoder(JSONObject config) throws Exception {
        MediaFormat format=format(config);String mime=format.getString(MediaFormat.KEY_MIME);
        for(MediaCodecInfo info:new MediaCodecList(MediaCodecList.REGULAR_CODECS).getCodecInfos()) {
            if(!info.isEncoder() || !info.isHardwareAccelerated())continue;
            try {if(info.getCapabilitiesForType(mime).isFormatSupported(format))return info.getName();}
            catch(IllegalArgumentException unsupported){}
        }
        throw new Unsupported("Hardware encoding doesn't support "+config.getInt("width")+" × "+config.getInt("height")+" at "+config.getInt("fps")+" FPS with "+(config.getString("codec").equals("hevc")?"HEVC":"H.264")+". Try a lower resolution or frame rate.");
    }
}
