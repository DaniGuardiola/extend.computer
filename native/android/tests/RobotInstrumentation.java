package computer.extend.android.tests;

import android.app.Instrumentation;
import android.os.Bundle;
import android.view.accessibility.AccessibilityNodeInfo;
import java.util.ArrayList;
import java.util.List;
import org.json.JSONArray;
import org.json.JSONObject;

/** Debug-only UI driver in the app-signed test APK, never in the shipped application. */
public final class RobotInstrumentation extends Instrumentation {
    private Bundle arguments;
    @Override public void onCreate(Bundle arguments) { super.onCreate(arguments); this.arguments = arguments; start(); }
    private void collect(AccessibilityNodeInfo node, List<AccessibilityNodeInfo> nodes) {
        if (node == null) return;
        nodes.add(node);
        for (int i = 0; i < node.getChildCount(); i++) collect(node.getChild(i), nodes);
    }
    @Override public void onStart() {
        Bundle result = new Bundle();
        try {
            getUiAutomation().waitForIdle(300, 3000);
            List<AccessibilityNodeInfo> nodes = new ArrayList<>();
            collect(getUiAutomation().getRootInActiveWindow(), nodes);
            String action = arguments.getString("action", "dump");
            String text = arguments.getString("text", "");
            String pkg = arguments.getString("package", "");
            JSONArray details = new JSONArray();
            boolean success = false;
            for (AccessibilityNodeInfo node : nodes) {
                if (!pkg.isEmpty() && !pkg.contentEquals(node.getPackageName() == null ? "" : node.getPackageName())) continue;
                String label = node.getText() == null ? "" : node.getText().toString();
                String description = node.getContentDescription() == null ? "" : node.getContentDescription().toString();
                // Passwords are never included in debug output.
                if (node.isPassword()) continue;
                if ("dump".equals(action)) {
                    success = true;
                    if (!label.isEmpty() || !description.isEmpty()) details.put(new JSONObject().put("text", label)
                        .put("description", description).put("checked", node.isChecked()).put("clickable", node.isClickable())
                        .put("resource", node.getViewIdResourceName()));
                } else if ("scroll".equals(action) && node.isScrollable()) {
                    success = node.performAction(AccessibilityNodeInfo.ACTION_SCROLL_FORWARD);
                    if (success) break;
                } else if ((label.equals(text) || description.equals(text)) && !text.isEmpty()) {
                    if ("click".equals(action)) {
                        AccessibilityNodeInfo target = node;
                        while (target != null && !target.isClickable()) target = target.getParent();
                        if (target != null) success = target.performAction(AccessibilityNodeInfo.ACTION_CLICK);
                    } else if ("setText".equals(action) && node.isEditable()) {
                        Bundle input = new Bundle();
                        input.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, arguments.getString("value", ""));
                        success = node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, input);
                    }
                    if (success) break;
                }
            }
            result.putString("status", success ? "passed" : "failed");
            if (!success) result.putString("error", "Requested app window or UI action was not available");
            result.putString("nodes", details.toString());
            finish(success ? -1 : 0, result);
        } catch (Throwable error) { result.putString("status", "failed"); result.putString("error", error.toString()); finish(0, result); }
    }
}
