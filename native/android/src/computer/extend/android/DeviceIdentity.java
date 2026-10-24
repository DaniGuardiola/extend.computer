package computer.extend.android;

import android.content.Context;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.AtomicFile;
import java.io.File;
import java.io.FileOutputStream;
import java.io.FileInputStream;
import java.io.FileNotFoundException;
import java.io.IOException;
import java.security.KeyStore;
import java.security.SecureRandom;
import java.util.Arrays;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

/** App-private X25519 seed wrapped by a non-exportable Android Keystore key. */
final class DeviceIdentity {
    private static final String ALIAS = "computer.extend.identity.wrap.v1";
    private static final byte[] AAD = "extend.computer/android/identity/v1".getBytes(java.nio.charset.StandardCharsets.UTF_8);

    private static SecretKey key(boolean create) throws Exception {
        KeyStore store = KeyStore.getInstance("AndroidKeyStore");
        store.load(null);
        if (store.containsAlias(ALIAS)) return (SecretKey) store.getKey(ALIAS, null);
        if (!create) throw new IOException("Device identity wrapping key is missing; refusing replacement.");
        KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
        generator.init(new KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
            .setKeySize(256).setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setRandomizedEncryptionRequired(true).build());
        return generator.generateKey();
    }

    static synchronized byte[] load(Context context) throws Exception {
        // Neither the encrypted identity nor its trust state is restored onto a
        // different device through Android backup. The wrapping key is local.
        File root = new File(context.getNoBackupFilesDir(), "engine");
        if (!root.isDirectory() && !root.mkdirs()) throw new IOException("Device identity storage unavailable.");
        AtomicFile file = new AtomicFile(new File(root, "identity.v1"));
        byte[] record = null;
        try (FileInputStream input = file.openRead()) {
            byte[] bounded = new byte[62];
            int size = 0, count;
            while (size < bounded.length && (count = input.read(bounded, size, bounded.length - size)) != -1) size += count;
            if (size != 61) throw new IOException("Invalid device identity; refusing replacement.");
            record = Arrays.copyOf(bounded, size);
        } catch (FileNotFoundException absent) {
            if (file.getBaseFile().exists()) throw absent;
        }
        if (record != null) {
            if (record.length != 61 || record[0] != 1) throw new IOException("Invalid device identity; refusing replacement.");
            Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
            cipher.init(Cipher.DECRYPT_MODE, key(false), new GCMParameterSpec(128, record, 1, 12));
            cipher.updateAAD(AAD);
            byte[] seed = cipher.doFinal(record, 13, record.length - 13);
            if (seed.length != 32) { Arrays.fill(seed, (byte) 0); throw new IOException("Invalid device identity seed."); }
            return seed;
        }
        byte[] seed = new byte[32];
        new SecureRandom().nextBytes(seed);
        FileOutputStream output = null;
        boolean saved = false;
        try {
            Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
            cipher.init(Cipher.ENCRYPT_MODE, key(true));
            cipher.updateAAD(AAD);
            byte[] iv = cipher.getIV();
            if (iv.length != 12) throw new IOException("Unsupported identity encryption nonce.");
            byte[] encrypted = cipher.doFinal(seed);
            output = file.startWrite();
            output.write(1); output.write(iv); output.write(encrypted);
            file.finishWrite(output); output = null;
            saved = true;
            return seed;
        } finally {
            if (output != null) file.failWrite(output);
            if (!saved) Arrays.fill(seed, (byte) 0);
        }
    }
}
