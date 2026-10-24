#!/usr/bin/env python3
"""Exercise one outgoing manual-pairing lifecycle through the normal mobile UI.

Uses a disposable loopback peer and the already-installed separate UI observer.
Preserves existing pairings, availability, app data, process and accessibility
services. Only this run's temporary peer is removed through the normal UI.
Root/device owner must run this after installing a build with manual cancel.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import time
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
FINGERPRINT = re.compile(r"[0-9a-f]{64}")
CODE = re.compile(r"\b(?:[0-9a-f]{4}-){3}[0-9a-f]{4}\b|\b[0-9a-f]{16}\b", re.I)


def save_private(path, value):
    with open(path, "x", opener=lambda file, flags: os.open(file, flags, 0o600)) as output:
        json.dump(value, output, indent=2)
        output.write("\n")


def sanitize_nodes(nodes):
    return [{key: ("[redacted]" if key == "text" and node.get("description") == "Pairing code"
                   else CODE.sub("[redacted]", value) if isinstance(value, str) else value)
             for key, value in node.items()} for node in nodes]


def native_dialog_action(nodes, title, action):
    """Resolve Android's uppercase button only inside the expected dialog."""
    if not any(node.get("text") == title for node in nodes):
        raise RuntimeError("Expected scoped native dialog title missing")
    matches = [node.get("text") for node in nodes if node.get("resource") == "android:id/button1"
               and node.get("text", "").casefold() == action.casefold()]
    if len(matches) != 1:
        raise RuntimeError("Expected scoped native dialog action missing or ambiguous")
    return matches[0]


def assert_preserved(before, after, fixture_peer=None, allow_fixture=False):
    """Compare public persistent state; never reads a key or account credential."""
    for section in ("trust", "routes", "names", "saved", "availability"):
        expected, actual = before[section], after[section]
        if section == "trust":
            expected = {**expected, "peers": dict(expected.get("peers", {}))}
            actual = {**actual, "peers": dict(actual.get("peers", {}))}
            if allow_fixture:
                actual["peers"].pop(fixture_peer, None)
        elif allow_fixture and section != "availability":
            actual = {key: value for key, value in actual.items() if key != fixture_peer}
        if expected != actual:
            raise RuntimeError("Existing public " + section + " state changed")


