package computer.extend.android;

import java.io.IOException;
import java.util.Arrays;

final class AnnexB {
    private static int prefix(byte[] data, int offset) {
        if (offset + 3 <= data.length && data[offset] == 0 && data[offset + 1] == 0) {
            if (data[offset + 2] == 1) return 3;
            if (offset + 4 <= data.length && data[offset + 2] == 0 && data[offset + 3] == 1) return 4;
        }
        return 0;
    }
    static byte[][] parameters(byte[] data, int offset, boolean hevc) throws IOException {
        if (offset < 0 || offset >= data.length) throw new IOException("Missing Annex B data.");
        byte[][] sets = new byte[hevc ? 3 : 2][];
        for (int start = offset; start < data.length;) {
            int prefix = prefix(data, start);
            if (prefix == 0 || start + prefix >= data.length) throw new IOException("Malformed Annex B data.");
            int end = start + prefix + 1;
            while (end < data.length && prefix(data, end) == 0) end++;
            int type = hevc ? (data[start + prefix] >> 1) & 63 : data[start + prefix] & 31;
            int first = hevc ? 32 : 7;
            if (type >= first && type < first + sets.length)
                sets[type - first] = Arrays.copyOfRange(data, start, end);
            start = end;
        }
        for (byte[] set : sets) if (set == null) throw new IOException("Video keyframe is missing parameter sets.");
        return sets;
    }
}
