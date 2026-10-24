package computer.extend.android;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.net.SocketTimeoutException;
import java.nio.charset.StandardCharsets;

/** Ephemeral IPv4 loopback callback, identical to the desktop PKCE contract. */
final class AccountBrowser implements AutoCloseable {
    interface Completion { void exchange(String code, String verifier) throws Exception; void failed(String message); }
    final String state = AccountProtocol.randomHex();
    private final String verifier = AccountProtocol.verifier();
    private final ServerSocket listener;
    private final String origin;
    private final long timeoutMillis;
    private volatile boolean closed;
    private volatile Socket activeClient;
    AccountBrowser(String origin) throws Exception { this(origin,300000); }
    AccountBrowser(String origin,long timeoutMillis) throws Exception {
        if(timeoutMillis<=0 || timeoutMillis>300000)throw new IllegalArgumentException("Invalid sign-in timeout.");
        this.timeoutMillis=timeoutMillis;
        this.origin=AccountProtocol.origin(origin);
        listener=new ServerSocket(0, 4, InetAddress.getByName("127.0.0.1")); listener.setSoTimeout(500);
    }
    String url() throws Exception { return origin+"/desktop/connect?port="+listener.getLocalPort()+"&state="+state+"&challenge="+AccountProtocol.challenge(verifier); }
    void start(Completion completion) {
        Thread worker=new Thread(() -> run(completion),"extend-account-browser"); worker.setDaemon(true); worker.start();
    }
    private void run(Completion completion) {
        long until=System.nanoTime()+timeoutMillis*1000000L;
        try {
            while(!closed && System.nanoTime()<until) {
                Socket socket;
                try { socket=listener.accept(); } catch(SocketTimeoutException retry) { continue; }
                activeClient=socket;
                try(Socket client=socket) {
                    if(closed)return;
                    String code;
                    try{code=AccountProtocol.callback(requestLine(client.getInputStream(),client,Math.min(until,System.nanoTime()+2000000000L)),state);}catch(Exception invalid){continue;}
                    if(code==null) { try{response(client,"400 Bad Request","Invalid sign-in callback.");}catch(Exception disconnected){} continue; }
                    listener.close();
                    try { completion.exchange(code,verifier); }
                    catch(Exception failed) {
                        try{response(client,"503 Service Unavailable","Couldn't finish sign-in. Return to the app and retry.");}catch(Exception disconnected){}
                        throw failed;
                    }
                    try {
                        String reply="HTTP/1.1 303 See Other\r\nLocation: "+origin+"/desktop/connected#close\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        client.getOutputStream().write(reply.getBytes(StandardCharsets.US_ASCII));
                    } catch(Exception disconnected) { /* The credential is already saved; closing this tab cannot undo success. */ }
                    return;
                } catch(SocketTimeoutException invalid) { /* An unrelated local socket cannot consume the sign-in. */ }
            }
            if(!closed)completion.failed("Browser sign-in timed out. Start again.");
        } catch(Exception failed) { if(!closed)completion.failed("Couldn't finish browser sign-in. Return to the app and retry."); }
        finally { close(); }
    }
    static String requestLine(InputStream input) throws Exception {
        return requestLine(input,null,System.nanoTime()+2000000000L);
    }
    private static String requestLine(InputStream input,Socket socket,long deadline) throws Exception {
        ByteArrayOutputStream bytes=new ByteArrayOutputStream();int value,ending=0;
        while(true){
            long remaining=deadline-System.nanoTime();if(remaining<=0)throw new SocketTimeoutException("Sign-in request timed out.");
            if(socket!=null)socket.setSoTimeout((int)Math.max(1,Math.min(2000,(remaining+999999)/1000000)));
            value=input.read();if(value==-1)return "";
            if(System.nanoTime()>deadline)throw new SocketTimeoutException("Sign-in request timed out.");
            if(bytes.size()>=8192)return "";
            bytes.write(value);
            ending=(ending<<8)|value;
            if(ending==0x0d0a0d0a){
                String headers=bytes.toString("US-ASCII");
                int first=headers.indexOf("\r\n");return first<0?"":headers.substring(0,first);
            }
        }
    }
    private void response(Socket client,String status,String text) throws Exception {
        byte[] body=text.getBytes(StandardCharsets.UTF_8);
        String headers="HTTP/1.1 "+status+"\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Length: "+body.length+"\r\nConnection: close\r\n\r\n";
        client.getOutputStream().write(headers.getBytes(StandardCharsets.US_ASCII)); client.getOutputStream().write(body);
    }
    public void close() { closed=true;try{listener.close();}catch(Exception ignored){}Socket client=activeClient;if(client!=null)try{client.close();}catch(Exception ignored){} }
}
