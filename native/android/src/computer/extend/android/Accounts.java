package computer.extend.android;

import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;
import org.json.JSONArray;
import org.json.JSONObject;

/** Native account owner. The RN bridge receives a whitelist, never this credential. */
public final class Accounts implements AutoCloseable {
    interface Store { JSONObject load(String origin) throws Exception; void save(String origin,JSONObject credential) throws Exception; void clear(String origin) throws Exception; }
    interface Identity { JSONObject publicIdentity() throws Exception; String name(); String proof(String key,String challenge) throws Exception; void replace(String scope,JSONArray devices) throws Exception; void clear(); default void relay(String server,String scope,JSONObject credential)throws Exception {} }
    interface Launcher { void open(String url) throws Exception; }
    private static Accounts shared;
    private final Object lock=new Object(), operation=new Object();
    private final Store store;
    private final Identity identity;
    private final Launcher launcher;
    private final AccountHttp http;
    private final long signInTimeoutMillis;
    private final ScheduledExecutorService worker=Executors.newSingleThreadScheduledExecutor();
    private final java.util.concurrent.atomic.AtomicBoolean refreshQueued=new java.util.concurrent.atomic.AtomicBoolean();
    private String origin, error="", email="", deviceId="";
    private JSONObject credential;
    private JSONArray devices=new JSONArray();
    private boolean loaded, storageError, busy, nativePending, closed;
    private long generation, updatedAt, signInDeadline;
    private AccountBrowser browser;
    private java.util.concurrent.ScheduledFuture<?> nativeTimeout;
    private Runnable dismissNative;

