package computer.extend.android;

import android.content.Context;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.AtomicFile;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileNotFoundException;
import java.io.FileOutputStream;
import java.nio.charset.StandardCharsets;
import java.security.KeyStore;
import java.security.MessageDigest;
import java.util.Arrays;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;
import org.json.JSONObject;

/** Origin-bound, no-backup encrypted credentials. A damaged record is never replaced silently. */
final class AccountCredentials {
    private static final String ALIAS = "computer.extend.accounts.wrap.v1";
    private final File root;
    AccountCredentials(Context context) throws Exception {
        root = new File(context.getNoBackupFilesDir(), "accounts");
        if (!root.isDirectory() && !root.mkdirs()) throw new IllegalStateException("Account storage unavailable.");
    }
    private AtomicFile file(String origin) throws Exception {
        String name = AccountProtocol.hexBytes(MessageDigest.getInstance("SHA-256").digest(origin.getBytes(StandardCharsets.UTF_8)));
        return new AtomicFile(new File(root, name + ".v1"));
    }
    private SecretKey key(boolean create) throws Exception {
        KeyStore store = KeyStore.getInstance("AndroidKeyStore"); store.load(null);
        if (store.containsAlias(ALIAS)) return (SecretKey)store.getKey(ALIAS,null);
        if (!create) throw new IllegalStateException("Saved login key is missing. Sign out and try again.");
        KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES,"AndroidKeyStore");
        generator.init(new KeyGenParameterSpec.Builder(ALIAS,KeyProperties.PURPOSE_ENCRYPT|KeyProperties.PURPOSE_DECRYPT)
            .setKeySize(256).setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setRandomizedEncryptionRequired(true).build());
        return generator.generateKey();
    }
    private byte[] aad(String origin) { return ("extend.computer/android/account/v1/" + origin).getBytes(StandardCharsets.UTF_8); }
    synchronized JSONObject load(String origin) throws Exception {
        AtomicFile file = file(origin); byte[] record;
        try (FileInputStream input = file.openRead()) {
            byte[] bounded = new byte[16385]; int size=0,count;
            while (size<bounded.length && (count=input.read(bounded,size,bounded.length-size))!=-1) size+=count;
            if (size<30 || size>16384 || bounded[0]!=1) throw new IllegalStateException("Saved login is damaged. Sign out and try again.");
            record=Arrays.copyOf(bounded,size);
        } catch (FileNotFoundException absent) { if (file.getBaseFile().exists()) throw absent; return null; }
        Cipher cipher=Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.DECRYPT_MODE,key(false),new GCMParameterSpec(128,record,1,12)); cipher.updateAAD(aad(origin));
        byte[] plain=cipher.doFinal(record,13,record.length-13);
        try { return new JSONObject(new String(plain,StandardCharsets.UTF_8)); }
        finally { Arrays.fill(plain,(byte)0); }
    }
    synchronized void save(String origin, JSONObject credential) throws Exception {
        byte[] plain=credential.toString().getBytes(StandardCharsets.UTF_8); FileOutputStream output=null; AtomicFile file=file(origin);
        try {
            if (plain.length>16000) throw new IllegalStateException("Account credential is too large.");
            Cipher cipher=Cipher.getInstance("AES/GCM/NoPadding"); cipher.init(Cipher.ENCRYPT_MODE,key(!file.getBaseFile().exists())); cipher.updateAAD(aad(origin));
            byte[] iv=cipher.getIV(); if (iv.length!=12) throw new IllegalStateException("Account encryption unavailable.");
            byte[] encrypted=cipher.doFinal(plain); output=file.startWrite(); output.write(1); output.write(iv); output.write(encrypted);
            file.finishWrite(output); output=null;
        } finally { Arrays.fill(plain,(byte)0); if(output!=null)file.failWrite(output); }
    }
    synchronized void clear(String origin) throws Exception {
        AtomicFile file=file(origin); file.delete();
        if(file.getBaseFile().exists())throw new IllegalStateException("Couldn't remove saved login.");
    }
}
