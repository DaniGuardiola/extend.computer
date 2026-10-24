package computer.extend.benchmarkcamera;

import android.app.Activity;
import android.os.*;
import android.view.*;
import android.widget.*;
import android.hardware.camera2.*;
import android.hardware.camera2.params.StreamConfigurationMap;
import android.hardware.camera2.params.MeteringRectangle;
import android.graphics.Rect;
import android.media.MediaRecorder;
import android.util.Range;
import android.util.Size;
import org.json.*;
import java.io.*;
import java.util.*;
import java.security.MessageDigest;

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
    volatile boolean finished = false;
    int fps, seconds, width, height, orientation, exposureCompensation;
    long startNs, stopNs, sensorCount = 0, firstSensor = 0, lastSensor = 0;
    String cameraId;
    JSONObject report = new JSONObject();
    CaptureRequest.Builder captureRequest;
    CameraCaptureSession.CaptureCallback captureCallback;
    Surface recordingSurface;
    boolean focusTriggerSent = false, fixedFocus = false;
    long focusStableSince = 0, focusPreparationNs = 0;
    Float previousFocus = null, lockedFocus = null;
    MeteringRectangle[] focusRegions;
    Double focusLower = null, focusUpper = null;
    int rejectedFocusLocks = 0;
    JSONArray rejectedFocusDistances = new JSONArray();
    Rect requestedCrop;
    String requestedOrientation;
    int sensorOrientation, acquiredRotation;
    Rect acquiredWindow;
    boolean orientationAcquired = false;
    boolean previewOnly = false, previewReady = false;
    final OrientationGate orientationGate = new OrientationGate();

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        requestedOrientation = getIntent().getStringExtra("camera_orientation");
        if (requestedOrientation == null) requestedOrientation = "sensor-landscape";
        if (requestedOrientation.equals("reverse-landscape"))
            setRequestedOrientation(android.content.pm.ActivityInfo.SCREEN_ORIENTATION_REVERSE_LANDSCAPE);
        else if (requestedOrientation.equals("sensor-landscape"))
            setRequestedOrientation(android.content.pm.ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE);
        else { finish(); return; }
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
            String operation = getIntent().getStringExtra("operation");
            if (operation == null) operation = "record";
            if (!operation.equals("record") && !operation.equals("probe") && !operation.equals("preview"))
                throw new Exception("Unknown camera operation");
            previewOnly = operation.equals("preview");
            String run = getIntent().getStringExtra("run_id");
            if (run == null || !run.matches("[a-zA-Z0-9-]{1,80}")) throw new Exception("Invalid run ID");
            directory = new File(getFilesDir(), run);
            if (!directory.mkdir()) throw new Exception("Run directory already exists");
            fps = getIntent().getIntExtra("fps", 240);
            seconds = getIntent().getIntExtra("seconds", 5);
            width = getIntent().getIntExtra("width", 1920);
            height = getIntent().getIntExtra("height", 1080);
            exposureCompensation = getIntent().getIntExtra("exposure_compensation_steps", 0);
            if (seconds < 1 || seconds > 60 || fps < 120 || fps > 480) throw new Exception("Invalid capture bounds");
            report.put("schema_version", 1).put("run_id", run).put("operation", operation);
            report.put("requested_capture_fps", fps).put("requested_seconds", seconds);
            report.put("requested_exposure_compensation_steps", exposureCompensation);
            MessageDigest digest = MessageDigest.getInstance("SHA-256");
            try (InputStream apk = new FileInputStream(getApplicationInfo().sourceDir)) {
                byte[] buffer = new byte[8192]; int count;
                while ((count = apk.read(buffer)) != -1) digest.update(buffer, 0, count);
            }
            StringBuilder hash = new StringBuilder();
            for (byte value : digest.digest()) hash.append(String.format(Locale.ROOT, "%02x", value & 255));
            report.put("helper_apk_sha256", hash.toString());
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
            if (operation.equals("probe")) { finishReport("probe_complete", null); return; }
            if (selected == null) throw new Exception("Requested fixed high-speed mode not supported");
            Float minimumFocus = selected.get(CameraCharacteristics.LENS_INFO_MINIMUM_FOCUS_DISTANCE);
            fixedFocus = minimumFocus != null && minimumFocus == 0;
            report.put("focus_policy", "settle-and-lock-v1").put("fixed_focus_lens",fixedFocus);
            report.put("minimum_focus_distance_diopters",minimumFocus);
            String distanceText=getIntent().getStringExtra("focus_distance_range");
            if(distanceText!=null) {
                JSONArray range=new JSONArray(distanceText);
                if(range.length()!=2) throw new Exception("Invalid calibrated focus distance range");
                double low=range.getDouble(0),high=range.getDouble(1);
                if(!Double.isFinite(low)||!Double.isFinite(high)||low<0||low>=high||high>100||minimumFocus==null||(!fixedFocus && high>minimumFocus)||fixedFocus && low>0)
                    throw new Exception("Calibrated focus distance range unsupported by lens");
                focusLower=low;focusUpper=high;
                report.put("requested_focus_distance_range_diopters",range);
            }
            Range<Integer> compensationRange = selected.get(CameraCharacteristics.CONTROL_AE_COMPENSATION_RANGE);
            if (compensationRange == null || !compensationRange.contains(exposureCompensation))
                throw new Exception("Exposure compensation outside camera's supported range");
            report.put("exposure_compensation_range", new JSONArray().put(compensationRange.getLower()).put(compensationRange.getUpper()));
            report.put("exposure_compensation_step_ev", selected.get(CameraCharacteristics.CONTROL_AE_COMPENSATION_STEP).doubleValue());
            report.put("camera", cameraId).put("width", width).put("height", height);
            report.put("sensor_timestamp_source", selected.get(CameraCharacteristics.SENSOR_INFO_TIMESTAMP_SOURCE));
            float zoom=getIntent().getFloatExtra("zoom",1f);
            Float maxZoom=selected.get(CameraCharacteristics.SCALER_AVAILABLE_MAX_DIGITAL_ZOOM);
            Rect activeArray=selected.get(CameraCharacteristics.SENSOR_INFO_ACTIVE_ARRAY_SIZE);
            if(!Float.isFinite(zoom)||zoom<1||zoom>2||maxZoom==null||zoom>maxZoom||activeArray==null)
                throw new Exception("Requested camera zoom unsupported");
            report.put("requested_zoom",zoom);
            if(zoom>1) {
                int cw=(int)Math.floor(activeArray.width()/zoom),ch=(int)Math.floor(activeArray.height()/zoom);
                int left=activeArray.left+(activeArray.width()-cw)/2,top=activeArray.top+(activeArray.height()-ch)/2;
                requestedCrop=new Rect(left,top,left+cw,top+ch);
                report.put("requested_crop_sensor",rectBounds(requestedCrop));
            }
            sensorOrientation = selected.get(CameraCharacteristics.SENSOR_ORIENTATION);
            String regionText=getIntent().getStringExtra("focus_regions");
            if (regionText!=null) {
                JSONArray regions=new JSONArray(regionText);
                Integer maxRegions=selected.get(CameraCharacteristics.CONTROL_MAX_REGIONS_AF);
                if (maxRegions==null || maxRegions<regions.length() || regions.length()<1 || regions.length()>2)
                    throw new Exception("Requested AF regions unsupported by camera");
                Rect active=requestedCrop!=null ? requestedCrop : activeArray;
                if (active==null) throw new Exception("Camera active array missing");
                // Account for the centered 16:9 video crop within the sensor array.
                double aspect=(double)width/height;
                int cw=active.width(),ch=active.height();
                if ((double)cw/ch>aspect) cw=(int)Math.round(ch*aspect); else ch=(int)Math.round(cw/aspect);
                int left=active.left+(active.width()-cw)/2,top=active.top+(active.height()-ch)/2;
                focusRegions=new MeteringRectangle[regions.length()];
                JSONArray mapped=new JSONArray();
                for(int i=0;i<regions.length();i++) {
                    JSONArray region=regions.getJSONArray(i);
                    if(region.length()!=4) throw new Exception("Invalid AF rectangle");
                    double x0=region.getDouble(0),y0=region.getDouble(1),x1=region.getDouble(2),y1=region.getDouble(3);
                    if(!Double.isFinite(x0)||!Double.isFinite(y0)||!Double.isFinite(x1)||!Double.isFinite(y1)||x0<0||y0<0||x1>1||y1>1||x0>=x1||y0>=y1) throw new Exception("Invalid normalized AF rectangle");
                    Rect rectangle=new Rect(left+(int)(x0*cw),top+(int)(y0*ch),left+(int)(x1*cw),top+(int)(y1*ch));
                    focusRegions[i]=new MeteringRectangle(rectangle,500);
                    mapped.put(new JSONArray().put(rectangle.left).put(rectangle.top).put(rectangle.right).put(rectangle.bottom));
                }
                report.put("focus_regions_coordinate_basis", "sensor-output-normalized")
                    .put("requested_focus_regions_normalized",regions).put("requested_focus_regions_sensor",mapped);
            }
            thread = new HandlerThread("benchmark-camera"); thread.start();
            cameraHandler = new Handler(thread.getLooper());
            preview.getHolder().setFixedSize(width,height);
            preview.getHolder().addCallback(new SurfaceHolder.Callback() {
                public void surfaceCreated(SurfaceHolder holder) { acquireOrientation(manager); }
                public void surfaceChanged(SurfaceHolder h, int f, int w, int height) {}
                public void surfaceDestroyed(SurfaceHolder h) { if (!finished) fail("Preview surface lost"); }
            });
            status.setText("Preparing " + width + "×" + height + " at " + fps + " FPS");
            main.postDelayed(() -> { if (!recording && !previewReady && !finished) fail("Camera focus/startup timeout"); }, 30000);
        } catch (Exception error) { fail(error.getMessage()); }
    }

    // Sampling in onCreate can precede the requested landscape display transition.
    void acquireOrientation(CameraManager manager) {
        if (finished || orientationAcquired) return;
        View decor = getWindow().getDecorView();
        Rect bounds = new Rect(0, 0, decor.getWidth(), decor.getHeight());
        int rotation = getWindowManager().getDefaultDisplay().getRotation();
        if (!OrientationGate.matchesRequest(requestedOrientation, rotation)) {
            orientationGate.observe(rotation, 0, 0, SystemClock.uptimeMillis());
            main.postDelayed(() -> acquireOrientation(manager), 50); return;
        }
        if (preview.isAttachedToWindow() && preview.getHolder().getSurface().isValid()
                && orientationGate.observe(rotation, bounds.width(), bounds.height(), SystemClock.uptimeMillis())) {
            acquiredRotation = rotation; acquiredWindow = bounds;
            orientation = OrientationGate.hint(sensorOrientation, rotation);
            if (focusRegions != null && !OrientationGate.supportsFocusRegions(orientation)) {
                fail("Focus regions require 0 or 180 degree acquisition"); return;
            }
            try {
                report.put("orientation_policy", "attached-landscape-stable-v1")
                    .put("requested_orientation", requestedOrientation)
                    .put("sensor_orientation", sensorOrientation).put("display_rotation", rotation)
                    .put("window_bounds", rectBounds(bounds)).put("orientation_hint", orientation)
                    .put("orientation_stable_ms", OrientationGate.STABLE_MS);
            } catch (Exception error) { fail(error.getMessage()); return; }
            orientationAcquired = true;
            preview.setRotation(orientation);
            monitorOrientation();
            cameraHandler.post(() -> open(manager));
        } else main.postDelayed(() -> acquireOrientation(manager), 50);
    }

    void monitorOrientation() {
        if (finished) return;
        View decor = getWindow().getDecorView();
        if (getWindowManager().getDefaultDisplay().getRotation() != acquiredRotation
                || decor.getWidth() != acquiredWindow.width() || decor.getHeight() != acquiredWindow.height()) {
            fail("Camera display orientation/window changed after acquisition"); return;
        }
        main.postDelayed(this::monitorOrientation, 50);
    }

    void open(CameraManager manager) {
        try {
            if (!previewOnly) {
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
            }
            manager.openCamera(cameraId, new CameraDevice.StateCallback() {
                public void onOpened(CameraDevice c) { camera=c; configure(); }
                public void onDisconnected(CameraDevice c) { c.close(); fail("Camera disconnected"); }
                public void onError(CameraDevice c,int error) { c.close(); fail("Camera error " + error); }
            }, cameraHandler);
        } catch (Exception error) { fail(error.getClass().getSimpleName() + ": " + error.getMessage()); }
    }

    JSONArray rectBounds(Rect rect) {
        return rect==null ? null : new JSONArray().put(rect.left).put(rect.top).put(rect.right).put(rect.bottom);
    }
    JSONArray regionBounds(MeteringRectangle[] regions) throws Exception {
        if(regions==null) return null;
        JSONArray values=new JSONArray();
        for(MeteringRectangle region:regions) {
            Rect rect=region.getRect();
            values.put(new JSONArray().put(rect.left).put(rect.top).put(rect.right).put(rect.bottom));
        }
        return values;
    }

    void configure() {
        try {
            List<Surface> surfaces = previewOnly ? Arrays.asList(preview.getHolder().getSurface())
                : Arrays.asList(preview.getHolder().getSurface(),recorder.getSurface());
            camera.createConstrainedHighSpeedCaptureSession(surfaces,new CameraCaptureSession.StateCallback() {
                public void onConfigured(CameraCaptureSession s) {
                    try {
                        session=(CameraConstrainedHighSpeedCaptureSession)s;
                        CaptureRequest.Builder request=camera.createCaptureRequest(CameraDevice.TEMPLATE_RECORD);
                        captureRequest=request;recordingSurface=previewOnly ? null : recorder.getSurface();
                        // Preview first: no workload or measurement starts until autofocus locks.
                        request.addTarget(preview.getHolder().getSurface());
                        request.set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE,new Range<Integer>(fps,fps));
                        request.set(CaptureRequest.CONTROL_AE_EXPOSURE_COMPENSATION, exposureCompensation);
                        request.set(CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE,CaptureRequest.CONTROL_VIDEO_STABILIZATION_MODE_OFF);
                        request.set(CaptureRequest.LENS_OPTICAL_STABILIZATION_MODE,CaptureRequest.LENS_OPTICAL_STABILIZATION_MODE_OFF);
                        if(focusRegions!=null) request.set(CaptureRequest.CONTROL_AF_REGIONS,focusRegions);
                        if(requestedCrop!=null) request.set(CaptureRequest.SCALER_CROP_REGION,requestedCrop);
                        focusPreparationNs=System.nanoTime();
                        captureCallback=new CameraCaptureSession.CaptureCallback() {
                            @Override public void onCaptureCompleted(CameraCaptureSession s,CaptureRequest r,TotalCaptureResult result) {
                                try {
                                    if (previewOnly) {
                                        if (finished) return;
                                        if (!previewReady) {
                                            previewReady = true;
                                            report.put("preview_only", true).put("preview_started_ns", System.nanoTime());
                                            try (FileWriter ready = new FileWriter(new File(directory,"preview.json"))) {
                                                ready.write(new JSONObject(report.toString()).put("status","preview").toString(2));
                                            }
                                            main.post(() -> status.setText("Preview only · no recording · " + width + "×" + height + " at " + fps + " FPS"));
                                            cameraHandler.postDelayed(() -> finishReport("preview_complete", null), seconds * 1000L);
                                        }
                                        return;
                                    }
                                    Integer af=result.get(CaptureResult.CONTROL_AF_STATE);
                                    Integer lens=result.get(CaptureResult.LENS_STATE);
                                    Float distance=result.get(CaptureResult.LENS_FOCUS_DISTANCE);
                                    Rect crop=result.get(CaptureResult.SCALER_CROP_REGION);
                                    if(requestedCrop!=null && !requestedCrop.equals(crop)) {
                                        fail("Observed camera crop differs from requested crop");return;
                                    }
                                    if(fixedFocus && distance==null) distance=0f;
                                    if (!recording) {
                                        if (finished) return;
                                        if (fixedFocus) { beginLockedRecording(distance); return; }
                                        boolean stationary=lens!=null && lens==CaptureResult.LENS_STATE_STATIONARY;
                                        boolean focused=af!=null && af==(focusTriggerSent ? CaptureResult.CONTROL_AF_STATE_FOCUSED_LOCKED : CaptureResult.CONTROL_AF_STATE_PASSIVE_FOCUSED);
                                        boolean stable=distance!=null && Float.isFinite(distance) && previousFocus!=null && Math.abs(distance-previousFocus)<=0.01f;
                                        previousFocus=distance;
                                        if (!stationary || !focused || !stable) focusStableSince=0;
                                        else if (focusStableSince==0) focusStableSince=System.nanoTime();
                                        else if (System.nanoTime()-focusStableSince>=300000000L) {
                                            if (!focusTriggerSent) {
                                                focusTriggerSent=true;focusStableSince=0;
                                                request.set(CaptureRequest.CONTROL_AF_TRIGGER,CaptureRequest.CONTROL_AF_TRIGGER_START);
                                                session.captureBurst(session.createHighSpeedRequestList(request.build()),captureCallback,cameraHandler);
                                                request.set(CaptureRequest.CONTROL_AF_TRIGGER,CaptureRequest.CONTROL_AF_TRIGGER_IDLE);
                                            } else beginLockedRecording(distance);
                                        }
                                        return;
                                    }
                                    if (!fixedFocus && (af==null || af!=CaptureResult.CONTROL_AF_STATE_FOCUSED_LOCKED || lens==null || lens!=CaptureResult.LENS_STATE_STATIONARY || distance==null || lockedFocus==null || Math.abs(distance-lockedFocus)>0.02f)) {
                                        stopRecording("focus_lost");return;
                                    }
                                    Long ts=result.get(CaptureResult.SENSOR_TIMESTAMP);
                                    Long exposure=result.get(CaptureResult.SENSOR_EXPOSURE_TIME);
                                    Long skew=result.get(CaptureResult.SENSOR_ROLLING_SHUTTER_SKEW);
                                    if (ts==null) return;
                                    sensorCount++;
                                    if(firstSensor==0) {
                                        firstSensor=ts;
                                        try(FileWriter ready=new FileWriter(new File(directory,"ready.json"))) {
                                            ready.write(new JSONObject().put("status","recording").put("sensor_ns",ts).put("capture_fps",fps).put("focus_policy","settle-and-lock-v1").put("focus_distance_diopters",lockedFocus).put("af_state",af).toString());
                                        }
                                    }
                                    lastSensor=ts;
                                    samples.write(new JSONObject().put("sensor_ns",ts).put("frame_number",result.getFrameNumber()).put("exposure_ns",exposure).put("sensor_sensitivity_iso",result.get(CaptureResult.SENSOR_SENSITIVITY)).put("crop_sensor",rectBounds(crop)).put("exposure_compensation_steps",result.get(CaptureResult.CONTROL_AE_EXPOSURE_COMPENSATION)).put("ae_state",result.get(CaptureResult.CONTROL_AE_STATE)).put("rolling_shutter_skew_ns",skew).put("focus_distance_diopters",distance).put("af_regions_sensor",regionBounds(result.get(CaptureResult.CONTROL_AF_REGIONS))).put("af_mode",result.get(CaptureResult.CONTROL_AF_MODE)).put("af_state",af).put("lens_state",lens).put("optical_stabilization_mode",result.get(CaptureResult.LENS_OPTICAL_STABILIZATION_MODE)).put("video_stabilization_mode",result.get(CaptureResult.CONTROL_VIDEO_STABILIZATION_MODE)).toString()+"\n");
                                } catch(Exception error) { fail("Camera capture/telemetry failed: "+error.getMessage()); }
                            }
                        };
                        session.setRepeatingBurst(session.createHighSpeedRequestList(request.build()),captureCallback,cameraHandler);
                        main.post(() -> status.setText("Waiting for stable autofocus and focus lock"));
                    } catch(Exception error) { fail(error.getClass().getSimpleName()+": "+error.getMessage()); }
                }
                public void onConfigureFailed(CameraCaptureSession s) { fail("High-speed session configuration failed"); }
            },cameraHandler);
        } catch(Exception error) { fail(error.getMessage()); }
    }

    void beginLockedRecording(Float distance) throws Exception {
        if(focusLower!=null && (distance==null || distance<focusLower || distance>focusUpper)) {
            rejectedFocusDistances.put(distance==null ? JSONObject.NULL : distance);
            report.put("rejected_focus_distances_diopters",rejectedFocusDistances);
            rejectedFocusLocks++;
            if(rejectedFocusLocks>=6) throw new Exception("Autofocus could not reach calibrated distance range");
            // Release the unsuccessful lock and let continuous-video AF retry.
            // The measured workload has not started and no frames are recorded.
            captureRequest.set(CaptureRequest.CONTROL_AF_TRIGGER,CaptureRequest.CONTROL_AF_TRIGGER_CANCEL);
            session.captureBurst(session.createHighSpeedRequestList(captureRequest.build()),captureCallback,cameraHandler);
            captureRequest.set(CaptureRequest.CONTROL_AF_TRIGGER,CaptureRequest.CONTROL_AF_TRIGGER_IDLE);
            focusTriggerSent=false;focusStableSince=0;previousFocus=null;
            return;
        }
        report.put("autofocus_rejected_lock_count",rejectedFocusLocks);
        lockedFocus=distance;
        report.put("locked_focus_distance_diopters",distance).put("focus_preparation_s",(System.nanoTime()-focusPreparationNs)/1e9);
        recorder.start();startNs=System.nanoTime();recording=true;
        captureRequest.addTarget(recordingSurface);
        session.setRepeatingBurst(session.createHighSpeedRequestList(captureRequest.build()),captureCallback,cameraHandler);
        main.post(() -> status.setText("Recording " + fps + " FPS for " + seconds + " seconds · focus locked"));
        cameraHandler.postDelayed(() -> stopRecording("complete"),seconds*1000L);
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
        else if(previewOnly && !finished && cameraHandler!=null)cameraHandler.post(() -> finishReport("preview_interrupted",null));
    }
    @Override public void onDestroy() {
        if(!finished)fail("Activity destroyed");
        if(thread!=null)thread.quitSafely();
        super.onDestroy();
    }
}