    public static synchronized Accounts get(Context supplied) throws Exception {
        if(shared!=null)return shared;
        Context context=supplied.getApplicationContext();
        AccountCredentials credentials=new AccountCredentials(context);
        String origin=AccountProtocol.origin(context.getSharedPreferences("account-config",0).getString("server","https://extend.computer"));
        shared=new Accounts(origin,new Store(){
            public JSONObject load(String origin)throws Exception{return credentials.load(origin);}
            public void save(String origin,JSONObject value)throws Exception{credentials.save(origin,value);}
            public void clear(String origin)throws Exception{credentials.clear(origin);}
        },new Identity(){
            public JSONObject publicIdentity()throws Exception{return NativeEngine.identity(context);}
            public String name(){return NativeEngine.localName(context);}
            public String proof(String key,String challenge){return NativeEngine.accountProof(key,challenge);}
            public void replace(String scope,JSONArray devices){NativeEngine.replaceAccountPeers(scope,devices.toString());}
            public void clear(){NativeEngine.clearAccountPeers();}
            public void relay(String server,String scope,JSONObject c)throws Exception {NativeEngine.configureAccountRelay(server,scope,AccountProtocol.id(c,"device_id"),AccountProtocol.id(c,"device_token"),AccountProtocol.id(c,"registered_fingerprint"));}
        },url->{Intent intent=new Intent(Intent.ACTION_VIEW,Uri.parse(url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);context.startActivity(intent);},new AccountHttp());
        shared.resume();
        shared.worker.scheduleWithFixedDelay(()->shared.resume(),30,30,TimeUnit.SECONDS);
        return shared;
    }
    Accounts(String origin,Store store,Identity identity,Launcher launcher,AccountHttp http) {
        this(origin,store,identity,launcher,http,300000);
    }
    Accounts(String origin,Store store,Identity identity,Launcher launcher,AccountHttp http,long signInTimeoutMillis) {
        if(signInTimeoutMillis<=0 || signInTimeoutMillis>300000)throw new IllegalArgumentException("Invalid sign-in timeout.");
        this.origin=origin;this.store=store;this.identity=identity;this.launcher=launcher;this.http=http;this.signInTimeoutMillis=signInTimeoutMillis;
    }
    private JSONObject copy(JSONObject value)throws Exception{return value==null?null:new JSONObject(value.toString());}
    private void load() throws Exception {
        synchronized(lock){
            if(closed)throw new IllegalStateException("Account service is closed.");
            if(loaded){if(storageError)throw new IllegalStateException(error);return;}
            try{
                JSONObject restored=store.load(origin);
                if(restored!=null){AccountProtocol.id(restored,"token");AccountProtocol.text(restored,"email",320);}
                credential=restored;email=restored==null?"":restored.getString("email");loaded=true;storageError=false;
            }catch(Exception failed){storageError=true;error="Couldn't read saved login. Sign out locally and try again.";throw new IllegalStateException(error);}
        }
    }
    public String state()throws Exception {
        synchronized(lock){
            JSONArray safe=new JSONArray(); boolean fresh=updatedAt!=0 && System.nanoTime()-updatedAt<90000000000L;
            for(int i=0;i<devices.length();i++){
                JSONObject row=new JSONObject(devices.getJSONObject(i).toString());if(!fresh)row.put("online",false);safe.put(row);
            }
            return new JSONObject().put("server",origin).put("email",email.isEmpty()?JSONObject.NULL:email).put("device_id",deviceId.isEmpty()?JSONObject.NULL:deviceId)
                .put("devices",safe).put("browser_pending",browser!=null).put("native_pending",nativePending).put("busy",busy).put("loaded",loaded)
                .put("storage_error",storageError).put("directory_fresh",fresh).put("error",error.isEmpty()?JSONObject.NULL:error).toString();
        }
    }
    public void resume(){synchronized(lock){if(closed)return;if(refreshQueued.compareAndSet(false,true))worker.execute(()->{try{refresh();}catch(Exception failed){synchronized(lock){if(!closed && !storageError)error="Couldn't refresh the account. Check your connection and retry.";}}finally{refreshQueued.set(false);}});}}
    public void prepare()throws Exception{synchronized(operation){load();}}
    void nativeFailed(long ticket){synchronized(lock){if(closed || generation!=ticket)return;if(nativePending){generation++;closeNativeLocked();}error="Couldn't open native sign-in. Return to the app and retry.";}}
    private void current(long ticket)throws Exception{
        if(generation==ticket && signInDeadline!=0 && System.nanoTime()>=signInDeadline && (browser!=null || nativePending)){
            generation++;if(browser!=null)browser.close();browser=null;closeNativeLocked();error="Sign-in timed out. Start again.";
        }
        if(closed || generation!=ticket)throw new IllegalStateException("Account operation canceled.");
    }
    private void commit(long ticket,JSONObject next)throws Exception {
        synchronized(lock){current(ticket);store.save(origin,next);credential=copy(next);email=next.getString("email");deviceId=next.optString("device_id","");}
    }
    private void accept(long ticket,String server,JSONObject response)throws Exception {
        String token=AccountProtocol.id(response,"token");
        try {
            String email=AccountProtocol.text(response.getJSONObject("account"),"email",320);
            synchronized(lock){commit(ticket,new JSONObject().put("token",token).put("email",email).put("key_verified",false));signInDeadline=0;}
        }catch(Exception failed){revoke(server,token);throw failed;}
    }
    private void revoke(String server,String token){try{http.request(server,"/auth/logout","POST",new JSONObject(),token);}catch(Exception ignored){}}
    public String refresh()throws Exception {
        synchronized(operation){
            load(); long ticket;JSONObject next;String server;
            synchronized(lock){if(busy)return state();busy=true;ticket=generation;next=copy(credential);server=origin;}
            try {
                if(next==null){identity.clear();return state();}
                JSONObject local=identity.publicIdentity();String fingerprint=AccountProtocol.id(local,"fingerprint");
                JSONObject user=http.request(server,"/account","GET",null,next.getString("token"));
                String account=AccountProtocol.id(user,"id");next.put("account_id",account).put("email",AccountProtocol.text(user,"email",320));
                String name=identity.name();
                if(!next.has("device_token") || !name.equals(next.optString("registered_name")) || !fingerprint.equals(next.optString("registered_fingerprint"))){
                    JSONObject registered=http.request(server,"/devices","POST",new JSONObject().put("public_key",AccountProtocol.id(local,"public_key"))
                        .put("name",name).put("platform","android"),next.getString("token"));
                    if(!fingerprint.equals(AccountProtocol.id(registered,"fingerprint")))throw new IllegalStateException("Device registration identity mismatch.");
                    next.put("device_id",AccountProtocol.id(registered,"id")).put("device_token",AccountProtocol.id(registered,"device_token")).put("registered_name",name).put("registered_fingerprint",fingerprint).put("key_verified",false);
                    commit(ticket,next);
                }
                String prefix="/devices/"+AccountProtocol.id(next,"device_id");
                if(!next.optBoolean("key_verified",false)){
                    JSONObject options=http.request(server,prefix+"/proof/options","POST",new JSONObject(),next.getString("device_token"));
                    String challenge=AccountProtocol.id(options,"challenge"),key=AccountProtocol.id(options,"server_key");
                    String proof=identity.proof(key,challenge);
                    http.request(server,prefix+"/proof/verify","POST",new JSONObject().put("challenge",challenge).put("proof",proof),next.getString("device_token"));
                    next.put("key_verified",true);commit(ticket,next);
                }
                http.request(server,prefix+"/heartbeat","POST",new JSONObject(),next.getString("device_token"));
                JSONArray directory=AccountProtocol.directory(http.request(server,"/devices","GET",null,next.getString("token")));
                synchronized(lock){current(ticket);commit(ticket,next);identity.replace(server+"/"+account,directory);identity.relay(server,server+"/"+account,next);devices=directory;updatedAt=System.nanoTime();error="";}
            }catch(AccountHttp.Failure failed){
                synchronized(lock){if(generation==ticket){if(failed.status==401)clearLocked();error=failed.getMessage();}}
            }catch(Exception failed){synchronized(lock){if(generation==ticket)error="Couldn't refresh the account. Check your connection and retry.";}}
            finally{synchronized(lock){busy=false;}}
            return state();
        }
    }
    public String browser()throws Exception {
        synchronized(operation){
            load();long ticket;String server;
            synchronized(lock){if(credential!=null || browser!=null || nativePending || busy)throw new IllegalStateException("Sign out or cancel the current sign-in first.");ticket=++generation;server=origin;busy=true;error="";}
            AccountBrowser next=null;
            try {
                JSONObject info=http.request(server,"/server","GET",null,null);
                if(!info.optBoolean("desktop_browser_login",false))throw new IllegalArgumentException("This server provides an account API. Use Sign in with password.");
                next=new AccountBrowser(server,signInTimeoutMillis);AccountBrowser pending=next;
                synchronized(lock){current(ticket);browser=next;signInDeadline=System.nanoTime()+signInTimeoutMillis*1000000L;}
                launcher.open(next.url());
                next.start(new AccountBrowser.Completion(){
                    public void exchange(String code,String verifier)throws Exception{
                        JSONObject value=http.request(server,"/auth/desktop/exchange","POST",new JSONObject().put("code",code).put("verifier",verifier),null);
                        synchronized(operation){accept(ticket,server,value);synchronized(lock){current(ticket);if(browser==pending)browser=null;signInDeadline=0;error="";}}
                        resume();
                    }
                    public void failed(String message){synchronized(lock){if(generation==ticket){browser=null;signInDeadline=0;error=message;}}}
                });
            }catch(Exception failed){if(next!=null)next.close();synchronized(lock){if(generation==ticket){browser=null;signInDeadline=0;error=failed instanceof IllegalArgumentException?failed.getMessage():"Couldn't open browser sign-in. Retry.";}}throw new IllegalStateException(error);}
            finally{synchronized(lock){busy=false;}}
            return state();
        }
    }
    public String cancel()throws Exception {
        synchronized(lock){generation++;if(browser!=null)browser.close();browser=null;closeNativeLocked();error="";}
        return state();
    }
    private void closeNativeLocked(){
        nativePending=false;signInDeadline=0;if(nativeTimeout!=null)nativeTimeout.cancel(false);nativeTimeout=null;
        if(dismissNative!=null)dismissNative.run();dismissNative=null;
    }
    private void clearLocked()throws Exception {
        credential=null;email="";deviceId="";devices=new JSONArray();updatedAt=0;loaded=true;Exception failure=null;
        try{identity.clear();}catch(Exception failed){failure=failed;error="Couldn't close account connections. Retry signing out locally.";}
        try{store.clear(origin);}catch(Exception failed){failure=failed;error="Couldn't remove saved login. Retry signing out locally.";}
        storageError=failure!=null;if(failure!=null)throw new IllegalStateException(error);
    }
    public String logout()throws Exception{return logout(-1);}
    private String logout(long expectedGeneration)throws Exception {
        JSONObject previous;String server;long logoutGeneration;Exception storageFailure=null;
        synchronized(lock){if(expectedGeneration>=0)current(expectedGeneration);logoutGeneration=++generation;if(browser!=null)browser.close();browser=null;closeNativeLocked();previous=copy(credential);server=origin;try{clearLocked();error="";}catch(Exception failed){storageFailure=failed;}}
        if(previous!=null)try{http.request(server,"/auth/logout","POST",new JSONObject(),previous.getString("token"));}
        catch(Exception unreachable){synchronized(lock){if(storageFailure==null && !closed && generation==logoutGeneration && origin.equals(server) && credential==null)error="Signed out locally. Server was unreachable; revoke the session in account settings.";}}
        if(storageFailure!=null)throw storageFailure;
        return state();
    }
    public String configure(Context context,String raw)throws Exception {
        String server=AccountProtocol.origin(raw);
        synchronized(operation){synchronized(lock){load();if(credential!=null || browser!=null || nativePending || busy)throw new IllegalStateException("Sign out or cancel sign-in before changing servers.");
            if(!context.getSharedPreferences("account-config",0).edit().putString("server",server).commit())throw new IllegalStateException("Couldn't save account server.");
            generation++;identity.clear();origin=server;loaded=false;error="";devices=new JSONArray();updatedAt=0;load();}}
        resume();return state();
    }
    public String remove(String id)throws Exception {
        if(!AccountProtocol.hex(id))throw new IllegalArgumentException("Invalid account device.");
        synchronized(operation){JSONObject c;long ticket;String server;
            synchronized(lock){current(generation);if(credential==null || busy)throw new IllegalStateException("Sign in first.");c=copy(credential);ticket=generation;server=origin;}
            try{http.request(server,"/devices/"+id,"DELETE",null,c.getString("token"));}
            catch(AccountHttp.Failure failed){synchronized(lock){if(generation==ticket && failed.status==401){clearLocked();error=failed.getMessage();}}throw failed;}
            if(id.equals(c.optString("device_id")))return logout(ticket);
            synchronized(lock){current(ticket);JSONArray kept=new JSONArray();for(int i=0;i<devices.length();i++)if(!id.equals(devices.getJSONObject(i).getString("id")))kept.put(devices.getJSONObject(i));
                identity.replace(server+"/"+AccountProtocol.id(c,"account_id"),kept);devices=kept;}
            return refresh();
        }
    }
    public void openPage(String page)throws Exception {
        if(!page.equals("signup") && !page.equals("account") && !page.equals("recover"))throw new IllegalArgumentException("Unknown account page.");
        String server;synchronized(lock){current(generation);server=origin;}
        if(!http.request(server,"/server","GET",null,null).optBoolean("desktop_browser_login",false))throw new IllegalStateException("This server provides only an API. Use its administrator's account website.");
        launcher.open(server+"/"+page);
    }
    long beginNative(Runnable dismiss)throws Exception {
        load();synchronized(lock){if(storageError)throw new IllegalStateException("Sign out locally before signing in.");if(credential!=null || browser!=null || nativePending || busy)throw new IllegalStateException("Cancel the current sign-in first.");nativePending=true;dismissNative=dismiss;error="";long ticket=++generation;signInDeadline=System.nanoTime()+signInTimeoutMillis*1000000L;
            nativeTimeout=worker.schedule(()->{synchronized(lock){if(generation==ticket && nativePending){generation++;closeNativeLocked();error="Sign-in timed out. Start again.";}}},signInTimeoutMillis,TimeUnit.MILLISECONDS);return ticket;}
    }
    JSONObject password(long ticket,String email,String password)throws Exception {
        synchronized(operation){String server;synchronized(lock){current(ticket);server=origin;}
            JSONObject response=http.request(server,"/auth/login","POST",new JSONObject().put("email",email).put("password",password),null);
            return nativeResult(ticket,server,response);
        }
    }
    JSONObject verify(long ticket,String secondFactor,String code)throws Exception {
        synchronized(operation){String server;synchronized(lock){current(ticket);server=origin;}
            JSONObject response=http.request(server,"/auth/mfa/verify","POST",new JSONObject().put("ticket",secondFactor).put("code",code),null);
            return nativeResult(ticket,server,response);
        }
    }
    private JSONObject nativeResult(long ticket,String server,JSONObject response)throws Exception {
        if(response.optBoolean("mfa_required",false)){synchronized(lock){current(ticket);}AccountProtocol.id(response,"ticket");return response;}
        accept(ticket,server,response);synchronized(lock){current(ticket);nativePending=false;dismissNative=null;signInDeadline=0;if(nativeTimeout!=null)nativeTimeout.cancel(false);nativeTimeout=null;error="";}resume();return new JSONObject();
    }
    void nativeClosed(long ticket){synchronized(lock){if(generation==ticket && nativePending){generation++;dismissNative=null;closeNativeLocked();}}}
    void submitNative(Runnable task){synchronized(lock){if(closed)throw new IllegalStateException("Account service is closed.");worker.execute(task);}}
    public void close(){try{synchronized(lock){if(closed)return;closed=true;generation++;if(browser!=null)browser.close();browser=null;closeNativeLocked();identity.clear();}}finally{worker.shutdownNow();}}
}
