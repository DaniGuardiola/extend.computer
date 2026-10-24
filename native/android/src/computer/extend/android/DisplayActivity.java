package computer.extend.android;

import android.app.Activity;
import android.graphics.Color;
import android.os.Bundle;
import android.view.Gravity;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.view.WindowManager;
import android.widget.FrameLayout;
import android.widget.TextView;
import java.net.Socket;

/** USB development receiver. Accounts and LAN sessions belong to the shared engine. */
public final class DisplayActivity extends Activity implements SurfaceHolder.Callback {
    private SurfaceView video;
    private TextView status;
    private FrameLayout root;
    private volatile Socket socket;
    private volatile int generation;
    private boolean resumed;
    private Thread worker;

    @Override public void onCreate(Bundle saved) {
        super.onCreate(saved);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        getWindow().getDecorView().setSystemUiVisibility(
            View.SYSTEM_UI_FLAG_FULLSCREEN | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
            | View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
            | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION | View.SYSTEM_UI_FLAG_LAYOUT_STABLE);
        root = new FrameLayout(this);
        root.setBackgroundColor(Color.BLACK);
        video = new SurfaceView(this);
        video.getHolder().addCallback(this);
        root.addView(video, new FrameLayout.LayoutParams(-1, -1, Gravity.CENTER));
        status = new TextView(this);
        status.setTextColor(Color.WHITE);
        status.setTextSize(20);
        status.setGravity(Gravity.CENTER);
        status.setPadding(32, 32, 32, 32);
        status.setText("Connect this tablet to your Mac with USB.\nStart the Android display session on your Mac.");
        root.addView(status, new FrameLayout.LayoutParams(-1, -1));
        setContentView(root);
    }

    @Override protected void onResume() {
        super.onResume(); resumed = true; startIfReady();
    }
    @Override protected void onPause() {
        resumed = false; stopSession(); super.onPause();
    }
    @Override public void surfaceCreated(SurfaceHolder holder) { startIfReady(); }
    @Override public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {}
    @Override public void surfaceDestroyed(SurfaceHolder holder) { stopSession(); }

    private void stopSession() {
        generation++;
        Socket active = socket;
        if (active != null) try { active.close(); } catch (Exception ignored) {}
        socket = null;
    }

    private void startIfReady() {
        if (!resumed || !video.getHolder().getSurface().isValid()
                || (worker != null && worker.isAlive())) return;
        String token = getIntent().getStringExtra("token");
        int port = getIntent().getIntExtra("port", 0);
        if (token == null || token.length() != 64 || port < 1024 || port > 65535) return;
        final int session = ++generation;
        status.setVisibility(View.VISIBLE);
        status.setText("Connecting to your Mac…");
        worker = new Thread(() -> {
            try (Socket connection = new Socket()) {
                if (session != generation) return;
                socket = connection;
                connection.connect(new java.net.InetSocketAddress("127.0.0.1", port), 10000);
                connection.setTcpNoDelay(true);
                connection.setSoTimeout(15000);
                DisplaySession.receive(connection, token, video.getHolder().getSurface(),
                    new DisplaySession.Listener() {
                        public void configured(int width, int height) {
                            runOnUiThread(() -> {
                                if (session != generation) return;
                                int availableWidth = root.getWidth(), availableHeight = root.getHeight();
                                double scale = Math.min((double) availableWidth / width,
                                    (double) availableHeight / height);
                                video.setLayoutParams(new FrameLayout.LayoutParams(
                                    (int) Math.round(width * scale), (int) Math.round(height * scale), Gravity.CENTER));
                                status.setText("Waiting for your Mac’s display…");
                            });
                        }
                        public void rendered() {
                            runOnUiThread(() -> {
                                if (session == generation) status.setVisibility(View.GONE);
                            });
                        }
                    });
            } catch (Exception error) {
                if (session == generation) {
                    android.util.Log.e("ExtendDisplay", "Display session ended: " + error.getClass().getSimpleName()
                        + ": " + error.getMessage());
                    runOnUiThread(() -> {
                        if (session != generation) return;
                        status.setText("Display disconnected.\n" + error.getMessage()
                            + "\nRestart the session on your Mac to reconnect.");
                        status.setVisibility(View.VISIBLE);
                    });
                }
            } finally {
                if (session == generation) socket = null;
            }
        }, "extend-display-session");
        worker.start();
    }
}
