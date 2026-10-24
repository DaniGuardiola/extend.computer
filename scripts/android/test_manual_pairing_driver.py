"""Host-only invariants for the public-state-preserving manual UI driver."""
import importlib.util
from contextlib import redirect_stdout
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("manual_pairing_ui", Path(__file__).with_name("test_manual_pairing_ui.py"))
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)


class DriverTests(unittest.TestCase):
    def setUp(self):
        self.peer = "a" * 64
        self.fixture = "b" * 64
        self.before = {"trust": {"peers": {self.peer: {}}, "revoked": [], "pending_unpairs": [], "account_blocked": []},
                       "routes": {self.peer: "192.168.1.10"}, "names": {self.peer: "Existing Mac"},
                       "saved": {self.peer: {"address": "192.168.1.10:48177", "name": "Existing Mac"}}, "availability": True}

    def test_cancel_requires_exact_baseline_and_preserves_revocation(self):
        driver.assert_preserved(self.before, self.before)
        for section in ("routes", "names", "saved"):
            after = {**self.before, section: {**self.before[section], self.fixture: "unexpected"}}
            with self.assertRaises(RuntimeError): driver.assert_preserved(self.before, after)
        for field in ("peers", "revoked", "pending_unpairs", "account_blocked"):
            trust = {**self.before["trust"], field: {self.fixture: {}} if field == "peers" else [self.fixture]}
            with self.assertRaises(RuntimeError): driver.assert_preserved(self.before, {**self.before, "trust": trust})
        with self.assertRaises(RuntimeError): driver.assert_preserved(self.before, {**self.before, "availability": False})

    def test_success_allows_only_disposable_peer_then_cleanup_exact(self):
        after = {"trust": {**self.before["trust"], "peers": {**self.before["trust"]["peers"], self.fixture: {}}},
                 **{key: {**self.before[key], self.fixture: "temporary"} for key in ("routes", "names", "saved")}, "availability": True}
        driver.assert_preserved(self.before, after, self.fixture, allow_fixture=True)
        after["routes"][self.peer] = "wrong-route"
        with self.assertRaises(RuntimeError): driver.assert_preserved(self.before, after, self.fixture, allow_fixture=True)

    def test_sanitized_ui_excludes_pairing_codes_not_full_fingerprints(self):
        nodes = [{"description": "Pairing code", "text": "1234-abcd-5678-efab"},
                 {"text": "Code 0123456789abcdef", "description": ""},
                 {"text": self.peer, "description": "Device ID"}]
        clean = driver.sanitize_nodes(nodes)
        self.assertEqual(clean[0]["text"], "[redacted]")
        self.assertEqual(clean[1]["text"], "Code [redacted]")
        self.assertEqual(clean[2]["text"], self.peer)

    def test_native_dialog_uppercase_action_requires_exact_fixture_title_and_positive_button(self):
        nodes = [{"text": "Manual test abcd1234"},
                 {"text": "UNPAIR DEVICE", "resource": "android:id/button1"}]
        self.assertEqual(driver.native_dialog_action(nodes, "Manual test abcd1234", "Unpair device"), "UNPAIR DEVICE")
        with self.assertRaises(RuntimeError): driver.native_dialog_action(nodes, "Existing Mac", "Unpair device")
        with self.assertRaises(RuntimeError): driver.native_dialog_action([{**node, "resource": "android:id/button2"} for node in nodes], "Manual test abcd1234", "Unpair device")
        with self.assertRaises(RuntimeError): driver.native_dialog_action(nodes + [nodes[1]], "Manual test abcd1234", "Unpair device")
        confirmation = [{"text": "Unpair device?"}, {"text": "UNPAIR", "resource": "android:id/button1"}]
        self.assertEqual(driver.native_dialog_action(confirmation, "Unpair device?", "Unpair"), "UNPAIR")

    def test_real_private_receipt_and_marker_writes_under_permissive_umask(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            old_mask = os.umask(0)
            try:
                receipt = path / "result.json"
                driver.save_private(receipt, {"status": "failed", "case": "cancel"})
                self.assertEqual(json.loads(receipt.read_text()), {"status": "failed", "case": "cancel"})
                self.assertEqual(receipt.stat().st_mode & 0o777, 0o600)
                with self.assertRaises(FileExistsError): driver.save_private(receipt, {"status": "overwritten"})
                control = driver.Driver("not-used", "computer.extend.android.development", path, path / "not-used")
                control.fixture = path
                control.marker("stop")
                self.assertEqual((path / "stop").read_bytes(), b"")
                self.assertEqual((path / "stop").stat().st_mode & 0o777, 0o600)
                with self.assertRaises(FileExistsError): control.marker("stop")
            finally:
                os.umask(old_mask)

    def test_early_failure_still_writes_real_private_result_without_device_calls(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            executable = root / "fixture"
            executable.write_text("Not executed")
            output = root / "failed-run"
            arguments = ["test_manual_pairing_ui.py", "--serial", "not-used", "--case", "cancel",
                         "--peer-executable", str(executable), "--output", str(output)]
            old_mask = os.umask(0)
            try:
                with patch.object(sys, "argv", arguments), patch.object(driver.Driver, "begin", side_effect=RuntimeError("Simulated early gate failure")), \
                        patch.object(driver.Driver, "adb", side_effect=AssertionError("No device action allowed")), redirect_stdout(io.StringIO()):
                    self.assertEqual(driver.main(), 1)
                result = json.loads((output / "result.json").read_text())
                self.assertEqual(result["status"], "failed")
                self.assertEqual(result["error"], "Simulated early gate failure")
                self.assertEqual((output / "result.json").stat().st_mode & 0o777, 0o600)
                self.assertEqual(output.stat().st_mode & 0o777, 0o700)
            finally:
                os.umask(old_mask)


if __name__ == "__main__":
    unittest.main()
