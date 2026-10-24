package computer.extend.android.tests;

import android.app.Instrumentation;
import android.content.Context;
import android.os.Bundle;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import org.json.JSONObject;

/** App-signed native diagnostic. Requires an explicit existing peer and own Mac test window. */
public final class ControlInstrumentation extends Instrumentation {
    private Bundle arguments;
    @Override public void onCreate(Bundle arguments) { super.onCreate(arguments); this.arguments = arguments; start(); }
    @Override public void onStart() {
        Bundle result = new Bundle();
        String id = null;
        Method stop = null;
        try {
            String peer = arguments.getString("peer", "");
            if (!peer.matches("[0-9a-f]{64}")) throw new IllegalArgumentException("Explicit verified peer required");
            Context context = getTargetContext();
            Class<?> engine = context.getClassLoader().loadClass("computer.extend.android.NativeEngine");
            engine.getMethod("identity", Context.class).invoke(null, context);
            result.putString("probe", engine.getMethod("check", Context.class, String.class).invoke(null,context,peer).toString());
            String metadata = context.getSharedPreferences("engine-peers",0).getString(peer,null);
            if (metadata == null) throw new IllegalStateException("No saved device route after probe");
            String address = new JSONObject(metadata).getString("address");
            JSONObject initial = new JSONObject(engine.getMethod("startControl",String.class,String.class).invoke(null,peer,address).toString());
            id = initial.getString("id"); stop = engine.getMethod("stopControl",String.class);
            Method state = engine.getMethod("controlState");
            long deadline = System.nanoTime()+8_000_000_000L;
            JSONObject current;
            do { current=new JSONObject(state.invoke(null).toString()); if (!current.getString("state").equals("connecting")) break; Thread.sleep(20); }
            while (System.nanoTime()<deadline);
            result.putString("control",current.toString());
            if (!current.getString("state").equals("connected")) throw new IllegalStateException("Control did not connect: "+current);
            Method event = engine.getMethod("controlEvent",String.class,String.class);
            for (String value : new String[]{"{\"kind\":\"move\",\"x\":0.5,\"y\":0.5}","{\"kind\":\"button\",\"button\":0,\"down\":true,\"clicks\":1}","{\"kind\":\"button\",\"button\":0,\"down\":false,\"clicks\":1}","{\"kind\":\"mac_key\",\"code\":14,\"down\":true,\"repeat\":false}","{\"kind\":\"mac_key\",\"code\":14,\"down\":false,\"repeat\":false}"}) event.invoke(null,id,value);
            Thread.sleep(500);
            result.putString("sent",state.invoke(null).toString());
            result.putString("status","passed");
        } catch (Throwable error) {
            Throwable cause=error instanceof InvocationTargetException ? error.getCause() : error;
            result.putString("status","failed"); result.putString("error",cause.toString());
        } finally {
            if (id!=null && stop!=null) try { stop.invoke(null,id); } catch (Exception ignored) {}
        }
        finish("passed".equals(result.getString("status")) ? -1 : 0,result);
    }
}
