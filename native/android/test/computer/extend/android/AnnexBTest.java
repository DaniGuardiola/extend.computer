package computer.extend.android;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.util.Arrays;

/** Pure-Java codec-header cases, runnable without a device or Android runtime. */
public final class AnnexBTest {
    private static byte[] concat(byte[]... chunks) throws IOException {
        ByteArrayOutputStream output = new ByteArrayOutputStream();
        for (byte[] chunk : chunks) output.write(chunk);
        return output.toByteArray();
    }
    private static void rejects(byte[] bytes, int offset, boolean hevc) throws Exception {
        try { AnnexB.parameters(bytes, offset, hevc); }
        catch (IOException expected) { return; }
        throw new AssertionError("Invalid stream was accepted");
    }
    public static void main(String[] args) throws Exception {
        byte[] sps = {0, 0, 0, 1, 0x67, 9, 0, 0, 3, 1};
        byte[] pps = {0, 0, 1, 0x68, 7};
        byte[] picture = {0, 0, 0, 1, 0x65, 1, 2};
        byte[][] avc = AnnexB.parameters(concat(new byte[9], sps, pps, picture), 9, false);
        if (!Arrays.equals(avc[0], sps) || !Arrays.equals(avc[1], pps)) throw new AssertionError("AVC parameters changed");
        byte[] vps = {0, 0, 1, 64, 1, 5};
        byte[] hevcSps = {0, 0, 0, 1, 66, 1, 6};
        byte[] hevcPps = {0, 0, 1, 68, 1, 7};
        byte[][] hevc = AnnexB.parameters(concat(vps, hevcSps, hevcPps), 0, true);
        if (!Arrays.equals(hevc[0], vps) || !Arrays.equals(hevc[1], hevcSps)
                || !Arrays.equals(hevc[2], hevcPps)) throw new AssertionError("HEVC parameters changed");
        rejects(sps, 0, false);
        rejects(concat(vps, hevcSps), 0, true);
        rejects(new byte[] {0, 0, 1}, 0, false);
        rejects(new byte[] {7, 8, 9}, 0, true);
        rejects(sps, -1, false);
        rejects(sps, sps.length, false);
        System.out.println("Annex B header validation passed.");
    }
}
