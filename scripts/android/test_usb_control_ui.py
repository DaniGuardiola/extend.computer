#!/usr/bin/env python3
"""Verify installed Desktop USB control in the existing disposable Android editor.

The device owner runs this with the development app and observer/target installed,
the existing devices paired, receiving enabled, Wi-Fi off, and the old Mac app
quit with USB disconnected. No pairing, ADB reverse, LAN fallback or global Mac
input capture. At READY, use the normal Mac GUI Stop sharing before the 90s source
watchdog. Receipts are private; they contain public device metadata only.
"""
import argparse
import json
import math
import os
from pathlib import Path
import re
import shlex
import subprocess
import time

from test_input_receiving_ui import (
    APP, TARGET, TARGET_ACTIVITY, UNICODE_TEXT, FINGERPRINT, ReceiverDriver,
    assert_editor_text, editor_focused, save_private, sanitize_nodes,
)
from test_input_permission_recovery_ui import SECURE_KEYS

ROOT = Path(__file__).resolve().parents[2]
ADB = ROOT / "mobile/.local/sdk/platform-tools/adb"
HASH_FILES = (
    "no_backup/engine/identity.v1", "no_backup/engine/trust/trust.json",
    "no_backup/engine/peer-names.json", "no_backup/engine/peer-hosts.json",
    "shared_prefs/engine-peers.xml", "shared_prefs/engine-local.xml",
    "shared_prefs/engine-labels.xml", "shared_prefs/account-config.xml",
    "shared_prefs/engine-screen-sharing.xml", "shared_prefs/appearance.xml",
)
MANDATORY_FILES = frozenset(HASH_FILES[:2])
SYSTEM_KEYS = ("secure", SECURE_KEYS), ("system", ("screen_off_timeout",)), (
    "global", ("stay_on_while_plugged_in",))


def file_digest(raw, path):
    """Never mistake a directory, missing file or shell error for a SHA receipt."""
    if path not in HASH_FILES:
        raise ValueError("Not an allowlisted metadata path")
    if raw.strip() == "ABSENT" and path not in MANDATORY_FILES:
        return {"present": False}
    match = re.fullmatch(r"([0-9a-f]{64}) [ *]" + re.escape(path) + r"\r?\n?", raw)
    if not match:
        raise RuntimeError("Required regular metadata file or exact SHA receipt missing")
    return {"present": True, "sha256": match[1]}


def validate_source(state, tablet, mac, session=None):
    if (state.get("status") != "diagnostic-only" or state.get("route") != "usb"
            or state.get("peer") != tablet or state.get("local_identity") != mac
            or state.get("global_input_capture") is not False
            or state.get("native_delivery_verified") is not False
            or state.get("unicode_fixture") != UNICODE_TEXT
            or state.get("unicode_utf16_units") != 28):
        raise RuntimeError("Installed source USB identity or fixed-input receipt differs")
    sequence = state.get("command_sequence")
    if type(sequence) is not int or not 0 <= sequence <= 16:
        raise RuntimeError("Source command sequence is invalid")
    phase = state.get("state")
    if phase not in ("starting", "connected", "click", "type", "unicode", "hold",
                     "closed", "disconnected", "expired", "failed"):
        raise RuntimeError("Unknown source phase")
    if phase != "starting" and phase != "failed":
        if (state.get("authenticated_peer") != tablet or state.get("text_supported") is not True
                or type(state.get("session")) is not int or state["session"] <= 0):
            raise RuntimeError("Exact pinned USB control and text grant missing")
        if session is not None and state["session"] != session:
            raise RuntimeError("Diagnostic session changed")
        for key in ("width", "height"):
            value = state.get(key)
            if type(value) not in (int, float) or not math.isfinite(value) or not 1 <= value <= 16384:
                raise RuntimeError("Authenticated display geometry invalid")
    if phase in ("failed", "expired"):
        raise RuntimeError("USB diagnostic failed or reached its lifetime limit")
    return state


def target_point(nodes, label, geometry):
    matches = [node for node in nodes if node.get("description") == label]
    if len(matches) != 1:
        raise RuntimeError("Disposable target is missing or ambiguous")
    try:
        a, b, c, d = map(int, matches[0]["bounds"].split())
        width, height = geometry["width"], geometry["height"]
        if not (0 <= a < c <= width and 0 <= b < d <= height):
            raise ValueError()
        return {"x": (a + c) / 2 / width, "y": (b + d) / 2 / height}
    except (ValueError, KeyError, TypeError):
        raise RuntimeError("Observed target exceeds authenticated display bounds") from None


def gui_disconnected(state, sequence):
    return (state.get("state") == "disconnected" and state.get("reason") == "session-cancelled"
            and state.get("command_sequence") == sequence)


