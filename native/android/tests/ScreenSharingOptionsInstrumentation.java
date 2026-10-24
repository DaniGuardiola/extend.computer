package computer.extend.android.tests;

import android.app.Instrumentation;
import android.content.Context;
import android.os.Bundle;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import org.json.JSONObject;

/** Exercise actual installed preferences and hardware capability checks without projection. */
public final class ScreenSharingOptionsInstrumentation extends Instrumentation {
    @Override public void onCreate(Bundle arguments){super.onCreate(arguments);start();}
    private static void require(boolean value,String message){if(!value)throw new AssertionError(message);}
    @Override public void onStart(){
        Bundle result=new Bundle();Context context=getTargetContext();String peer=new String(new char[64]).replace('\0','0');
        android.content.SharedPreferences prefs=context.getSharedPreferences("engine-screen-sharing",0);String original=prefs.getString(peer,null);
        try {
            Class<?> options=context.getClassLoader().loadClass("computer.extend.android.ScreenSharingOptions");
            Method defaults=options.getMethod("defaults"),validate=options.getMethod("validate",String.class),config=options.getMethod("config",Context.class,JSONObject.class);
            JSONObject value=(JSONObject)defaults.invoke(null);require(value.getInt("fps")==60,"Default frame rate mismatch");
            for(String field:new String[]{"fps","codec","quality","resolution"}) {
                JSONObject invalid=new JSONObject(value.toString());invalid.put(field,field.equals("fps")?31:"invalid");
                try {validate.invoke(null,invalid.toString());throw new AssertionError("Invalid "+field+" accepted");}
                catch(InvocationTargetException expected){require(expected.getCause() instanceof IllegalArgumentException,"Wrong validation error");}
            }
            value.put("codec","hevc").put("fps",120).put("quality","detail").put("resolution","fit");
            options.getMethod("save",Context.class,String.class,String.class).invoke(null,context,peer,value.toString());
            JSONObject saved=(JSONObject)options.getMethod("read",Context.class,String.class).invoke(null,context,peer);
            require(saved.toString().equals(value.toString()),"Preferences did not round-trip");
            JSONObject capture=(JSONObject)config.invoke(null,context,saved);require(capture.getInt("bitrate")==40000000,"Quality bitrate mismatch");
            require(capture.getInt("width")>=640 && capture.getInt("height")>=480,"Invalid protocol geometry");
            require(capture.getInt("width")<=4096 && capture.getInt("height")<=4096,"Oversized screen geometry");
            prefs.edit().putString(peer,"broken").commit();JSONObject recovered=(JSONObject)options.getMethod("read",Context.class,String.class).invoke(null,context,peer);
            require(recovered.toString().equals(defaults.invoke(null).toString()),"Malformed saved preferences did not recover");
            JSONObject support=new JSONObject();
            for(String codec:new String[]{"h264","hevc"}) {
                JSONObject requested=(JSONObject)defaults.invoke(null);requested.put("codec",codec);
                JSONObject candidate=(JSONObject)config.invoke(null,context,requested);
                String name=(String)options.getMethod("encoder",JSONObject.class).invoke(null,candidate);require(name!=null && !name.isEmpty(),"Hardware encoder missing");support.put(codec,name);
            }
            org.json.JSONArray profiles=new org.json.JSONArray();
            for(String codec:new String[]{"h264","hevc"}) for(String resolution:new String[]{"1280","1920","fit"}) for(int fps:new int[]{30,60,120}) for(String quality:new String[]{"responsive","balanced","detail"}) {
                JSONObject requested=(JSONObject)defaults.invoke(null);requested.put("codec",codec).put("resolution",resolution).put("fps",fps).put("quality",quality);
                JSONObject candidate=(JSONObject)config.invoke(null,context,requested);JSONObject profile=new JSONObject(requested.toString());profile.put("width",candidate.getInt("width")).put("height",candidate.getInt("height"));
                try {profile.put("encoder",options.getMethod("encoder",JSONObject.class).invoke(null,candidate)).put("supported",true);}
                catch(InvocationTargetException unavailable){require(unavailable.getCause() instanceof IllegalStateException,"Unexpected capability error");profile.put("supported",false);}
                profiles.put(profile);
            }
            result.putString("profiles",profiles.toString());result.putString("status","passed");result.putString("encoders",support.toString());finish(-1,result);
        }catch(Throwable error){result.putString("status","failed");result.putString("error",error.toString());finish(0,result);}
        finally {android.content.SharedPreferences.Editor editor=prefs.edit();if(original==null)editor.remove(peer);else editor.putString(peer,original);editor.commit();}
    }
}
