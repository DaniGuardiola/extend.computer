"""Physical receiving-permission recovery, using only disposable paired input fixtures.

Requires the lifecycle-capable tablet_input_source host fixture and the existing
observer/receiver target APKs. Does not perform camera or performance measurements.
Receipts contain private baseline metadata and must not be published verbatim.
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import time

from test_input_receiving_ui import (
    APP, TARGET, TARGET_ACTIVITY, UNICODE_TEXT, FINGERPRINT, ReceiverDriver,
    assert_editor_text, save_private, sanitize_nodes,
)

OWN_SERVICE = APP + "/computer.extend.android.InputReceivingService"
SECURE_KEYS = ("enabled_accessibility_services", "accessibility_enabled",
               "enabled_input_methods", "default_input_method", "selected_input_method_subtype")


def canonical_component(value):
    package, separator, name = value.partition("/")
    if not separator or not package or not name:
        raise RuntimeError("Malformed enabled accessibility component; no settings changed")
    return package + "/" + (package + name if name.startswith(".") else name)


def without_own_service(original):
    """Preserve every other component's spelling and ordering, without a broad reset."""
    if original is None or not original:
        raise RuntimeError("Own accessibility service is not enabled; no settings changed")
    entries = original.split(":")
    own = [index for index, value in enumerate(entries) if canonical_component(value) == OWN_SERVICE]
    if len(own) != 1:
        raise RuntimeError("Own accessibility service missing or ambiguous; no settings changed")
    return ":".join(value for index, value in enumerate(entries) if index != own[0])


def assert_denied(state, peer):
    if (state.get("peer") != peer or state.get("state") != "denied"
            or state.get("denial") != "remote-consent-denied" or state.get("text_supported") is not False):
        raise RuntimeError("Reconnect did not prove exact same-peer remote-consent-denied")


