package computer.extend.android.displayprobe;

import android.app.Activity;
import android.app.ActivityManager;
import android.app.ActivityOptions;
import android.app.Instrumentation;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.graphics.Bitmap;
import android.graphics.PixelFormat;
import android.hardware.display.DisplayManager;
import android.hardware.display.VirtualDisplay;
import android.media.Image;
import android.media.ImageReader;
import android.os.Bundle;
import android.os.Process;
import android.os.SystemClock;
import java.io.File;
import java.io.FileOutputStream;
import org.json.JSONObject;

/** No shell permission adoption, projection token, or privileged display flags. */
public final class ProbeInstrumentation extends Instrumentation {
    @Override public void onCreate(Bundle arguments) { super.onCreate(arguments); start(); }
    @Override public void onStart() {
        Bundle result = new Bundle();
        VirtualDisplay display = null;
        ImageReader reader = null;
        Activity owner = null;
        JSONObject evidence = new JSONObject();
        try {
            Context context = getTargetContext();
            evidence.put("uid", Process.myUid());
            evidence.put("package", context.getPackageName());
            evidence.put("shell_permission_adoption", false);
            evidence.put("projection_token", false);
            evidence.put("secondary_activities_supported", context.getPackageManager().hasSystemFeature(PackageManager.FEATURE_ACTIVITIES_ON_SECONDARY_DISPLAYS));
            Intent foreground = new Intent(context, ProbeActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            owner = startActivitySync(foreground);
            reader = ImageReader.newInstance(1280, 800, PixelFormat.RGBA_8888, 3);
            DisplayManager manager = (DisplayManager) context.getSystemService(Context.DISPLAY_SERVICE);
            int flags = DisplayManager.VIRTUAL_DISPLAY_FLAG_PUBLIC | DisplayManager.VIRTUAL_DISPLAY_FLAG_OWN_CONTENT_ONLY;
            evidence.put("flags", flags);
            display = manager.createVirtualDisplay("Extend extension capability probe", 1280, 800, 240, reader.getSurface(), flags);
            if (display == null) throw new IllegalStateException("Virtual display creation returned null");
            int id = display.getDisplay().getDisplayId();
            evidence.put("display_id", id);
            evidence.put("virtual_display_created", true);
            Intent target = new Intent().setClassName("computer.extend.android.enginetest", "computer.extend.android.tests.ReceiverTargetActivity")
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_MULTIPLE_TASK);
            evidence.put("target_package", target.getComponent().getPackageName());
            evidence.put("target_uid", context.getPackageManager().getApplicationInfo(target.getComponent().getPackageName(), 0).uid);
            ActivityManager activities = (ActivityManager) context.getSystemService(Context.ACTIVITY_SERVICE);
            boolean allowed = activities.isActivityStartAllowedOnDisplay(context, id, target);
            evidence.put("cross_uid_activity_launch_allowed", allowed);
            if (!allowed) throw new IllegalStateException("Activity launch on public display was rejected");
            ActivityOptions options = ActivityOptions.makeBasic().setLaunchDisplayId(id);
            context.startActivity(target, options.toBundle());
            long deadline = SystemClock.uptimeMillis() + 12000;
            Bitmap captured = null;
            long nonBlack = 0;
            while (SystemClock.uptimeMillis() < deadline) {
                Image image = reader.acquireLatestImage();
                if (image != null) {
                    try {
                        Image.Plane plane = image.getPlanes()[0];
                        int pixelStride = plane.getPixelStride(), rowStride = plane.getRowStride();
                        if (pixelStride != 4 || rowStride < 1280 * pixelStride) throw new IllegalStateException("Unexpected image layout");
                        Bitmap padded = Bitmap.createBitmap(rowStride / pixelStride, 800, Bitmap.Config.ARGB_8888);
                        padded.copyPixelsFromBuffer(plane.getBuffer());
                        Bitmap next = Bitmap.createBitmap(padded, 0, 0, 1280, 800);
                        if (next != padded) padded.recycle();
                        int[] pixels = new int[1280 * 800]; next.getPixels(pixels, 0, 1280, 0, 0, 1280, 800);
                        long count = 0;
                        for (int pixel : pixels) if ((pixel & 0x00ffffff) != 0) count++;
                        if (captured != null) captured.recycle();
                        captured = next; nonBlack = count;
                        if (count > pixels.length / 10) break;
                    } finally { image.close(); }
                }
                SystemClock.sleep(100);
            }
            evidence.put("captured_nonblack_pixels", nonBlack);
            if (captured == null || nonBlack < 1280 * 800 / 10) throw new IllegalStateException("No secondary activity pixels reached the app-owned surface");
            File screenshot = new File(context.getExternalFilesDir(null), "extension-probe.png");
            try (FileOutputStream output = new FileOutputStream(screenshot)) { captured.compress(Bitmap.CompressFormat.PNG, 100, output); }
            captured.recycle();
            evidence.put("screenshot", screenshot.getAbsolutePath());
            evidence.put("status", "passed");
            result.putString("status", "passed");
        } catch (Throwable error) {
            try { evidence.put("status", "failed"); evidence.put("error", error.getClass().getName() + ": " + error.getMessage()); }
            catch (Exception ignored) {}
            result.putString("status", "failed");
        } finally {
            if (display != null) display.release();
            if (reader != null) reader.close();
            final Activity activity = owner;
            if (activity != null) runOnMainSync(() -> activity.finish());
            result.putString("evidence", evidence.toString());
            finish("passed".equals(result.getString("status")) ? Activity.RESULT_OK : Activity.RESULT_CANCELED, result);
        }
    }
}
