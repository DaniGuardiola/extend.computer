#!/usr/bin/env python3
"""Verify authenticated physical keys and committed Unicode in a disposable editor.

Requires the development app, separate observer and app-signed engine-test target
already installed; availability/input receiving enabled; fixed orientation; and
Devices foreground. Preserves baseline public state, settings and every existing
peer. Only this run's newly paired source is removed through normal UI.
"""
import argparse
import ipaddress
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import time
import xml.etree.ElementTree as ET

from test_manual_pairing_ui import Driver, FINGERPRINT, assert_preserved, save_private, sanitize_nodes

ROOT = Path(__file__).resolve().parents[2]
APP = "computer.extend.android.development"
TARGET = "computer.extend.android.enginetest"
TARGET_ACTIVITY = TARGET + "/computer.extend.android.tests.ReceiverTargetActivity"
UNICODE_TEXT = "Café é — 中文测试 👩‍💻 🧑🏽‍🚀"


def target_preferences(raw):
    values = {}
    for item in ET.fromstring(raw):
        name = item.attrib.get("name", "")
        if item.tag == "string": values[name] = item.text or ""
        elif item.tag in ("int", "long"): values[name] = int(item.attrib["value"])
        elif item.tag == "boolean": values[name] = item.attrib["value"] == "true"
    return values


def assert_editor_text(state, expected):
    units = len(expected.encode("utf-16-le")) // 2
    if state.get("text") != expected or state.get("text-utf16") != units:
        raise RuntimeError("Disposable editor content differs from committed input")
    if state.get("text-changes", 0) <= 0:
        raise RuntimeError("Disposable editor did not record a real text change")


def editor_focused(nodes):
    """Actual input focus on the one disposable editor, not accessibility focus."""
    editors = [node for node in nodes if node.get("description") == "Remote typing target"]
    if len(editors) > 1: raise RuntimeError("Disposable editor is ambiguous")
    return len(editors) == 1 and editors[0].get("focused") is True


def fixture_fields(nodes, requested_address=None):
    codes = {node.get("text", "") for node in nodes if re.fullmatch(r"[0-9a-f]{4}(?:-[0-9a-f]{4}){3}", node.get("text", ""))}
    addresses = list(dict.fromkeys(node.get("description", "").removeprefix("Pairing address ") for node in nodes
                                   if node.get("description", "").startswith("Pairing address ")))
    if len(codes) != 1 or not addresses:
        raise RuntimeError("One current native code and an observed pairing address required")
    if requested_address:
        if requested_address not in addresses:
            raise RuntimeError("Requested address was not displayed by the tablet")
        address = requested_address
    else:
        ipv4 = [value for value in addresses if not value.startswith("[")]
        address = (ipv4 or addresses)[0]
    host, port = address.rsplit(":", 1)
    ipaddress.ip_address(host.strip("[]"))
    if not 0 < int(port) <= 65535:
        raise RuntimeError("Observed pairing port invalid")
    return next(iter(codes)), address


def source_failure(state):
    """Only allowlisted diagnostic fields; never forward arbitrary source errors."""
    stage = state.get("failure_stage", state.get("stage", "unknown"))
    if not isinstance(stage, str) or not re.fullmatch(r"[a-z-]{1,64}", stage): stage = "unknown"
    kind = state.get("error_kind")
    if not isinstance(kind, str) or not re.fullmatch(r"[A-Za-z]{1,40}", kind): kind = "unclassified"
    address = state.get("selected_pairing_address", "unknown")
    try:
        host, port = address.rsplit(":", 1)
        ipaddress.ip_address(host.strip("[]"))
        if not 0 < int(port) <= 65535: address = "unknown"
    except (ValueError, AttributeError): address = "unknown"
    return f"Disposable input source failed: stage={stage}, io={kind}, pairing_address={address}"