class PermissionRecoveryDriver(ReceiverDriver):
    def __init__(self, *arguments):
        super().__init__(*arguments)
        self.secure_baseline = None

    def secure_get(self, key):
        value = self.adb("shell", shlex.join(["settings", "get", "secure", key])).rstrip("\r\n")
        return None if value == "null" else value

    def secure_set(self, key, value):
        command = ["settings", "delete", "secure", key] if value is None else ["settings", "put", "secure", key, value]
        self.adb("shell", shlex.join(command))

    def capture_preferences(self):
        baseline = {key: self.secure_get(key) for key in SECURE_KEYS}
        without_own_service(baseline["enabled_accessibility_services"])
        if baseline["accessibility_enabled"] != "1":
            raise RuntimeError("Accessibility baseline is not enabled")
        # Capture all preferences before mutation. No settings or clipboard dumps.
        self.secure_baseline = baseline
        save_private(self.output / "system-preferences-before.json", baseline)

    def remove_permission(self):
        remaining = without_own_service(self.secure_baseline["enabled_accessibility_services"])
        self.secure_set("enabled_accessibility_services", remaining)
        actual = self.secure_get("enabled_accessibility_services")
        if actual != remaining:
            raise RuntimeError("Scoped accessibility removal did not retain the exact other services")
        # Leave the global accessibility switch alone, including when our service was the only one.
        save_private(self.output / "system-preferences-revoked.json", {key: self.secure_get(key) for key in SECURE_KEYS})

    def restore_preferences(self):
        """Try every captured key even if an earlier restore fails; fail closed afterward."""
        if self.secure_baseline is None:
            return []
        errors = []
        # Restore service list before its global switch. Never alter stay-awake preferences.
        for key in SECURE_KEYS:
            try:
                if self.secure_get(key) != self.secure_baseline[key]:
                    self.secure_set(key, self.secure_baseline[key])
            except Exception:
                errors.append("restore-" + key + "-failed")
        for key in SECURE_KEYS:
            try:
                if self.secure_get(key) != self.secure_baseline[key]:
                    errors.append("restore-" + key + "-mismatch")
            except Exception:
                errors.append("verify-" + key + "-failed")
        return errors

    def open_settings(self, label):
        self.adb("shell", "am", "start", "-n", APP + "/.MainActivity")
        self.wait_ui("Devices")
        self.ui("click", "Settings")
        nodes = self.wait_ui(label)
        save_private(self.output / ("permission-needed-ui.json" if label == "Accessibility permission needed" else "permission-restored-ui.json"), sanitize_nodes(nodes))
        self.preserve(self.public_state(), allow_fixture=True)
        self.ui("click", "Back"); self.wait_ui("Devices")

    def start_target(self):
        self.adb("shell", "am", "force-stop", TARGET)
        self.adb("shell", "am", "start", "-n", TARGET_ACTIVITY)
        self.wait_target_label("Extend receiver input check")

    def require_text(self, state):
        if state.get("text_supported") is not True or state.get("unicode_fixture") != UNICODE_TEXT:
            raise RuntimeError("Authenticated receiver did not grant the exact Unicode fixture")
        if state.get("unicode_utf16_units") != len(UNICODE_TEXT.encode("utf-16-le")) // 2:
            raise RuntimeError("Host Unicode fixture UTF-16 length differs")

    def exercise_recovery(self, without_held_contact=False):
        self.start_target()
        geometry = self.send({"kind": "connect"}, "connected")
        self.require_text(geometry)
        if without_held_contact:
            self.click_target("Remote click target", geometry)
            self.wait_target_label("Remote click received")
            balanced = self.wait_target_state(lambda state: state.get("touch") == "released")
            if self.target_state().get("touch") != "released":
                raise RuntimeError("Contact was not balanced before permission removal")
            save_private(self.output / "native-balanced-click-before-revoke.json", balanced)
        else:
            nodes = self.observe_target()
            button = [node for node in nodes if node.get("description") == "Remote click target"]
            if len(button) != 1:
                raise RuntimeError("Disposable held-contact target missing or ambiguous")
            a, b, c, d = map(int, button[0]["bounds"].split())
            x, y = (a+c)/2/geometry["width"], (b+d)/2/geometry["height"]
            if not (0 <= x <= 1 and 0 <= y <= 1 and a < c and b < d):
                raise RuntimeError("Held-contact target differs from authenticated geometry")
            time.sleep(1)  # Detach observer before starting a continued native contact.
            self.send({"kind": "hold", "x": x, "y": y})
            held = self.wait_target_state(lambda state: state.get("touch") == "held")
            time.sleep(.5)
            if self.target_state().get("touch") != "held":
                raise RuntimeError("Contact was not still held at permission removal")
            save_private(self.output / "native-contact-before-revoke.json", held)
        self.remove_permission()
        if not without_held_contact:
            released = self.wait_target_state(lambda state: state.get("touch") == "released")
            save_private(self.output / "native-contact-after-revoke.json", released)
        disconnected = self.source_state("disconnected")
        save_private(self.output / "source-after-revoke.json", disconnected)
        self.open_settings("Accessibility permission needed")
        denied = self.send({"kind": "connect"}, "denied")
        assert_denied(denied, self.peer)
        save_private(self.output / "source-reconnect-denied.json", denied)
        errors = self.restore_preferences()
        if errors:
            raise RuntimeError("Original service/IME preferences could not be restored")
        self.open_settings("Ready for paired devices")
        self.start_target()
        geometry = self.send({"kind": "connect"}, "connected")
        self.require_text(geometry)
        self.click_target("Remote click target", geometry)
        self.wait_target_label("Remote click received")
        save_private(self.output / "native-click-after-restore.json", self.target_state())
        self.click_target("Remote typing target", geometry)
        self.wait_editor_focus()
        self.send({"kind": "unicode"})
        text = self.wait_target_state(lambda state: state.get("text") == UNICODE_TEXT)
        assert_editor_text(text, UNICODE_TEXT)
        observed = self.observe_target()
        typing = [node for node in observed if node.get("description") == "Remote typing target"]
        if len(typing) != 1 or typing[0].get("text") != UNICODE_TEXT:
            raise RuntimeError("Actual editor accessibility content differs from delivered Unicode")
        save_private(self.output / "native-unicode-after-restore.json", text)
        save_private(self.output / "unicode-after-restore-ui.json", sanitize_nodes(observed))
        self.send({"kind": "close"}, "closed")
        self.process.wait(timeout=5)
        if self.process.returncode:
            raise RuntimeError("Disposable lifecycle source did not close normally")

    def cleanup_peer(self):
        self.adb("shell", "am", "start", "-n", APP + "/.MainActivity")
        self.wait_ui("Devices")
        self.remove_own_peer(self.name)
        final = self.public_state()
        self.preserve(final)
        save_private(self.output / "public-state-after.json", final)


