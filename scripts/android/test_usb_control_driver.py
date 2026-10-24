"""Host-only USB acceptance guards; never invokes ADB or launches the app."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from test_usb_control_ui import (
    HASH_FILES, UNICODE_TEXT, UsbControlDriver, file_digest, gui_disconnected,
    save_private, target_point, validate_source,
)

TABLET, MAC = "a" * 64, "b" * 64


def granted():
    return {"status": "diagnostic-only", "route": "usb", "peer": TABLET, "local_identity": MAC,
            "global_input_capture": False, "native_delivery_verified": False,
            "unicode_fixture": UNICODE_TEXT, "unicode_utf16_units": 28, "command_sequence": 5,
            "state": "hold", "authenticated_peer": TABLET, "text_supported": True,
            "session": 4, "width": 1600, "height": 2560}


class UsbDriverTests(unittest.TestCase):
    def test_pinned_grant_route_and_fixed_sample(self):
        validate_source(granted(), TABLET, MAC, 4)
        for key, wrong in (("route", "lan"), ("authenticated_peer", MAC), ("local_identity", TABLET),
                           ("text_supported", False), ("session", 5), ("width", float("nan")),
                           ("global_input_capture", True), ("unicode_fixture", "arbitrary"),
                           ("command_sequence", True)):
            value = granted(); value[key] = wrong
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                validate_source(value, TABLET, MAC, 4)

    def test_gui_cancel_cannot_be_eof_or_stale_sequence(self):
        value = {**granted(), "state": "disconnected", "reason": "session-cancelled"}
        self.assertTrue(gui_disconnected(value, 5))
        for delta in ({"state": "closed", "reason": "stdin-eof"}, {"state": "expired"},
                      {"command_sequence": 4}, {"reason": "stdin-eof"}):
            self.assertFalse(gui_disconnected({**value, **delta}, 5))

    def test_only_exact_regular_file_sha_is_accepted(self):
        path = "no_backup/engine/trust/trust.json"
        raw = "d" * 64 + "  " + path + "\n"
        self.assertEqual(file_digest(raw, path), {"present": True, "sha256": "d" * 64})
        for wrong in ("ABSENT", "cat: Is a directory\n", raw.replace("trust/trust.json", "trust"),
                      raw + raw, "" ):
            with self.assertRaises(RuntimeError): file_digest(wrong, path)
        self.assertEqual(file_digest("ABSENT", HASH_FILES[-1]), {"present": False})
        with self.assertRaises(ValueError): file_digest(raw, "private-credentials.json")

    def test_coordinate_admission_uses_whole_observed_bounds(self):
        node = {"description": "Remote click target", "bounds": "40 100 1560 240"}
        self.assertEqual(target_point([node], node["description"], granted()), {"x": .5, "y": 170 / 2560})
        for nodes in ([node, node], [{**node, "bounds": "-1 100 1560 240"}],
                      [{**node, "bounds": "40 100 1601 240"}], []):
            with self.assertRaises(RuntimeError): target_point(nodes, node["description"], granted())

    def test_private_real_receipt_write_and_no_clobber(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "receipt.json"
            save_private(path, granted())
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            self.assertEqual(json.loads(path.read_text()), granted())
            with self.assertRaises(FileExistsError): save_private(path, {})

    def test_source_transition_retains_session_and_rejects_replacement(self):
        with tempfile.TemporaryDirectory() as directory:
            driver = UsbControlDriver("wired", Path(directory), Path("/test.app"), TABLET, MAC, "2717", "on")
            driver.receipt_path.write_text(json.dumps(granted()))
            self.assertEqual(driver.read_source()["session"], 4)
            value = {**granted(), "state": "disconnected", "reason": "session-cancelled"}
            driver.receipt_path.write_text(json.dumps(value))
            self.assertTrue(gui_disconnected(driver.read_source(), 5))
            driver.receipt_path.write_text(json.dumps({**value, "session": 9}))
            with self.assertRaises(RuntimeError): driver.read_source()

    def test_failed_preflight_does_not_mutate_device(self):
        with tempfile.TemporaryDirectory() as directory:
            driver = UsbControlDriver("wired", Path(directory), Path("/test.app"), TABLET, MAC, "2717", "on")
            with patch.object(driver, "adb", side_effect=AssertionError("Device mutation before ownership")):
                self.assertEqual(driver.cleanup(), [])


if __name__ == "__main__": unittest.main()