class UsbControlDriver(ReceiverDriver):
    def __init__(self, serial, output, bundle, tablet, mac, vendor, restore_wifi):
        executable = bundle / "Contents/MacOS/extend-computer-desktop"
        super().__init__(serial, output, executable, tablet)
        self.base = [str(ADB), "-s", serial]
        self.peer, self.mac, self.vendor, self.serial = mac, mac, vendor, serial
        self.session = self.deadline = self.preferences = self.hashes = self.initial_pid = None
        self.restore_wifi, self.cleanup_armed, self.target_started = restore_wifi, False, False

    def adb(self, *arguments, timeout=10):
        # Bound each call against the active source lifetime, including observer.
        if self.deadline is not None:
            timeout = min(timeout, max(.1, self.deadline - time.monotonic()))
        return super().adb(*arguments, timeout=timeout)

    def setting(self, namespace, key):
        value = self.adb("shell", shlex.join(["settings", "get", namespace, key])).rstrip("\r\n")
        return None if value == "null" else value

    def system_preferences(self):
        return {namespace: {key: self.setting(namespace, key) for key in keys}
                for namespace, keys in SYSTEM_KEYS}

    def metadata_hashes(self):
        result = {}
        for path in HASH_FILES:
            script = f"if test -f {path}; then sha256sum {path}; else printf ABSENT; fi"
            raw = self.adb("shell", shlex.join(["run-as", APP, "sh", "-c", script]))
            result[path] = file_digest(raw, path)
        return result

    def read_source(self):
        if not self.receipt_path.is_file():
            return None
        if self.receipt_path.stat().st_size > 8192:
            raise RuntimeError("Source receipt exceeds its fixed bound")
        state = json.loads(self.receipt_path.read_text())
        self.last_source = state
        validate_source(state, self.tablet_peer, self.mac, self.session)
        if state.get("authenticated_peer"):
            self.session = state["session"]
        return state

    def capture_failure(self):
        evidence = {"status": "failed", "captured_before_cleanup": True, "unavailable": []}
        try: self.read_source()
        except Exception: pass
        for label, read in (("native-editor", self.target_state),
                            ("editor-ui", lambda: sanitize_nodes(self.observe_target())),
                            ("public-state", self.public_state)):
            try:
                value = read()
                filename = "failure-" + label + ".json"
                save_private(self.output / filename, value)
                evidence[label] = filename
                if label == "public-state" and self.baseline is not None:
                    evidence["input_receiving"] = {"before": self.baseline["input_receiving"], "observed": value["input_receiving"]}
                    try: self.preserve(value); evidence["existing_peers_preserved"] = True
                    except RuntimeError: evidence["existing_peers_preserved"] = False
            except Exception: evidence["unavailable"].append(label)
        evidence["source"] = self.last_source
        save_private(self.output / "failure-evidence.json", evidence)
        return evidence

    def source_state(self, expected, sequence=None, timeout=20):
        deadline = min(time.monotonic() + timeout, self.deadline or float("inf"))
        while time.monotonic() < deadline:
            state = self.read_source()
            if state and state["state"] == expected and (sequence is None or state["command_sequence"] == sequence):
                return state
            if state and state["state"] in ("closed", "disconnected"):
                raise RuntimeError("USB source ended before the requested delivery stage")
            if self.process.poll() is not None:
                raise RuntimeError("Installed Mac source process ended unexpectedly")
            time.sleep(.05)
        raise RuntimeError("USB source receipt timed out")

    def preflight(self):
        if self.adb("get-state").strip() != "device" or self.setting("global", "wifi_on") != "0":
            raise RuntimeError("Wired ADB and Wi-Fi off are required before starting")
        self.cleanup_armed = True
        self.initial_pid = self.adb("shell", "pidof", APP).strip()
        if not re.fullmatch(r"[0-9]+", self.initial_pid):
            raise RuntimeError("One existing application process is required")
        self.wait_ui("Devices", timeout=10)
        self.hashes = self.metadata_hashes()
        self.baseline = self.public_state()
        if self.mac not in self.baseline["trust"]["peers"] or not self.baseline["availability"] or not self.baseline["input_receiving"]:
            raise RuntimeError("Exact existing Mac pairing and receiving preferences required")
        self.preferences = self.system_preferences()
        save_private(self.output / "baseline.json", {"public": self.baseline, "files": self.hashes,
                     "preferences": self.preferences, "wifi": "0", "application_pid": self.initial_pid})
        self.ui("click", "Settings")
        self.wait_ui("Ready for paired devices", timeout=10)
        # The full fingerprint is lower in the normal settings scroll view.
        for _ in range(4):
            nodes = self.ui()
            if any(node.get("text") == self.tablet_peer for node in nodes):
                save_private(self.output / "tablet-identity-ui.json", sanitize_nodes(nodes))
                break
            self.ui("scroll")
        else:
            raise RuntimeError("Normal settings did not show the expected tablet identity")
        self.ui("click", "Back"); self.wait_ui("Devices", timeout=10)
        self.adb("shell", "am", "force-stop", TARGET)
        self.adb("shell", "am", "start", "-n", TARGET_ACTIVITY)
        self.target_started = True
        initial = self.wait_target_label("Extend receiver input check")
        editors = [n for n in initial if n.get("description") == "Remote typing target"]
        if len(editors) != 1 or editors[0].get("text") not in ("", "Remote typing target") or self.target_state().get("text", ""):
            raise RuntimeError("Disposable editor did not start empty")
        save_private(self.output / "target-before.json", sanitize_nodes(initial))

    def exercise(self, disconnect_timeout):
        with open(self.output / "source.log", "x", opener=lambda path, flags: os.open(path, flags, 0o600)) as log:
            self.process = subprocess.Popen([str(self.executable), "--usb-control-test", self.tablet_peer,
                self.vendor, self.serial, str(self.receipt_path)], stdin=subprocess.PIPE, stdout=log, stderr=log, text=True)
        geometry = self.source_state("connected", 0, timeout=45)
        self.deadline = time.monotonic() + 85
        save_private(self.output / "source-connected.json", geometry)
        if self.setting("global", "wifi_on") != "0" or self.adb("shell", "pidof", APP).strip() != self.initial_pid:
            raise RuntimeError("Wi-Fi or existing app process changed during USB startup")
        # Accessory attachment can foreground the main app. Return only our
        # disposable target after the grant; never capture another foreground app.
        self.adb("shell", "am", "start", "-n", TARGET_ACTIVITY)
        self.wait_target_label("Extend receiver input check")
        self.send({"kind": "click", **target_point(self.observe_target(), "Remote click target", geometry)})
        self.wait_target_label("Remote click received")
        click = self.wait_target_state(lambda state: state.get("touch") == "released")
        save_private(self.output / "native-click.json", click)
        self.send({"kind": "click", **target_point(self.observe_target(), "Remote typing target", geometry)})
        self.wait_editor_focus()
        self.send({"kind": "type"})
        physical = self.wait_target_state(lambda state: state.get("text") == "extend")
        assert_editor_text(physical, "extend")
        save_private(self.output / "physical-key-editor.json", physical)
        self.send({"kind": "unicode"})
        expected = "extend" + UNICODE_TEXT
        text = self.wait_target_state(lambda state: state.get("text") == expected)
        assert_editor_text(text, expected)
        if text["text-changes"] <= physical["text-changes"]:
            raise RuntimeError("Unicode delivery did not make a new native editor change")
        observed = self.observe_target()
        editors = [node for node in observed if node.get("description") == "Remote typing target"]
        if len(editors) != 1 or editors[0].get("text") != expected or not editor_focused(observed):
            raise RuntimeError("Independent observer did not verify exact focused Unicode editor")
        save_private(self.output / "unicode-editor.json", text)
        save_private(self.output / "unicode-editor-ui.json", sanitize_nodes(observed))
        point = target_point(observed, "Remote click target", geometry)
        self.send({"kind": "hold", **point})
        held = self.wait_target_state(lambda state: state.get("touch") == "held")
        if self.setting("global", "wifi_on") != "0":
            raise RuntimeError("Wi-Fi must remain off through the held USB contact")
        save_private(self.output / "ready-for-gui-disconnect.json", {"status": "awaiting-normal-mac-disconnect",
            "source": self.last_source, "native_held": held, "wifi": "0",
            "instruction": "Click Stop sharing in the normal Mac app now. Do not close driver stdin."})
        print("READY: click normal Mac Stop sharing now; receipt " + str(self.output / "ready-for-gui-disconnect.json"), flush=True)
        terminal = self.source_state("disconnected", self.command_sequence, timeout=disconnect_timeout)
        if not gui_disconnected(terminal, self.command_sequence):
            raise RuntimeError("Source closure was not cancellation of the held GUI session")
        released = self.wait_target_state(lambda state: state.get("touch") == "released")
        assert_editor_text(released, expected)
        save_private(self.output / "source-after-gui-disconnect.json", terminal)
        save_private(self.output / "native-after-gui-disconnect.json", released)

    def cleanup(self):
        errors = []
        self.deadline = None
        # Own stdin only; the Mac GUI/carrier remains for the device owner.
        if self.process and self.process.stdin:
            try: self.process.stdin.close()
            except OSError: errors.append("source-stdin-close-failed")
        if not self.cleanup_armed:
            return errors
        if self.preferences is not None:
            for key, value in self.preferences["secure"].items():
                try:
                    if self.setting("secure", key) != value:
                        command = ["settings", "delete", "secure", key] if value is None else ["settings", "put", "secure", key, value]
                        self.adb("shell", shlex.join(command))
                except Exception: errors.append("secure-preference-restore-failed-" + key)
        try:
            expected_wifi = "1" if self.restore_wifi == "on" else "0"
            self.adb("shell", "svc", "wifi", "enable" if self.restore_wifi == "on" else "disable")
            deadline = time.monotonic() + 10
            while self.setting("global", "wifi_on") != expected_wifi:
                if time.monotonic() >= deadline: raise RuntimeError("Wi-Fi restoration timed out")
                time.sleep(.1)
        except Exception: errors.append("wifi-restore-failed")
        try:
            if self.target_started:
                self.adb("shell", "am", "start", "-n", APP + "/.MainActivity")
                self.wait_ui("Devices", timeout=10)
            if self.baseline is not None:
                state = self.public_state()
                if any(state[key] != self.baseline[key] for key in ("availability", "input_receiving")):
                    self.ui("click", "Settings")
                    for key, label in (("availability", "Stay available"), ("input_receiving", "Receive keyboard and mouse")):
                        if self.public_state()[key] != self.baseline[key]: self.ui("click", label)
                    self.ui("click", "Back"); self.wait_ui("Devices", timeout=10)
                final = self.public_state()
                self.preserve(final)
                hashes = self.metadata_hashes()
                if hashes != self.hashes: raise RuntimeError("Persistent file hashes changed")
                preferences = self.system_preferences()
                if preferences != self.preferences: raise RuntimeError("Original system preferences changed")
                pid = self.adb("shell", "pidof", APP).strip()
                if pid != self.initial_pid: raise RuntimeError("Original app process changed")
                save_private(self.output / "baseline-after.json", {"public": final, "files": hashes,
                    "preferences": preferences, "wifi": expected_wifi, "application_pid": pid})
        except Exception: errors.append("target-dismiss-or-baseline-restore-failed")
        return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True, help="Wired tablet ADB/USB serial, never an IP address")
    parser.add_argument("--bundle", type=Path, required=True, help="Exact installed diagnostic-enabled Mac .app")
    parser.add_argument("--tablet-peer", required=True)
    parser.add_argument("--mac-peer", required=True)
    parser.add_argument("--vendor", default="2717", help="Observed USB vendor ID in hexadecimal")
    parser.add_argument("--restore-wifi", choices=("on", "off"), required=True, help="Owner-authorized Wi-Fi cleanup state")
    parser.add_argument("--output", type=Path, required=True, help="New private receipt directory")
    parser.add_argument("--disconnect-timeout", type=int, default=40, choices=range(1, 61), metavar="1..60")
    args = parser.parse_args()
    bundle = args.bundle.resolve()
    if (not re.fullmatch(r"[A-Za-z0-9._-]{1,127}", args.serial)
            or not re.fullmatch(r"[0-9a-fA-F]{1,4}", args.vendor)
            or any(not FINGERPRINT.fullmatch(peer) for peer in (args.tablet_peer, args.mac_peer))
            or args.tablet_peer == args.mac_peer or bundle.suffix != ".app"
            or not (bundle / "Contents/MacOS/extend-computer-desktop").is_file() or not ADB.is_file()):
        parser.error("Exact paired identities, wired serial, vendor and installed Mac bundle required")
    os.umask(0o077)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False, mode=0o700)
    driver = UsbControlDriver(args.serial, output, bundle, args.tablet_peer, args.mac_peer, args.vendor, args.restore_wifi)
    result = {"status": "failed", "route": "usb", "tablet_peer": args.tablet_peer, "mac_peer": args.mac_peer,
              "global_input_capture": False, "scope": "Installed native USB delivery and normal GUI disconnect only"}
    try:
        driver.preflight()
        driver.exercise(args.disconnect_timeout)
        result.update(status="passed", checks=["wifi-off-pinned-usb-control-and-text-grant", "native-button-click",
            "physical-extend-keys", "exact-28-unit-unicode-native-and-independent-observer",
            "held-contact-normal-gui-disconnect", "native-contact-released"])
    except Exception as error:
        result["error"] = str(error) if isinstance(error, RuntimeError) else "USB acceptance failed; raw details suppressed"
        try: result["failure_evidence"] = driver.capture_failure()
        except Exception: result["failure_evidence"] = "unavailable-before-cleanup"
    finally:
        try: result["cleanup_errors"] = driver.cleanup()
        except Exception: result["cleanup_errors"] = ["cleanup-failed"]
        if result["cleanup_errors"]: result["status"] = "failed"
        result["source"] = driver.last_source
        save_private(output / "result.json", result)
    print(result["status"].upper() + ": " + str(output / "result.json"), flush=True)
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
