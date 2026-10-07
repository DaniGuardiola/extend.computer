package computer.extend.benchmarkcamera;

import android.app.Activity;
import android.os.*;
import android.view.*;
import android.widget.*;
import android.hardware.camera2.*;
import android.hardware.camera2.params.StreamConfigurationMap;
import android.media.MediaRecorder;
import android.util.Range;
import android.util.Size;
import org.json.*;
import java.io.*;
import java.util.*;

/** Foreground high-speed capture controlled by explicit ADB activity arguments. */
public final class CaptureActivity extends Activity {
    final Handler main = new Handler(Looper.getMainLooper());
    HandlerThread thread;
    Handler cameraHandler;
    CameraDevice camera;
    CameraConstrainedHighSpeedCaptureSession session;
    MediaRecorder recorder;
    SurfaceView preview;
    TextView status;
    File directory;
    BufferedWriter samples;
    volatile boolean recording = false;
    boolean finished = false;
    int fps, seconds, width, height, orientation;
    long startNs, stopNs, sensorCount = 0, firstSensor = 0, lastSensor = 0;
    String cameraId;
    JSONObject report = new JSONObject();

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        FrameLayout root = new FrameLayout(this);
        preview = new SurfaceView(this);
        root.addView(preview, new FrameLayout.LayoutParams(-1, -1));
        status = new TextView(this);
        status.setTextColor(0xffffffff);
        status.setBackgroundColor(0x99000000);
        status.setTextSize(20);
        root.addView(status, new FrameLayout.LayoutParams(-1, -2));
        setContentView(root);
        try {
            String run = getIntent().getStringExtra("run_id");
            if (run == null || !run.matches("[a-zA-Z0-9-]{1,80}")) throw new Exception("Invalid run ID");
            directory = new File(getFilesDir(), run);
            if (!directory.mkdir()) throw new Exception("Run directory already exists");
            fps = getIntent().getIntExtra("fps", 240);
            seconds = getIntent().getIntExtra("seconds", 5);
            width = getIntent().getIntExtra("width", 1920);
            height = getIntent().getIntExtra("height", 1080);
            if (seconds < 1 || seconds > 60 || fps < 120 || fps > 480) throw new Exception("Invalid capture bounds");
            report.put("schema_version", 1).put("run_id", run);
            report.put("requested_capture_fps", fps).put("requested_seconds", seconds);
            CameraManager manager = (CameraManager)getSystemService(CAMERA_SERVICE);
            JSONArray capabilities = new JSONArray();
            CameraCharacteristics selected = null;
            for (String id : manager.getCameraIdList()) {
                CameraCharacteristics c = manager.getCameraCharacteristics(id);
                Integer facing = c.get(CameraCharacteristics.LENS_FACING);
                if (facing == null || facing != CameraCharacteristics.LENS_FACING_BACK) continue;
                StreamConfigurationMap map = c.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP);
                if (map == null) continue;
                for (Size size : map.getHighSpeedVideoSizes()) {
                    for (Range<Integer> range : map.getHighSpeedVideoFpsRangesFor(size)) {
                        capabilities.put(new JSONObject().put("camera", id).put("width", size.getWidth()).put("height", size.getHeight()).put("min_fps", range.getLower()).put("max_fps", range.getUpper()));
                        if (selected == null && size.equals(new Size(width,height)) && range.equals(new Range<Integer>(fps,fps))) {
                            selected = c; cameraId = id;
                        }
                    }
                }
            }
            report.put("capabilities", capabilities);
            if ("probe".equals(getIntent().getStringExtra("operation"))) { finishReport("probe_complete", null); return; }
            if (selected == null) throw new Exception("Requested fixed high-speed mode not supported");
            report.put("camera", cameraId).put("width", width).put("height", height);
            report.put("sensor_timestamp_source", selected.get(CameraCharacteristics.SENSOR_INFO_TIMESTAMP_SOURCE));
            int[] rotations = {0,90,180,270};
            orientation = (selected.get(CameraCharacteristics.SENSOR_ORIENTATION) - rotations[getWindowManager().getDefaultDisplay().getRotation()] + 360) % 360;
            report.put("orientation_hint", orientation);
            preview.setRotation(orientation);
            thread = new HandlerThread("benchmark-camera"); thread.start();
            cameraHandler = new Handler(thread.getLooper());
            preview.getHolder().setFixedSize(width,height);
            preview.getHolder().addCallback(new SurfaceHolder.Callback() {
                public void surfaceCreated(SurfaceHolder holder) { cameraHandler.post(() -> open(manager)); }
                public void surfaceChanged(SurfaceHolder h, int f, int w, int height) {}
                public void surfaceDestroyed(SurfaceHolder h) { if (!finished) fail("Preview surface lost"); }
            });
            status.setText("Preparing " + width + "×" + height + " at " + fps + " FPS");
            main.postDelayed(() -> { if (!recording && !finished) fail("Camera startup timeout"); }, 20000);
        } catch (Exception error) { fail(error.getMessage()); }
    }

    void open(CameraManager manager) {
        try {
            samples = new BufferedWriter(new FileWriter(new File(directory,"sensor.jsonl")));
            recorder = new MediaRecorder();
            recorder.setVideoSource(MediaRecorder.VideoSource.SURFACE);
            recorder.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4);
            recorder.setOutputFile(new File(directory,"video.mp4").getAbsolutePath());
            recorder.setVideoEncodingBitRate(12000000);
            recorder.setVideoEncoder(MediaRecorder.VideoEncoder.H264);
            recorder.setVideoSize(width,height);
            // Keep all high-speed samples; playback is deliberately 8x slower.
            recorder.setVideoFrameRate(30);
            recorder.setCaptureRate(fps);
            recorder.setOrientationHint(orientation);
            recorder.prepare();
            manager.openCamera(cameraId, new CameraDevice.StateCallback() {
                public void onOpened(CameraDevice c) { camera=c; configure(); }
                public void onDisconnected(CameraDevice c) { c.close(); fail("Camera disconnected"); }
                public void onError(CameraDevice c,int error) { c.close(); fail("Camera error " + error); }
            }, cameraHandler);
        } catch (Exception error) { fail(error.getClass().getSimpleName() + ": " + error.getMessage()); }
    }

    void configure() {
        try {
            List<Surface> surfaces = Arrays.asList(preview.getHolder().getSurface(),recorder.getSurface());
            camera.createConstrainedHighSpeedCaptureSession(surfaces,new CameraCaptureSession.StateCallback() {
                public void onConfigured(CameraCaptureSession s) {
                    try {
                        session=(CameraConstrainedHighSpeedCaptureSession)s;
                        CaptureRequest.Builder request=camera.createCaptureRequest(CameraDevice.TEMPLATE_RECORD);
                        for (Surface surface:surfaces) request.addTarget(surface);
                        request.set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE,new Range<Integer>(fps,fps));
                        request.set(CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE,CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE_OFF);
                        request.set(CaptureRequest.LENS_OPTICAL_STABILIZATION_MODE,CaptureRequest.LENS_OPTICAL_STABILIZATION_MODE_OFF);
                        session.setRepeatingBurst(session.createHighSpeedRequestList(request.build()),new CameraCaptureSession.CaptureCallback() {
                            @Override public void onCaptureCompleted(CameraCaptureSession s,CaptureRequest r,TotalCaptureResult result) {
                                if (!recording) return;
                                try {
                                    Long ts=result.get(CaptureResult.SENSOR_TIMESTAMP);
                                    Long exposure=result.get(CaptureResult.SENSOR_EXPOSURE_TIME);
                                    Long skew=result.get(CaptureResult.SENSOR_ROLLING_SHUTTER_SKEW);
                                    if (ts==null) return;
                                    sensorCount++;
                                    if(firstSensor==0) {
                                        firstSensor=ts;
                                        try(FileWriter ready=new FileWriter(new File(directory,"ready.json"))) {
                                            ready.write(new JSONObject().put("status","recording").put("sensor_ns",ts).put("capture_fps",fps).toString());
                                        }
                                    }
                                    lastSensor=ts;
                                    samples.write(new JSONObject().put("sensor_ns",ts).put("frame_number",result.getFrameNumber()).put("exposure_ns",exposure).put("rolling_shutter_skew_ns",skew).put("focus_distance_diopters",result.get(CaptureResult.LENS_FOCUS_DISTANCE)).put("optical_stabilization_mode",result.get(CaptureResult.LENS_OPTICAL_STABILIZATION_MODE)).put("video_stabilization_mode",result.get(CaptureResult.CONTROL_VIDEO_STABILIZATION_MODE)).toString()+"\n");
                                } catch(Exception error) { fail("Sensor telemetry write failed"); }
                            }
                        },cameraHandler);
                        recorder.start();startNs=System.nanoTime();recording=true;
                        main.post(() -> status.setText("Recording " + fps + " FPS for " + seconds + " seconds"));
                        cameraHandler.postDelayed(() -> stopRecording("complete"),seconds*1000L);
                    } catch(Exception error) { fail(error.getClass().getSimpleName()+": "+error.getMessage()); }
                }
                public void onConfigureFailed(CameraCaptureSession s) { fail("High-speed session configuration failed"); }
            },cameraHandler);
        } catch(Exception error) { fail(error.getMessage()); }
    }

    void stopRecording(String outcome) {
        if(finished)return;
        try {
            recording=false;stopNs=System.nanoTime();
            if(session!=null)session.stopRepeating();
            recorder.stop();
            report.put("recording_elapsed_s",(stopNs-startNs)/1e9);
            report.put("sensor_callback_count",sensorCount).put("first_sensor_ns",firstSensor).put("last_sensor_ns",lastSensor);
            report.put("sensor_callback_hz",sensorCount>1 && lastSensor>firstSensor ? (sensorCount-1)*1e9/(lastSensor-firstSensor) : 0);
            finishReport(outcome,null);
        } catch(Exception error) { fail("Recording finalization failed: "+error.getMessage()); }
    }

    void fail(String message) {
        if(cameraHandler!=null && Looper.myLooper()!=cameraHandler.getLooper())cameraHandler.post(() -> finishReport("failed",message));
        else finishReport("failed",message);
    }
    synchronized void finishReport(String outcome,String error) {
        if(finished)return;finished=true;recording=false;
        try { if(session!=null)session.close();if(camera!=null)camera.close();if(recorder!=null)recorder.release();if(samples!=null)samples.close(); } catch(Exception ignored) {}
        try {
            report.put("status",outcome);if(error!=null)report.put("error",error);
            if(directory!=null && directory.isDirectory()) {
                File file=new File(directory,"result.json");
                if(!file.exists())try(FileWriter out=new FileWriter(file)){out.write(report.toString(2));}
            }
        } catch(Exception ignored) {}
        main.post(() -> status.setText(outcome+(error==null?"":": "+error)));
    }
    @Override public void onPause() {
        super.onPause();
        if(recording && cameraHandler!=null)cameraHandler.post(() -> stopRecording("interrupted"));
    }
    @Override public void onDestroy() {
        if(!finished)fail("Activity destroyed");
        if(thread!=null)thread.quitSafely();
        super.onDestroy();
    }
}
