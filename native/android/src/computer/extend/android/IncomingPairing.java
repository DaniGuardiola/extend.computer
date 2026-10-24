package computer.extend.android;

import android.content.Context;
import android.net.nsd.NsdManager;
import android.net.nsd.NsdServiceInfo;
import org.json.JSONObject;
import org.json.JSONArray;
import java.net.NetworkInterface;
import java.net.Inet4Address;
import java.net.InetAddress;
import java.util.Collections;

/** Foreground pairing advertisement. The code is never included in DNS-SD. */
public final class IncomingPairing {
    private NsdManager manager;
    private NsdManager.RegistrationListener advertisement;
    public synchronized String start(Context context, String name) throws Exception {
        stop();
        JSONObject identity = NativeEngine.identity(context);
        String state = NativeEngine.startIncoming(name);
        try {
            manager = (NsdManager) context.getSystemService(Context.NSD_SERVICE);
            NsdServiceInfo service = new NsdServiceInfo();
            service.setServiceName("extend-computer-" + identity.getString("fingerprint").substring(0, 12));
            service.setServiceType("_extendpair._tcp.");
            service.setPort(new JSONObject(state).getInt("port"));
            service.setAttribute("version", "1");
            service.setAttribute("capability", "pairing");
            service.setAttribute("name", name);
            service.setAttribute("discovery_id", identity.getString("fingerprint"));
            advertisement = new NsdManager.RegistrationListener() {
                public void onServiceRegistered(NsdServiceInfo info) {}
                public void onRegistrationFailed(NsdServiceInfo info, int error) {}
                public void onServiceUnregistered(NsdServiceInfo info) {}
                public void onUnregistrationFailed(NsdServiceInfo info, int error) {}
            };
            if (manager != null) manager.registerService(service, NsdManager.PROTOCOL_DNS_SD, advertisement);
        } catch (RuntimeException failed) { withdraw(); } // Manual IP pairing remains available.
        return withAddresses(state);
    }
    public synchronized String state() throws Exception {
        String snapshot = NativeEngine.incomingState();
        if (new JSONObject(snapshot).isNull("code")) withdraw();
        return withAddresses(snapshot);
    }
    private String withAddresses(String snapshot) throws Exception {
        JSONObject result = new JSONObject(snapshot);
        JSONArray addresses = new JSONArray();
        int port = result.getInt("port");
        if (port != 0) {
            for (NetworkInterface network : Collections.list(NetworkInterface.getNetworkInterfaces())) {
                if (!network.isUp() || network.isLoopback()) continue;
                for (InetAddress address : Collections.list(network.getInetAddresses())) {
                    if (address instanceof Inet4Address && !address.isLoopbackAddress() && !address.isLinkLocalAddress())
                        addresses.put(address.getHostAddress() + ":" + port);
                }
            }
        }
        return result.put("addresses", addresses).toString();
    }
    private void withdraw() {
        if (advertisement != null && manager != null) {
            try { manager.unregisterService(advertisement); } catch (RuntimeException ignored) {}
        }
        advertisement = null; manager = null;
    }
    public synchronized void stop() { withdraw(); NativeEngine.stopIncoming(); }
}
