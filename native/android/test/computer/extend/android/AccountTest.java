package computer.extend.android;

import com.sun.net.httpserver.HttpExchange;
import com.sun.net.httpserver.HttpServer;
import java.io.ByteArrayOutputStream;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.net.URI;
import java.net.URLDecoder;
import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;
import org.json.JSONArray;
import org.json.JSONObject;

/** Real loopback HTTP checks for the native client. Keystore and identity crypto need Android/backend checks. */
public final class AccountTest {
    static final String TOKEN="a".repeat(64), DEVICE_TOKEN="b".repeat(64), OWN="1".repeat(64), PEER="2".repeat(64), ACCOUNT="3".repeat(64), CODE="4".repeat(64), KEY="5".repeat(64), MFA="6".repeat(64);
    static void check(boolean value,String message){if(!value)throw new AssertionError(message);}
    static void equal(Object expected,Object actual){check(expected.equals(actual),expected+" != "+actual);}
    interface Throwing { void run()throws Exception; }
    static void rejected(Throwing action)throws Exception {try{action.run();}catch(Exception expected){return;}throw new AssertionError("Invalid input accepted");}
    static JSONObject object(String raw)throws Exception{return new JSONObject(raw);}
    static JSONObject row(String id,String name,boolean verified)throws Exception{return new JSONObject().put("id",id).put("fingerprint",id).put("name",name).put("platform",id.equals(OWN)?"android":"macos").put("key_verified",verified).put("online",true);}
    static void waitFor(java.util.function.BooleanSupplier done)throws Exception {long until=System.nanoTime()+TimeUnit.SECONDS.toNanos(4);while(!done.getAsBoolean()&&System.nanoTime()<until)Thread.sleep(5);check(done.getAsBoolean(),"Timed out waiting for client state");}
    static void idle(Accounts accounts)throws Exception {CountDownLatch ready=new CountDownLatch(1);accounts.submitNative(ready::countDown);check(ready.await(2,TimeUnit.SECONDS),"Account worker did not drain");}
    static Map<String,String> query(String url)throws Exception{Map<String,String> values=new HashMap<>();for(String pair:new URI(url).getRawQuery().split("&")){String[] pieces=pair.split("=",2);values.put(pieces[0],URLDecoder.decode(pieces[1],"UTF-8"));}return values;}
    static String callback(Map<String,String> q,String state)throws Exception{
        try(Socket socket=new Socket("127.0.0.1",Integer.parseInt(q.get("port")))){
            socket.setSoTimeout(5000);socket.getOutputStream().write(("GET /callback?state="+state+"&code="+CODE+" HTTP/1.1\r\nHost: localhost\r\n\r\n").getBytes(StandardCharsets.US_ASCII));
            return new String(socket.getInputStream().readAllBytes(),StandardCharsets.UTF_8);
        }
    }
    static void sanitized(String state)throws Exception{
        for(String secret:new String[]{TOKEN,DEVICE_TOKEN,CODE,MFA,"native-secret-password","proof-secret"})check(!state.contains(secret),"Credential leaked into RN state");
        JSONObject safe=object(state);for(String key:new String[]{"token","device_token","ticket","verifier","password","proof"})check(!safe.has(key),"Secret state key exposed");
    }
    static final class MemoryStore implements Accounts.Store {
        JSONObject saved;boolean failSave,failClear;
        public synchronized JSONObject load(String origin)throws Exception{return saved==null?null:object(saved.toString());}
        public synchronized void save(String origin,JSONObject value)throws Exception{if(failSave)throw new Exception("Storage rejected");saved=object(value.toString());}
        public synchronized void clear(String origin)throws Exception{if(failClear)throw new Exception("Storage rejected");saved=null;}
    }
    static final class Identity implements Accounts.Identity {
        volatile JSONArray directory=new JSONArray();volatile boolean failClear;final AtomicInteger renewals=new AtomicInteger(),clears=new AtomicInteger(),relayConfigurations=new AtomicInteger();
        public JSONObject publicIdentity()throws Exception{return new JSONObject().put("fingerprint",OWN).put("public_key",KEY);}
        public String name(){return "Android tablet";}
        public String proof(String key,String challenge)throws Exception{equal(KEY,key);equal(CODE,challenge);return "proof-secret";}
        public void replace(String scope,JSONArray value)throws Exception{check(scope.endsWith("/"+ACCOUNT),"Wrong account scope");directory=new JSONArray(value.toString());renewals.incrementAndGet();}
        public void relay(String server,String scope,JSONObject credential)throws Exception {check(directory.length()>0,"Relay configured before verified directory");equal(server+"/"+ACCOUNT,scope);equal(OWN,credential.getString("registered_fingerprint"));equal(OWN,credential.getString("device_id"));equal(DEVICE_TOKEN,credential.getString("device_token"));check(credential.getBoolean("key_verified"),"Relay configured before native proof");relayConfigurations.incrementAndGet();}
        public void clear(){if(failClear)throw new IllegalStateException("Native cleanup rejected");directory=new JSONArray();clears.incrementAndGet();}
    }
    static final class Fixture implements AutoCloseable {
        final HttpServer server=HttpServer.create(new InetSocketAddress("127.0.0.1",0),0);
        final ExecutorService requests=Executors.newCachedThreadPool();
        final MemoryStore store=new MemoryStore();final Identity identity=new Identity();
        final AtomicInteger registers=new AtomicInteger(),proofs=new AtomicInteger(),heartbeats=new AtomicInteger(),revokes=new AtomicInteger(),redirectTrap=new AtomicInteger();
        final CountDownLatch exchanged=new CountDownLatch(1),continueExchange=new CountDownLatch(1),deleting=new CountDownLatch(1),continueDelete=new CountDownLatch(1),loggingOut=new CountDownLatch(1),continueLogout=new CountDownLatch(1);
        final String origin;final Accounts accounts;
        volatile String launched,expectedChallenge;volatile boolean blockExchange,blockLogin,blockDelete,blockLogout,expired,offline,mfa,peerRemoved;
        volatile Throwable serverFailure;
        Fixture()throws Exception{this(300000);}
        Fixture(long signInTimeoutMillis)throws Exception{
            origin="http://127.0.0.1:"+server.getAddress().getPort();
            server.createContext("/",this::handle);server.setExecutor(requests);server.start();
            accounts=new Accounts(origin,store,identity,url->{launched=url;expectedChallenge=query(url).get("challenge");},new AccountHttp(),signInTimeoutMillis);
        }
        void auth(HttpExchange request,String token){equal("Bearer "+token,request.getRequestHeaders().getFirst("Authorization"));}
        JSONObject login()throws Exception{return new JSONObject().put("token",TOKEN).put("account",new JSONObject().put("id",ACCOUNT).put("email","tablet@example.test"));}
        void handle(HttpExchange request){
            try{
                equal(origin,request.getRequestHeaders().getFirst("Origin"));equal("desktop",request.getRequestHeaders().getFirst("X-Extend-Client"));
                String path=request.getRequestURI().getPath(),method=request.getRequestMethod();byte[] bytes=request.getRequestBody().readAllBytes();JSONObject body=bytes.length==0?new JSONObject():object(new String(bytes,StandardCharsets.UTF_8));
                if(path.equals("/v1/server")){reply(request,200,new JSONObject().put("desktop_browser_login",!mfa));return;}
                if(path.equals("/v1/auth/desktop/exchange")){
                    equal("POST",method);equal(CODE,body.getString("code"));equal(expectedChallenge,AccountProtocol.challenge(body.getString("verifier")));check(request.getRequestHeaders().getFirst("Authorization")==null,"Exchange used bearer");
                    exchanged.countDown();if(blockExchange)check(continueExchange.await(4,TimeUnit.SECONDS),"Exchange gate timeout");reply(request,200,login());return;
                }
                if(path.equals("/v1/auth/logout")){auth(request,TOKEN);revokes.incrementAndGet();if(blockLogout){loggingOut.countDown();check(continueLogout.await(4,TimeUnit.SECONDS),"Logout gate timeout");reply(request,503,new JSONObject().put("error",TOKEN));}else reply(request,204,null);return;}
                if(path.equals("/v1/auth/login")){equal("tablet@example.test",body.getString("email"));equal("native-secret-password",body.getString("password"));if(blockLogin){exchanged.countDown();check(continueExchange.await(4,TimeUnit.SECONDS),"Native login gate timeout");}reply(request,200,mfa?new JSONObject().put("mfa_required",true).put("ticket",MFA).put("totp",true).put("recovery",true):login());return;}
                if(path.equals("/v1/auth/mfa/verify")){equal(MFA,body.getString("ticket"));equal("012345",body.getString("code"));reply(request,200,login());return;}
                if(path.equals("/v1/redirect")){request.getResponseHeaders().set("Location",origin+"/v1/trap");reply(request,302,null);return;}
                if(path.equals("/v1/trap")){redirectTrap.incrementAndGet();reply(request,200,new JSONObject());return;}
                if(path.equals("/v1/large")){replyBytes(request,200,new byte[256*1024+1]);return;}
                if(expired){reply(request,401,new JSONObject().put("error",TOKEN));return;}
                if(offline){reply(request,503,new JSONObject().put("error",TOKEN));return;}
                if(path.equals("/v1/account")){auth(request,TOKEN);reply(request,200,new JSONObject().put("id",ACCOUNT).put("email","tablet@example.test"));return;}
                if(path.equals("/v1/devices")&&method.equals("POST")){auth(request,TOKEN);equal("android",body.getString("platform"));equal(KEY,body.getString("public_key"));equal("Android tablet",body.getString("name"));registers.incrementAndGet();reply(request,200,new JSONObject().put("id",OWN).put("fingerprint",OWN).put("device_token",DEVICE_TOKEN));return;}
                if(path.equals("/v1/devices/"+OWN+"/proof/options")){auth(request,DEVICE_TOKEN);reply(request,200,new JSONObject().put("challenge",CODE).put("server_key",KEY));return;}
                if(path.equals("/v1/devices/"+OWN+"/proof/verify")){auth(request,DEVICE_TOKEN);equal(CODE,body.getString("challenge"));equal("proof-secret",body.getString("proof"));proofs.incrementAndGet();reply(request,204,null);return;}
                if(path.equals("/v1/devices/"+OWN+"/heartbeat")){auth(request,DEVICE_TOKEN);heartbeats.incrementAndGet();reply(request,204,null);return;}
                if((path.equals("/v1/devices/"+PEER)||path.equals("/v1/devices/"+OWN))&&method.equals("DELETE")){auth(request,TOKEN);if(blockDelete){deleting.countDown();check(continueDelete.await(4,TimeUnit.SECONDS),"Device removal gate timeout");}peerRemoved=true;reply(request,204,null);return;}
                if(path.equals("/v1/devices")){auth(request,TOKEN);JSONArray rows=new JSONArray().put(row(OWN,"Android tablet",true));if(!peerRemoved)rows.put(row(PEER,"Mac",true).put("token",TOKEN));reply(request,200,new JSONObject().put("devices",rows));return;}
                throw new AssertionError("Unexpected request: "+method+" "+path);
            }catch(Throwable failed){serverFailure=failed;try{reply(request,500,new JSONObject());}catch(Exception ignored){}}
            finally{request.close();}
        }
        void assertServer(){if(serverFailure!=null)throw new AssertionError("Mock contract failed",serverFailure);}
        public void close(){continueExchange.countDown();continueDelete.countDown();continueLogout.countDown();accounts.close();server.stop(1);requests.shutdownNow();assertServer();}
    }
    static void reply(HttpExchange request,int status,JSONObject value)throws Exception{replyBytes(request,status,value==null?null:value.toString().getBytes(StandardCharsets.UTF_8));}
    static void replyBytes(HttpExchange request,int status,byte[] value)throws Exception{request.getResponseHeaders().set("Content-Type","application/json");request.sendResponseHeaders(status,value==null?-1:value.length);if(value!=null)request.getResponseBody().write(value);}
    static void protocol()throws Exception{
        equal("https://example.test",AccountProtocol.origin("HTTPS://EXAMPLE.TEST:443/"));equal("http://[::1]",AccountProtocol.origin("http://[::1]:80/"));
        for(String raw:new String[]{"http://example.test","https://user:pass@example.test","https://example.test/path","https://example.test?x=1","https://example.test#secret","https://example.test:65536"})rejected(()->AccountProtocol.origin(raw));
        equal(CODE,AccountProtocol.callback("GET /callback?state="+TOKEN+"&code="+CODE+" HTTP/1.1",TOKEN));
        for(String target:new String[]{"/callback?state="+TOKEN+"&state="+TOKEN+"&code="+CODE,"/callback?state="+TOKEN+"&code="+CODE+"&extra=x","http://localhost/callback?state="+TOKEN+"&code="+CODE,"/callback?state="+DEVICE_TOKEN+"&code="+CODE})check(AccountProtocol.callback("GET "+target+" HTTP/1.1",TOKEN)==null,"Malformed callback accepted");
        rejected(()->AccountProtocol.directory(new JSONObject().put("devices",new JSONArray().put(row(PEER,"Mac",true)).put(row(PEER,"Mac",true)))));
        equal("GET /callback HTTP/1.1",AccountBrowser.requestLine(new java.io.ByteArrayInputStream("GET /callback HTTP/1.1\r\nHost: localhost\r\n\r\n".getBytes(StandardCharsets.US_ASCII))));
        equal("",AccountBrowser.requestLine(new java.io.ByteArrayInputStream(("GET /callback HTTP/1.1\r\nX: "+"x".repeat(8192)).getBytes(StandardCharsets.US_ASCII))));
    }
    static void browser()throws Exception{
        try(Fixture f=new Fixture()){
            f.accounts.browser();Map<String,String> q=query(f.launched);check(AccountProtocol.hex(q.get("state")),"Missing random state");check(callback(q,TOKEN).startsWith("HTTP/1.1 400"),"Wrong state consumed login");check(object(f.accounts.state()).getBoolean("browser_pending"),"Wrong state ended pending flow");
            String response=callback(q,q.get("state"));check(response.startsWith("HTTP/1.1 303"),"Valid callback failed");check(response.contains("Location: "+f.origin+"/desktop/connected#close"),"Unsafe completion URL");check(!response.contains(CODE)&&!response.contains(q.get("state")),"Callback secrets retained in completion");
            f.accounts.refresh();JSONObject state=object(f.accounts.state());equal("tablet@example.test",state.getString("email"));equal(2,state.getJSONArray("devices").length());equal(1,f.registers.get());equal(1,f.proofs.get());check(f.heartbeats.get()>0,"No heartbeat");check(f.identity.relayConfigurations.get()>0,"No native relay configuration after proof");sanitized(state.toString());
            rejected(()->callback(q,q.get("state")));f.accounts.refresh();equal(1,f.registers.get());equal(1,f.proofs.get());
            int renewed=f.identity.renewals.get();f.offline=true;f.accounts.refresh();equal(renewed,f.identity.renewals.get());sanitized(f.accounts.state());f.offline=false;
            f.accounts.remove(PEER);equal(1,object(f.accounts.state()).getJSONArray("devices").length());equal(1,f.identity.directory.length());
            f.expired=true;f.accounts.refresh();check(object(f.accounts.state()).isNull("email"),"Expired account kept credential");equal(0,f.identity.directory.length());check(f.store.saved==null,"Expired token retained on disk");
        }
    }
    static void nativeFallback()throws Exception{
        try(Fixture f=new Fixture()){
            f.mfa=true;rejected(f.accounts::browser);long ticket=f.accounts.beginNative(()->{});JSONObject second=f.accounts.password(ticket,"tablet@example.test","native-secret-password");check(second.getBoolean("mfa_required"),"Missing MFA challenge");sanitized(f.accounts.state());
            f.accounts.verify(ticket,second.getString("ticket"),"012345");f.accounts.refresh();check(!object(f.accounts.state()).getBoolean("native_pending"),"Completed native form left pending");sanitized(f.accounts.state());
            f.accounts.close();
            try(Accounts restored=new Accounts(f.origin,f.store,f.identity,url->{},new AccountHttp())){restored.refresh();equal("tablet@example.test",object(restored.state()).getString("email"));equal(1,f.registers.get());equal(1,f.proofs.get());}
            f.store.saved.put("registered_fingerprint",PEER);
            try(Accounts recovered=new Accounts(f.origin,f.store,f.identity,url->{},new AccountHttp())){recovered.refresh();equal(2,f.registers.get());equal(2,f.proofs.get());equal(OWN,f.store.saved.getString("registered_fingerprint"));}
            f.store.saved.remove("registered_fingerprint");
            try(Accounts migrated=new Accounts(f.origin,f.store,f.identity,url->{},new AccountHttp())){migrated.refresh();equal(3,f.registers.get());equal(3,f.proofs.get());migrated.logout();check(f.store.saved==null,"Logout didn't clear credential");}
            f.assertServer();
        }
    }
    static void persistenceAndCancellation()throws Exception{
        try(Fixture f=new Fixture()){
            f.store.failSave=true;f.accounts.browser();Map<String,String> q=query(f.launched);check(callback(q,q.get("state")).startsWith("HTTP/1.1 503"),"Failed persistence reported browser success");equal(1,f.revokes.get());check(object(f.accounts.state()).isNull("email"),"Unsaved credential accepted");sanitized(f.accounts.state());
        }
        try(Fixture f=new Fixture()){
            f.blockExchange=true;f.accounts.browser();Map<String,String> q=query(f.launched);ExecutorService callbacks=Executors.newSingleThreadExecutor();
            try{java.util.concurrent.Future<String> result=callbacks.submit(()->callback(q,q.get("state")));check(f.exchanged.await(2,TimeUnit.SECONDS),"Exchange not reached");f.accounts.cancel();f.continueExchange.countDown();String canceled;try{canceled=result.get(4,TimeUnit.SECONDS);}catch(java.util.concurrent.ExecutionException disconnected){check(disconnected.getCause() instanceof java.io.IOException,"Unexpected callback failure");canceled="";}check(!canceled.startsWith("HTTP/1.1 303"),"Canceled exchange reported success");waitFor(()->f.revokes.get()==1);check(f.store.saved==null,"Canceled exchange saved login");check(object(f.accounts.state()).isNull("email"),"Late response restored account");}finally{callbacks.shutdownNow();}
        }
        try(Fixture f=new Fixture()){
            long ticket=f.accounts.beginNative(()->{});f.accounts.nativeClosed(ticket);rejected(()->f.accounts.password(ticket,"tablet@example.test","native-secret-password"));check(object(f.accounts.state()).isNull("email"),"Closed form accepted stale native request");
            long rejected=f.accounts.beginNative(()->{});f.accounts.nativeFailed(rejected);check(!object(f.accounts.state()).getBoolean("native_pending"),"Failed native dialog left sign-in stuck");
            long current=f.accounts.beginNative(()->{});f.accounts.nativeFailed(rejected);check(object(f.accounts.state()).getBoolean("native_pending"),"Stale dialog failure canceled current sign-in");f.accounts.nativeClosed(current);
        }
        try(Fixture f=new Fixture()){
            long ticket=f.accounts.beginNative(()->{});f.accounts.password(ticket,"tablet@example.test","native-secret-password");f.accounts.refresh();f.store.failClear=true;rejected(f.accounts::logout);
            equal(1,f.revokes.get());check(object(f.accounts.state()).isNull("email"),"Failed disk clear retained in-memory login");equal(0,f.identity.directory.length());check(object(f.accounts.state()).getBoolean("storage_error"),"Failed disk clear hid recovery state");
            int renewals=f.identity.renewals.get();rejected(f.accounts::refresh);rejected(f.accounts::browser);equal(renewals,f.identity.renewals.get());f.store.failClear=false;f.accounts.logout();check(f.store.saved==null,"Local sign-out recovery failed");
        }
        try(Fixture f=new Fixture()){
            long ticket=f.accounts.beginNative(()->{});f.accounts.password(ticket,"tablet@example.test","native-secret-password");f.accounts.refresh();f.identity.failClear=true;rejected(f.accounts::logout);
            equal(1,f.revokes.get());check(f.store.saved==null,"Native cleanup failure retained credential on disk");check(object(f.accounts.state()).isNull("email"),"Native cleanup failure retained in-memory login");rejected(f.accounts::refresh);f.identity.failClear=false;f.accounts.logout();equal(0,f.identity.directory.length());
        }
        try(Fixture f=new Fixture()){
            long first=f.accounts.beginNative(()->{});f.accounts.password(first,"tablet@example.test","native-secret-password");f.accounts.refresh();idle(f.accounts);f.blockDelete=true;ExecutorService removes=Executors.newSingleThreadExecutor();
            try{java.util.concurrent.Future<String> removed=removes.submit(()->f.accounts.remove(OWN));check(f.deleting.await(2,TimeUnit.SECONDS),"Self-device removal not reached");f.accounts.logout();long next=f.accounts.beginNative(()->{});f.continueDelete.countDown();
                try{removed.get(2,TimeUnit.SECONDS);throw new AssertionError("Stale self-removal applied");}catch(java.util.concurrent.ExecutionException expected){check(expected.getCause() instanceof IllegalStateException,"Unexpected self-removal failure");}
                check(object(f.accounts.state()).getBoolean("native_pending"),"Old self-removal canceled new sign-in");equal(1,f.revokes.get());f.accounts.nativeClosed(next);
            }finally{removes.shutdownNow();}
        }
        try(Fixture f=new Fixture()){
            long first=f.accounts.beginNative(()->{});f.accounts.password(first,"tablet@example.test","native-secret-password");f.accounts.refresh();idle(f.accounts);f.blockLogout=true;ExecutorService logouts=Executors.newSingleThreadExecutor();
            try{java.util.concurrent.Future<String> loggedOut=logouts.submit(f.accounts::logout);check(f.loggingOut.await(2,TimeUnit.SECONDS),"Old logout request not reached");long next=f.accounts.beginNative(()->{});f.accounts.password(next,"tablet@example.test","native-secret-password");f.accounts.refresh();f.continueLogout.countDown();loggedOut.get(2,TimeUnit.SECONDS);
                JSONObject state=object(f.accounts.state());equal("tablet@example.test",state.getString("email"));check(state.isNull("error"),"Old remote logout failure overlaid new login");check(f.store.saved!=null,"Old remote logout failure cleared new credential");
            }finally{logouts.shutdownNow();}
        }
    }
    static void boundsAndTimeout()throws Exception{
        try(Fixture f=new Fixture()){
            AccountHttp http=new AccountHttp();rejected(()->http.request(f.origin,"/redirect","GET",null,TOKEN));equal(0,f.redirectTrap.get());rejected(()->http.request(f.origin,"/large","GET",null,TOKEN));
            CountDownLatch expired=new CountDownLatch(1);AtomicInteger exchangeCount=new AtomicInteger();
            try(AccountBrowser browser=new AccountBrowser(f.origin,10)){browser.start(new AccountBrowser.Completion(){public void exchange(String code,String verifier){exchangeCount.incrementAndGet();}public void failed(String message){expired.countDown();}});check(expired.await(2,TimeUnit.SECONDS),"Browser pending timeout failed");equal(0,exchangeCount.get());}
            CountDownLatch trickleExpired=new CountDownLatch(1);
            try(AccountBrowser browser=new AccountBrowser(f.origin,150);Socket slow=new Socket("127.0.0.1",Integer.parseInt(query(browser.url()).get("port")))){
                browser.start(new AccountBrowser.Completion(){public void exchange(String code,String verifier){throw new AssertionError("Slow malformed request accepted");}public void failed(String message){trickleExpired.countDown();}});
                Thread trickle=new Thread(()->{try{for(int i=0;i<100;i++){slow.getOutputStream().write('G');slow.getOutputStream().flush();Thread.sleep(25);}}catch(Exception closed){}});trickle.setDaemon(true);trickle.start();
                check(trickleExpired.await(1,TimeUnit.SECONDS),"Slow byte trickle extended total browser deadline");
            }
            try(AccountBrowser browser=new AccountBrowser(f.origin,5000);Socket idle=new Socket("127.0.0.1",Integer.parseInt(query(browser.url()).get("port")))){
                browser.start(new AccountBrowser.Completion(){public void exchange(String code,String verifier){}public void failed(String message){}});idle.getOutputStream().write('G');Thread.sleep(30);browser.close();idle.setSoTimeout(300);equal(-1,idle.getInputStream().read());
            }
        }
        try(Fixture f=new Fixture(100)){
            f.blockLogin=true;long ticket=f.accounts.beginNative(()->{});CountDownLatch done=new CountDownLatch(1);AtomicReference<Exception> result=new AtomicReference<>();
            // The same worker is busy with HTTP, so its scheduled timer cannot fire first.
            f.accounts.submitNative(()->{try{f.accounts.password(ticket,"tablet@example.test","native-secret-password");}catch(Exception expected){result.set(expected);}finally{done.countDown();}});
            check(f.exchanged.await(2,TimeUnit.SECONDS),"Native timeout request not reached");Thread.sleep(150);f.continueExchange.countDown();check(done.await(2,TimeUnit.SECONDS),"Native deadline failed to terminate");check(result.get()!=null,"Late native response accepted despite blocked timer");equal(1,f.revokes.get());check(f.store.saved==null,"Expired native operation persisted token");check(!object(f.accounts.state()).getBoolean("native_pending"),"Expired native form remained pending");
        }
    }
    public static void main(String[] args)throws Exception{
        // Android allows Origin. The host JVM needs this flag for its restricted-header policy.
        System.setProperty("sun.net.http.allowRestrictedHeaders","true");
        protocol();browser();nativeFallback();persistenceAndCancellation();boundsAndTimeout();
        System.out.println("PASS: origin/callback validation, real HTTP PKCE state/replay, native MFA fallback, Android registration/proof/heartbeat, sanitized state, persistence restore/failure revocation, identity binding/migration, late cancel, expired auth, device removal, redirect/response bounds, slow-trickle browser deadline, idle callback cancellation, blocked-worker native deadline, cleanup failure recovery");
    }
}