def run(runner, requested_address, preserve_names, without_held_contact=False):
    result = {"status": "failed", "tablet_peer": runner.tablet_peer,
              "mode": "recovery-only" if without_held_contact else "strict-held-contact",
              "held_revocation_accepted": False}
    try:
        initial = runner.wait_ui("Devices")
        for name in preserve_names:
            if not any(node.get("text") == name for node in initial):
                raise RuntimeError("Required baseline peer is not visible")
        runner.capture_preferences()
        runner.pair(requested_address, lifecycle=True)
        runner.exercise_recovery(without_held_contact=without_held_contact)
        contact_checks = (["native-balanced-click-before-revoke"] if without_held_contact else
                          ["native-held-contact", "native-contact-release"])
        result.update(status="passed", checks=contact_checks + ["own-service-only-revocation",
            "source-disconnect", "permission-needed-ui",
            "same-identity-exact-reconnect-denial", "original-service-and-ime-preferences-restored",
            "ready-ui", "fresh-native-click", "exact-native-and-accessibility-unicode"])
    except Exception as error:
        result["error"] = str(error) if isinstance(error, RuntimeError) else "Permission recovery gate failed; raw details suppressed"
        # Preserve the failed state before restoration or peer cleanup.
        try:
            result["failure_evidence"] = runner.capture_failure()
        except Exception:
            result["failure_evidence"] = "unavailable-before-cleanup"
        try:
            save_private(runner.output / "failure-system-preferences.json", {key: runner.secure_get(key) for key in SECURE_KEYS})
        except Exception:
            result["failure_system_preferences"] = "unavailable"
    finally:
        result["peer"] = runner.peer
        result["source"] = runner.last_source
        try:
            result["cleanup_errors"] = runner.cleanup()
        except Exception:
            result["cleanup_errors"] = ["source-cleanup-failed"]
            result["status"] = "failed"
        # Independent from process cleanup, including cleanup exceptions.
        result["restore_errors"] = runner.restore_preferences()
        if result["restore_errors"]:
            result["status"] = "failed"
        if runner.secure_baseline is not None:
            try:
                save_private(runner.output / "system-preferences-after.json",
                             {key: runner.secure_get(key) for key in SECURE_KEYS})
            except Exception:
                result["status"] = "failed"
                result["restore_errors"].append("final-preference-receipt-failed")
        if runner.peer:
            try:
                runner.cleanup_peer()
                final = runner.ui()
                for name in preserve_names:
                    if not any(node.get("text") == name for node in final):
                        raise RuntimeError("Required baseline peer is no longer visible")
                result["baseline_preserved"] = True
                if result["status"] == "passed":
                    result["checks"].extend(["own-peer-only-unpair", "baseline-public-state-and-settings-preserved"])
            except Exception:
                result["status"] = "failed"
                result["own_peer_cleanup"] = "needs-root-review"
        result["private_code_deleted"] = not runner.fixture or not runner.fixture.exists()
        if not result["private_code_deleted"]:
            result["status"] = "failed"
        result["held_revocation_accepted"] = result["status"] == "passed" and not without_held_contact
        save_private(runner.output / "result.json", result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--tablet-peer", required=True)
    parser.add_argument("--pair-address")
    parser.add_argument("--without-held-contact", action="store_true",
                        help="Recovery-only check after a balanced click; does not accept held-contact revocation")
    parser.add_argument("--preserve-name", action="append", default=[])
    parser.add_argument("--peer-executable", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="New private receipt directory")
    args = parser.parse_args()
    if not FINGERPRINT.fullmatch(args.tablet_peer): parser.error("Invalid tablet fingerprint")
    if not args.peer_executable.is_file(): parser.error("Build the isolated lifecycle fixture first")
    os.umask(0o077)
    output = args.output.resolve(); output.mkdir(parents=True, exist_ok=False, mode=0o700)
    runner = PermissionRecoveryDriver(args.serial, output, args.peer_executable.resolve(), args.tablet_peer)
    result = run(runner, args.pair_address, args.preserve_name, args.without_held_contact)
    print(json.dumps({"status": result["status"], "output": str(output)}))
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