class Driver:
    def __init__(self, serial, package, output, executable):
        self.base = ["adb", "-s", serial]
        self.package, self.output, self.executable = package, output, executable
        self.peer = self.fixture = self.process = self.port = self.name = None
        self.reverse_created = False
        self.baseline = None

    def adb(self, *arguments, timeout=30):
        result = subprocess.run(self.base + list(arguments), text=True, capture_output=True, timeout=timeout)
        if result.returncode:
            # Some commands carry a transient pairing code. Never include their
            # argv, stdout or stderr in exceptions or sanitized run receipts.
            raise RuntimeError("ADB command failed; raw details suppressed")
        return result.stdout

    def ui(self, action="dump", label="", value=""):
        command = ["am", "instrument", "-w", "-e", "action", action,
                   "-e", "package", self.package, "-e", "text", label, "-e", "value", value,
                   "computer.extend.android.observer/computer.extend.android.tests.ObserverInstrumentation"]
        output = self.adb("shell", shlex.join(command))
        fields = dict(line.removeprefix("INSTRUMENTATION_RESULT: ").split("=", 1)
                      for line in output.splitlines() if line.startswith("INSTRUMENTATION_RESULT: ") and "=" in line)
        if fields.get("status") != "passed":
            raise RuntimeError("Normal UI action was unavailable: " + action + " " + label)
        return json.loads(fields.get("nodes", "[]"))

    def wait_ui(self, label, timeout=30):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            nodes = self.ui()
            if any(label in (node.get("text"), node.get("description")) for node in nodes):
                return nodes
            time.sleep(.1)
        raise RuntimeError("Expected normal UI label missing: " + label)

    def dialog_action(self, title, action):
        self.wait_ui(title)
        self.ui("click", native_dialog_action(self.ui(), title, action))

    def read_file(self, path, default):
        # Fixed allowlisted public files only; no credential/key read command.
        allowed = {"no_backup/engine/trust/trust.json", "no_backup/engine/peer-hosts.json",
                   "no_backup/engine/peer-names.json", "shared_prefs/engine-peers.xml", "shared_prefs/engine-local.xml"}
        if path not in allowed:
            raise ValueError("Not a public test metadata path")
        command = ["run-as", self.package, "sh", "-c", f"if test -f {path}; then cat {path}; else printf '%s' {shlex.quote(default)}; fi"]
        return self.adb("shell", shlex.join(command))

    def public_state(self):
        trust = json.loads(self.read_file("no_backup/engine/trust/trust.json", '{"peers":{},"revoked":[]}'))
        if not isinstance(trust, dict) or not isinstance(trust.get("peers"), dict):
            raise RuntimeError("Unexpected public trust fixture format")
        # Normalize optional empty sets so adding/removing our disposable peer
        # cannot fail solely because serde elides an empty default field.
        trust = {**trust, **{key: trust.get(key, []) for key in ("revoked", "pending_unpairs", "account_blocked")}}
        saved = {item.attrib["name"]: json.loads(item.text or "{}") for item in
                 ET.fromstring(self.read_file("shared_prefs/engine-peers.xml", "<map/>")) if item.tag == "string"}
        local = ET.fromstring(self.read_file("shared_prefs/engine-local.xml", "<map/>"))
        availability = next((item.attrib.get("value") == "true" for item in local
                             if item.tag == "boolean" and item.attrib.get("name") == "receiving"), False)
        return {"trust": trust, "availability": availability,
                "routes": json.loads(self.read_file("no_backup/engine/peer-hosts.json", "{}")),
                "names": json.loads(self.read_file("no_backup/engine/peer-names.json", "{}")), "saved": saved}

    def wait_state(self, check, timeout=10):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            state = self.public_state()
            if check(state):
                return state
            time.sleep(.1)
        raise RuntimeError("Public local trust/route condition timed out")

    def wait_fixture(self, check, timeout=15):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            path = self.fixture / "status.json"
            if path.is_file():
                state = json.loads(path.read_text())
                if check(state):
                    return state
            if self.process.poll() is not None:
                raise RuntimeError("Disposable peer ended before the expected receipt")
            time.sleep(.05)
        raise RuntimeError("Disposable peer receipt timed out")

    def marker(self, name):
        if name not in ("grant", "deny", "stop"):
            raise ValueError("Unknown fixture action")
        with open(self.fixture / name, "x", opener=lambda file, flags: os.open(file, flags, 0o600)):
            pass

    def begin(self):
        self.wait_ui("Devices")
        self.baseline = self.public_state()
        save_private(self.output / "public-state-before.json", self.baseline)
        self.ui("click", "Pair device")
        self.wait_ui("Pair manually")
        self.ui("click", "Pair manually")
        self.wait_ui("Device address")
        self.fixture = self.output / "peer"
        log = open(self.output / "fixture.log", "x", opener=lambda file, flags: os.open(file, flags, 0o600))
        try:
            self.process = subprocess.Popen([str(self.executable), str(self.fixture)], stdout=log, stderr=log)
        finally:
            log.close()
        self.wait_fixture(lambda state: state["state"] == "ready")
        private_path = self.fixture / "private.json"
        if private_path.stat().st_mode & 0o077:
            raise RuntimeError("Pairing code file is not private")
        private = json.loads(private_path.read_text())
        self.peer = private["peer"]
        self.name = private["name"]
        if not FINGERPRINT.fullmatch(self.peer) or any(self.peer in self.baseline[key] for key in ("routes", "names", "saved")) or self.peer in self.baseline["trust"]["peers"]:
            raise RuntimeError("Disposable peer conflicts with existing state")
        self.port = private["address"].rsplit(":", 1)[1]
        self.adb("reverse", "--no-rebind", "tcp:" + self.port, "tcp:" + self.port)
        self.reverse_created = True
        self.ui("setText", "Device name", private["name"])
        self.ui("setText", "Device address", private["address"])
        self.ui("setText", "Pairing code", private["code"])
        self.ui("click", "Pair device")
        pending = self.wait_fixture(lambda state: state["pending_approval"] is True)
        if pending["eof_before_decision"] or pending["remote_trusted"]:
            raise RuntimeError("Pending approval was not an open, untrusted session")
        self.wait_ui("Cancel pairing")
        assert_preserved(self.baseline, self.public_state())
        save_private(self.output / "pending-ui.json", sanitize_nodes(self.ui()))
        save_private(self.output / "pending.json", pending)
        return private["name"]

    def cancel(self, mode):
        if mode == "cancel":
            self.ui("click", "Cancel pairing")
        elif mode == "back":
            self.ui("click", "Back")
        elif mode == "home":
            self.adb("shell", "input", "keyevent", "3")
        else:
            raise ValueError("Unknown cancellation case")
        eof = self.wait_fixture(lambda state: state["eof_before_decision"] is True, timeout=10)
        save_private(self.output / "eof-before-late-grant.json", eof)
        assert_preserved(self.baseline, self.public_state())
        self.marker("grant")
        terminal = self.wait_fixture(lambda state: state["state"] == "finished")
        if terminal["outcome"] != "granted-after-peer-eof" or not terminal["remote_trusted"]:
            raise RuntimeError("Fixture did not prove late remote approval after EOF")
        self.process.wait(timeout=5)
        if mode == "home":
            self.adb("shell", "am", "start", "-n", self.package + "/.MainActivity")
        if mode == "back":
            self.wait_ui("Pair manually")
            self.ui("click", "Pair manually")
        self.wait_ui("Pair manually")
        nodes = self.ui()
        if any(node.get("description") == "Cancel pairing" or node.get("text") == "Pairing…" for node in nodes):
            raise RuntimeError("Canceled pairing left a stale active UI")
        code_nodes = [node for node in nodes if node.get("description") == "Pairing code"]
        if len(code_nodes) != 1 or CODE.search(code_nodes[0].get("text", "")):
            raise RuntimeError("Canceled pairing retained the consumed code")
        # Wait through multiple normal poll intervals after the late grant.
        time.sleep(.7)
        self.wait_ui("Pair manually")
        assert_preserved(self.baseline, self.public_state())
        save_private(self.output / "returned-ui.json", sanitize_nodes(self.ui()))
        self.ui("click", "Back"); self.wait_ui("Pair a device")
        self.ui("click", "Back"); self.wait_ui("Devices")
        return terminal

    def retry(self, name):
        self.marker("grant")
        terminal = self.wait_fixture(lambda state: state["state"] == "finished")
        self.process.wait(timeout=5)
        if not terminal["remote_trusted"] or not terminal["clean_close"]:
            raise RuntimeError("Fresh-code retry did not complete the shared protocol")
        self.wait_ui(name)
        state = self.wait_state(lambda state: self.peer in state["trust"]["peers"] and self.peer in state["saved"] and self.peer in state["routes"])
        assert_preserved(self.baseline, state, self.peer, allow_fixture=True)
        save_private(self.output / "public-state-paired.json", state)
        self.wait_ui("Devices")
        save_private(self.output / "paired-ui.json", sanitize_nodes(self.ui()))
        return terminal

    def remove_own_peer(self, name):
        if self.peer and self.peer in self.public_state()["trust"]["peers"]:
            self.wait_ui("Devices")
            self.ui("click", "Settings for " + name)
            self.dialog_action(name, "Unpair device")
            self.dialog_action("Unpair device?", "Unpair")
            self.wait_state(lambda state: self.peer not in state["trust"]["peers"] and self.peer not in state["saved"] and self.peer not in state["routes"])

    def cleanup(self):
        errors = []
        if self.process and self.process.poll() is None:
            try:
                if not (self.fixture / "stop").exists(): self.marker("stop")
                self.process.wait(timeout=3)
            except (subprocess.TimeoutExpired, OSError):
                self.process.terminate()
                try: self.process.wait(timeout=3)
                except subprocess.TimeoutExpired: self.process.kill(); self.process.wait(timeout=3)
        if self.reverse_created:
            try: self.adb("reverse", "--remove", "tcp:" + self.port)
            except Exception: errors.append("own-reverse-removal-failed")
        if self.fixture:
            (self.fixture / "private.json").unlink(missing_ok=True)
        return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--package", default="computer.extend.android.development")
    parser.add_argument("--case", choices=("cancel", "back", "home", "retry"), required=True)
    parser.add_argument("--peer-executable", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="New private receipt directory")
    args = parser.parse_args()
    if args.package != "computer.extend.android.development":
        parser.error("This driver requires the app-signed development build")
    if not args.peer_executable.is_file():
        parser.error("Build the isolated host fixture first")
    os.umask(0o077)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False, mode=0o700)
    driver = Driver(args.serial, args.package, output, args.peer_executable.resolve())
    receipt, name = {"status": "failed", "case": args.case, "peer": None}, None
    try:
        name = driver.begin()
        receipt["peer"] = driver.peer
        terminal = driver.retry(name) if args.case == "retry" else driver.cancel(args.case)
        if args.case == "retry": driver.remove_own_peer(name)
        final = driver.public_state()
        assert_preserved(driver.baseline, final)
        save_private(output / "public-state-after.json", final)
        receipt.update(status="passed", remote=terminal, existing_state_preserved=True,
                       code_cleared=args.case != "retry", fresh_retry=args.case == "retry")
    except Exception as error:
        # Keep diagnostics bounded and sanitized. No subprocess repr/traceback
        # can accidentally reveal the private code passed to ACTION_SET_TEXT.
        receipt["error"] = str(error) if isinstance(error, RuntimeError) else "A required UI/protocol/state gate failed. Review the sanitized receipts."
        if driver.name and driver.peer:
            try: driver.remove_own_peer(driver.name)
            except Exception: receipt["own_peer_cleanup"] = "needs-root-review"
    finally:
        receipt["cleanup_errors"] = driver.cleanup()
        if receipt["cleanup_errors"]: receipt["status"] = "failed"
        receipt["private_code_deleted"] = True
        save_private(output / "result.json", receipt)
    print(json.dumps({"status": receipt["status"], "case": args.case, "output": str(output)}))
    return 0 if receipt["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
