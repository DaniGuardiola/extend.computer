package computer.extend.android;

import java.util.Arrays;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import org.json.JSONObject;

/** Exercise the production route-save transaction and the shared check/unpair monitor. */
public final class PeerAddressTest {
    private static final String PEER = "1".repeat(64);
    private static void check(boolean value, String message) { if (!value) throw new AssertionError(message); }
    private static void equal(Object expected, Object actual) { check(expected.equals(actual), expected + " != " + actual); }
    interface Task { void run() throws Exception; }
    private static void rejects(Task task) throws Exception {
        try { task.run(); } catch (IllegalArgumentException | IllegalStateException expected) { return; }
        throw new AssertionError("Invalid or stale route was accepted");
    }
    private static final class Store implements PeerAddress.Store {
        String metadata = new JSONObject().put("name", "Authenticated Mac").put("address", "192.0.2.1:48177")
            .put("peer", PEER).put("label", "My computer").put("trustSource", "account")
            .put("extra", new JSONObject().put("preserve", true)).toString();
        boolean paired = true, writable = true;
        int checks, reads, writes;
        Task onRead = () -> {};
        Task onPaired = () -> {};
        final List<String> events = new ArrayList<>();
        @Override public boolean paired(String peer) throws Exception {
            check(Thread.holdsLock(NativeEngine.class), "Pairing check outside shared monitor");
            equal(PEER, peer); checks++; onPaired.run(); events.add("paired"); return paired;
        }
        @Override public String read(String peer) throws Exception {
            check(Thread.holdsLock(NativeEngine.class), "Metadata read outside shared monitor");
            equal(PEER, peer); reads++; onRead.run(); events.add("read"); return metadata;
        }
        @Override public boolean commit(String peer, String value) {
            check(Thread.holdsLock(NativeEngine.class), "Metadata commit outside shared monitor");
            equal(PEER, peer); writes++; events.add("commit");
            if (writable) metadata = value;
            return writable;
        }
    }
    private static void numericAddresses() throws Exception {
        equal("192.0.2.3:48177", PeerAddress.normalize(" 192.0.2.3:48177 "));
        equal("127.0.0.1:1", PeerAddress.normalize("127.0.0.1:00001"));
        equal("255.255.255.255:65535", PeerAddress.normalize("255.255.255.255:65535"));
        equal("[2001:db8::a]:48177", PeerAddress.normalize("[2001:DB8::A]:48177"));
        equal("[fe80::1%3]:48177", PeerAddress.normalize("[fe80::1%3]:48177"));
        equal("[fe80::1%4294967295]:48177", PeerAddress.normalize("[fe80::1%4294967295]:48177"));
        equal("[::ffff:192.0.2.4]:48177", PeerAddress.normalize("[::ffff:192.0.2.4]:48177"));
        for (String bad : Arrays.asList("", " ", "mac.local:48177", "localhost:48177", "127.1:48177", "01.2.3.4:48177",
                "256.2.3.4:48177", "192.0.2.1:0", "192.0.2.1:65536", "192.0.2.1:-1", "192.0.2.1:123456",
                "192.0.2.1:1/", "192.0.2.1:1?x", "192.0.2.1:1#x", "user@192.0.2.1:1", "http://192.0.2.1:1",
                "0.0.0.0:1", "224.0.0.1:1", "::1:48177", "[::]:1", "[::%3]:1", "[0:0:0:0:0:0:0:0]:1", "[ff02::1]:1",
                "[1:2:3:4:5:6:7]:1", "[1:2:3:4:5:6:7:8:9]:1", "[1::2::3]:1", "[:::1]:1", "[::g]:1",
                "[::ffff:192.0.2.001]:1", "[fe80::1%wlan0]:1", "[fe80::1%4294967296]:1", "[::1]:1]",
                "192.0.2.1:1\n", "192.0.2.1:１", "[::1] :1")) {
            Store store = new Store(); String before = store.metadata;
            try { rejects(() -> PeerAddress.save(PEER, bad, store)); }
            catch (AssertionError accepted) { throw new AssertionError("Accepted invalid endpoint: " + bad, accepted); }
            equal(before, store.metadata); equal(0, store.checks); equal(0, store.writes);
        }
        rejects(() -> PeerAddress.normalize(null));
    }
    private static void saveAndStaleIdentity() throws Exception {
        Store store = new Store(); JSONObject before = new JSONObject(store.metadata);
        PeerAddress.save(PEER, "192.0.2.200:55555", store); // No reachability requirement or network callback.
        JSONObject after = new JSONObject(store.metadata);
        equal("192.0.2.200:55555", after.getString("address"));
        before.remove("address"); after.remove("address");
        check(before.similar(after), "Address Save changed identity, name, label or other metadata");
        equal(Arrays.asList("paired", "read", "paired", "commit"), store.events);
        Store unknown = new Store(); unknown.paired = false;
        rejects(() -> PeerAddress.save(PEER, "192.0.2.200:55555", unknown)); equal(0, unknown.reads); equal(0, unknown.writes);
        Store retired = new Store(); retired.onPaired = () -> { if (retired.checks == 2) retired.paired = false; };
        String saved = retired.metadata;
        rejects(() -> PeerAddress.save(PEER, "192.0.2.200:55555", retired)); equal(saved, retired.metadata); equal(0, retired.writes);
        Store failed = new Store(); failed.writable = false;
        rejects(() -> PeerAddress.save(PEER, "192.0.2.200:55555", failed)); equal(1, failed.writes);
        Store fresh = new Store(); fresh.metadata = null;
        PeerAddress.save(PEER, "[2001:db8::2]:48177", fresh);
        equal(1, new JSONObject(fresh.metadata).length());
        String unchanged = fresh.metadata;
        JSONObject account = PeerAddress.metadata(fresh.metadata, "Account computer");
        JSONObject accountLabel = DeviceSettings.decorate(account, "Account computer", "My Mac");
        equal("My Mac", accountLabel.getString("name"));
        equal("Account computer", accountLabel.getString("defaultName"));
        equal("Account computer", accountLabel.getString("authenticatedName"));
        equal("[2001:db8::2]:48177", accountLabel.getString("address"));
        equal(unchanged, fresh.metadata); // Account names remain live metadata, not a saved override.
        equal("", PeerAddress.metadata(null, "Account computer").getString("address"));
        equal("Authenticated Mac", PeerAddress.metadata(store.metadata, "Account computer").getString("name"));
        rejects(() -> PeerAddress.save("invalid-peer", "192.0.2.1:1", fresh));
    }
    private static void pinnedRepair() throws Exception {
        Store failed = new Store(); String saved = failed.metadata;
        try {
            PeerAddress.verifyAndSave(PEER, "192.0.2.200:55555", failed, (address, peer) -> {
                equal(PEER, peer); equal("192.0.2.200:55555", address);
                throw new Exception("Pinned identity mismatch");
            });
            throw new AssertionError("Failed pinned probe repaired route");
        } catch (Exception expected) { equal("Pinned identity mismatch", expected.getMessage()); }
        equal(saved, failed.metadata); equal(0, failed.writes);
        Store verified = new Store();
        JSONObject result = PeerAddress.verifyAndSave(PEER, "192.0.2.200:55555", verified, (address, peer) -> {
            check(Thread.holdsLock(NativeEngine.class), "Pinned probe outside check/unpair monitor");
            equal(PEER, peer); verified.events.add("probe"); return "{\"rtt_ms\":3}";
        });
        equal(3, result.getInt("rtt_ms"));
        equal(Arrays.asList("probe", "paired", "read", "paired", "commit"), verified.events);
        Store revoked = new Store();
        rejects(() -> PeerAddress.verifyAndSave(PEER, "192.0.2.200:55555", revoked, (address, peer) -> {
            revoked.paired = false; return "{\"rtt_ms\":3}";
        }));
        equal(0, revoked.writes);
    }
    private static void sharedUnpairOrdering() throws Exception {
        Store store = new Store(); CountDownLatch read = new CountDownLatch(1), proceed = new CountDownLatch(1), forgetting = new CountDownLatch(1);
        AtomicReference<Throwable> failed = new AtomicReference<>();
        store.onRead = () -> { read.countDown(); check(proceed.await(2, TimeUnit.SECONDS), "Save barrier timed out"); };
        Thread save = new Thread(() -> { try { PeerAddress.save(PEER, "192.0.2.200:55555", store); } catch (Throwable error) { failed.set(error); } });
        Thread forget = new Thread(() -> {
            forgetting.countDown();
            synchronized (NativeEngine.class) { store.paired = false; store.metadata = null; }
        });
        save.start(); check(read.await(2, TimeUnit.SECONDS), "Save did not enter transaction");
        forget.start(); check(forgetting.await(2, TimeUnit.SECONDS), "Unpair did not start");
        long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(2);
        while (forget.getState() != Thread.State.BLOCKED && System.nanoTime() < deadline) Thread.onSpinWait();
        check(forget.getState() == Thread.State.BLOCKED, "Unpair bypassed address-save monitor");
        proceed.countDown(); save.join(2000); forget.join(2000);
        check(!save.isAlive() && !forget.isAlive() && failed.get() == null, "Concurrent transaction failed");
        check(!store.paired && store.metadata == null, "Save recreated metadata after Unpair");
        int writes = store.writes;
        rejects(() -> PeerAddress.save(PEER, "192.0.2.201:55555", store)); equal(writes, store.writes);
    }
    public static void main(String[] args) throws Exception {
        numericAddresses(); saveAndStaleIdentity(); pinnedRepair(); sharedUnpairOrdering();
        DeviceAvailabilityTest.main(args);
        System.out.println("PASS: numeric offline Save, rejected-route immutability, metadata preservation, paired recheck, pinned repair and shared Unpair ordering");
    }
}