class ReceiverDriver(Driver):
    def __init__(self, serial, output, executable, tablet_peer):
        super().__init__(serial, APP, output, executable)
        self.tablet_peer = tablet_peer
        self.command_sequence = 0
        self.receipt_path = output / "source.json"
        self.last_source = None

    def public_state(self):
        state = super().public_state()
        local = ET.fromstring(self.read_file("shared_prefs/engine-local.xml", "<map/>"))
        state["input_receiving"] = next((item.attrib.get("value") == "true" for item in local
                                         if item.tag == "boolean" and item.attrib.get("name") == "input-receiving"), False)
        return state

    def preserve(self, state, allow_fixture=False):
        assert_preserved(self.baseline, state, self.peer, allow_fixture)
        if state["input_receiving"] != self.baseline["input_receiving"]:
            raise RuntimeError("Input receiving preference changed")

    def observe_target(self):
        command = ["am", "instrument", "-w", "-e", "action", "dump", "-e", "package", TARGET,
                   "computer.extend.android.observer/computer.extend.android.tests.ObserverInstrumentation"]
        raw = self.adb("shell", shlex.join(command))
        fields = dict(line.removeprefix("INSTRUMENTATION_RESULT: ").split("=", 1) for line in raw.splitlines()
                      if line.startswith("INSTRUMENTATION_RESULT: ") and "=" in line)
        if fields.get("status") != "passed":
            raise RuntimeError("Disposable editor is not the observed foreground window")
        return json.loads(fields["nodes"])

    def wait_target_label(self, label):
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            nodes = self.observe_target()
            if any(label in (node.get("text"), node.get("description")) for node in nodes): return nodes
            time.sleep(.1)
        raise RuntimeError("Disposable editor target label missing")

    def read_source(self):
        if not self.receipt_path.is_file(): return None
        state = json.loads(self.receipt_path.read_text())
        if not FINGERPRINT.fullmatch(state.get("peer", "")):
            raise RuntimeError("Source receipt fingerprint invalid")
        if self.peer is not None and state["peer"] != self.peer:
            raise RuntimeError("Source identity changed within this run")
        self.last_source = state
        if self.peer is None:
            if state["peer"] in self.baseline["trust"]["peers"]:
                raise RuntimeError("Fresh source identity conflicts with existing trust")
            if state.get("name") != "Input test " + state["peer"][:8]:
                raise RuntimeError("Fresh source name differs from its disposable identity")
            self.peer, self.name = state["peer"], state["name"]
        if state["state"] == "failed": raise RuntimeError(source_failure(state))
        return state

    def source_state(self, expected, sequence=None, timeout=20):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            state = self.read_source()
            if state and (expected is None or state["state"] == expected) and (sequence is None or state["command_sequence"] == sequence):
                return state
            if self.process.poll() is not None:
                raise RuntimeError("Disposable input source ended before the required receipt")
            time.sleep(.05)
        raise RuntimeError("Disposable input source receipt timed out: " + str(expected))

    def pairing_request(self):
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            self.read_source()
            if self.process.poll() is not None:
                raise RuntimeError("Disposable input source ended before the pairing request")
            nodes = self.ui()
            if any(node.get("text") == "Allow pairing?" for node in nodes): return nodes
            time.sleep(.05)
        raise RuntimeError("Incoming pairing approval did not appear; inspect the saved stage receipt")

    def send(self, command, expected=None):
        self.command_sequence += 1
        self.process.stdin.write(json.dumps({**command, "sequence": self.command_sequence}) + "\n")
        self.process.stdin.flush()
        state = self.source_state(expected or command["kind"], self.command_sequence)
        save_private(self.output / ("command-" + str(self.command_sequence) + ".json"), state)
        return state

    def target_state(self):
        # Only our disposable target's event record, not another app's content.
        return target_preferences(self.adb("shell", "run-as", TARGET, "cat", "shared_prefs/input-target.xml"))

    def wait_target_state(self, check, timeout=10):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            state = self.target_state()
            if check(state): return state
            time.sleep(.05)
        raise RuntimeError("Disposable editor did not deliver the required native event")

    def wait_editor_focus(self, timeout=10):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self.read_source()
            nodes = self.observe_target()
            if editor_focused(nodes):
                save_private(self.output / "editor-focused-before-type.json", sanitize_nodes(nodes))
                return nodes
            if self.process.poll() is not None:
                raise RuntimeError("Disposable source ended before editor input focus")
            time.sleep(.05)
        raise RuntimeError("Remote click did not focus the actual disposable editor")

    def capture_failure(self):
        """Capture evidence before source/UI cleanup changes the observed state."""
        evidence = {"status": "failed", "captured_before_cleanup": True, "unavailable": []}
        try: self.read_source()
        except Exception: pass  # read_source retains a terminal failed receipt before raising.
        for label, filename, read in (
            ("native-editor", "failure-target-native.json", self.target_state),
            ("editor-ui", "failure-target-ui.json", lambda: sanitize_nodes(self.observe_target())),
            ("public-state", "failure-public-state.json", self.public_state),
        ):
            try:
                value = read()
                save_private(self.output / filename, value)
                evidence[label] = filename
                if label == "editor-ui": evidence["editor_focused"] = editor_focused(value)
                if label == "public-state" and self.baseline is not None:
                    evidence["input_receiving"] = {"before": self.baseline["input_receiving"], "observed": value["input_receiving"]}
                    evidence["availability"] = {"before": self.baseline["availability"], "observed": value["availability"]}
                    try:
                        self.preserve(value, allow_fixture=True)
                        evidence["baseline_preserved_allowing_only_fixture"] = True
                    except RuntimeError:
                        evidence["baseline_preserved_allowing_only_fixture"] = False
            except Exception:
                # Only our target and allowlisted public metadata are queried.
                # Do not retain arbitrary ADB/instrumentation exception text.
                evidence["unavailable"].append(label)
        evidence["source"] = self.last_source
        save_private(self.output / "failure-evidence.json", evidence)
        return evidence

    def click_target(self, label, geometry):
        matches = [node for node in self.observe_target() if node.get("description") == label]
        if len(matches) != 1: raise RuntimeError("Disposable input target missing or ambiguous")
        a, b, c, d = map(int, matches[0]["bounds"].split())
        x, y = (a+c)/2/geometry["width"], (b+d)/2/geometry["height"]
        if not (0 <= x <= 1 and 0 <= y <= 1 and a < c and b < d):
            raise RuntimeError("Disposable target differs from the authenticated physical display")
        self.send({"kind": "click", "x": x, "y": y})

    def pair(self, requested_address, lifecycle=False):
        self.wait_ui("Devices")
        self.baseline = self.public_state()
        save_private(self.output / "public-state-before.json", self.baseline)
        if not self.baseline["availability"] or not self.baseline["input_receiving"]:
            raise RuntimeError("Enable availability/input receiving normally before running")
        self.ui("click", "Settings")
        self.wait_ui("Ready for paired devices")
        self.ui("click", "Back"); self.wait_ui("Devices")
        self.ui("click", "Pair device"); self.wait_ui("Pair manually")
        self.ui("click", "Pair manually"); self.wait_ui("Show my code")
        self.ui("click", "Show my code"); self.wait_ui("Pairing code")
        code_nodes = self.ui()
        code, address = fixture_fields(code_nodes, requested_address)
        save_private(self.output / "pairing-selection.json", {
            "selected_pairing_address": address,
            "observed_addresses": list(dict.fromkeys(node.get("description", "").removeprefix("Pairing address ")
                for node in code_nodes if node.get("description", "").startswith("Pairing address "))),
            "code_observed": True,
            "expiry_labels": [node["text"] for node in code_nodes if re.fullmatch(r"Expires in \d+ seconds", node.get("text", ""))],
        })
        self.fixture = self.output / "private-fixture.json"
        save_private(self.fixture, {"address": address, "code": code, "peer": self.tablet_peer,
                                    **({"lifecycle": True} if lifecycle else {})})
        with open(self.output / "source.log", "x", opener=lambda file, flags: os.open(file, flags, 0o600)) as log:
            self.process = subprocess.Popen([str(self.executable), str(self.fixture), str(self.receipt_path)],
                                            stdin=subprocess.PIPE, stdout=log, stderr=log, text=True)
        self.source_state(None)
        nodes = self.pairing_request()
        if not any(node.get("text") == "Device ID: " + self.peer for node in nodes):
            raise RuntimeError("Pairing dialog is not the disposable source")
        self.ui("click", "Allow pairing"); self.wait_ui("Device paired")
        self.ui("click", "Done"); self.wait_ui("Devices")
        self.source_state("paired")
        paired = self.public_state()
        if self.peer not in paired["trust"]["peers"]:
            raise RuntimeError("Explicit consent did not create the expected pinned peer")
        self.preserve(paired, allow_fixture=True)
        save_private(self.output / "public-state-paired.json", paired)

    def exercise(self):
        self.adb("shell", "am", "force-stop", TARGET)
        self.adb("shell", "am", "start", "-n", TARGET_ACTIVITY)
        self.wait_target_label("Extend receiver input check")
        geometry = self.send({"kind": "connect"}, "connected")
        if not geometry["text_supported"]:
            raise RuntimeError("Authenticated Android receiver did not grant text support")
        if geometry["unicode_fixture"] != UNICODE_TEXT or geometry["unicode_utf16_units"] != len(UNICODE_TEXT.encode("utf-16-le")) // 2:
            raise RuntimeError("Host Unicode fixture differs from expected fixed public input")
        self.click_target("Remote click target", geometry); self.wait_target_label("Remote click received")
        self.click_target("Remote typing target", geometry)
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
            raise RuntimeError("Committed text produced no new editor change")
        save_private(self.output / "unicode-editor.json", text)
        observed = self.observe_target()
        typing = [node for node in observed if node.get("description") == "Remote typing target"]
        if len(typing) != 1 or typing[0].get("text") != expected:
            raise RuntimeError("Actual disposable editor accessibility value differs from received Unicode")
        save_private(self.output / "unicode-editor-ui.json", sanitize_nodes(observed))
        self.send({"kind": "release"})
        self.click_target("Remote click target", geometry); self.wait_target_label("Remote click received")
        nodes = self.observe_target()
        button = next(node for node in nodes if node.get("description") == "Remote click target")
        scroll = next(node for node in nodes if node.get("description") == "Remote scroll target")
        a, b, c, d = map(int, button["bounds"].split()); sa, sb, sc, sd = map(int, scroll["bounds"].split())
        self.send({"kind": "move", "x": (sa+sc)/2/geometry["width"], "y": (sb+sd)/2/geometry["height"]})
        self.send({"kind": "scroll"}); self.wait_target_state(lambda state: state.get("scroll", 0) > 0)
        time.sleep(1)  # Let observer detach before starting a continued contact.
        self.send({"kind": "hold", "x": (a+c)/2/geometry["width"], "y": (b+d)/2/geometry["height"]})
        self.wait_target_state(lambda state: state.get("touch") == "held")
        time.sleep(2)
        if self.target_state().get("touch") != "held": raise RuntimeError("Stationary held contact was released prematurely")
        self.send({"kind": "move", "x": ((a+c)/2+40)/geometry["width"], "y": (b+d)/2/geometry["height"]})
        self.wait_target_state(lambda state: state.get("moves", 0) > 0)
        self.send({"kind": "close"}, "closed"); self.process.wait(timeout=5)
        if self.process.returncode: raise RuntimeError("Disposable source did not close normally")
        ended = self.wait_target_state(lambda state: state.get("touch") == "released")
        assert_editor_text(ended, expected)
        save_private(self.output / "target-disconnect.json", ended)
        self.adb("shell", "am", "start", "-n", APP + "/.MainActivity"); self.wait_ui("Devices")
        self.preserve(self.public_state(), allow_fixture=True)
        self.remove_own_peer(self.name)
        final = self.public_state(); self.preserve(final)
        save_private(self.output / "public-state-after.json", final)

    def cleanup(self):
        errors = []
        if self.process and self.process.poll() is None:
            try:
                if self.process.stdin: self.process.stdin.close()
                self.process.wait(timeout=3)
            except (subprocess.TimeoutExpired, OSError):
                self.process.terminate()
                try: self.process.wait(timeout=3)
                except subprocess.TimeoutExpired: self.process.kill(); self.process.wait(timeout=3)
        if self.fixture: self.fixture.unlink(missing_ok=True)
        return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--tablet-peer", required=True, help="Physically verified native tablet fingerprint")
    parser.add_argument("--pair-address", help="Optional exact address observed on this tablet's code screen")
    parser.add_argument("--preserve-name", action="append", default=[], help="Existing visible peer names to retain")
    parser.add_argument("--peer-executable", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="New private receipt directory")
    args = parser.parse_args()
    if not FINGERPRINT.fullmatch(args.tablet_peer): parser.error("Invalid tablet fingerprint")
    if not args.peer_executable.is_file(): parser.error("Build the isolated host fixture first")
    os.umask(0o077)
    output = args.output.resolve(); output.mkdir(parents=True, exist_ok=False, mode=0o700)
    runner = ReceiverDriver(args.serial, output, args.peer_executable.resolve(), args.tablet_peer)
    result = {"status": "failed", "peer": None, "tablet_peer": args.tablet_peer}
    try:
        initial = runner.wait_ui("Devices")
        for name in args.preserve_name:
            if not any(node.get("text") == name for node in initial): raise RuntimeError("Required baseline peer is not visible")
        runner.pair(args.pair_address)
        result["peer"] = runner.peer
        runner.exercise()
        final = runner.ui()
        for name in args.preserve_name:
            if not any(node.get("text") == name for node in final): raise RuntimeError("Required baseline peer is no longer visible")
        result.update(status="passed", negotiated_text=runner.last_source["text_supported"], source=runner.last_source,
                      checks=["pinned-control", "physical-keys", "negotiated-text", "native-editor-exact-unicode",
                              "unicode-utf16-and-change-receipt", "actual-editor-accessibility-value", "release-keeps-control",
                              "actual-editor-input-focus-before-physical-keys",
                              "click-scroll-drag", "stationary-hold", "disconnect-releases-held-contact", "own-peer-only-unpair",
                              "baseline-public-state-and-settings-preserved"])
    except Exception as error:
        result["error"] = str(error) if isinstance(error, RuntimeError) else "Required receiver gate failed; raw details suppressed"
        result["source"] = runner.last_source
        result["peer"] = runner.peer
        result["failure_evidence"] = runner.capture_failure()
        if runner.peer:
            # End control before an ordinary UI cleanup; never alter another peer.
            runner.cleanup()
            try:
                runner.adb("shell", "am", "start", "-n", APP + "/.MainActivity")
                runner.remove_own_peer(runner.name)
            except Exception: result["own_peer_cleanup"] = "needs-root-review"
    finally:
        result["cleanup_errors"] = runner.cleanup()
        result["private_code_deleted"] = True
        save_private(output / "result.json", result)
    print(json.dumps({"status": result["status"], "output": str(output)}))
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
