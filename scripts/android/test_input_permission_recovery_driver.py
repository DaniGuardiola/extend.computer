"""Host-only permission recovery gates. No ADB, subprocesses, or live input."""
from pathlib import Path
import tempfile
import unittest
import json
from unittest.mock import patch

import test_input_permission_recovery_ui as recovery


class RecoveryTests(unittest.TestCase):
    def test_removes_only_own_component_preserving_other_services_and_spelling(self):
        # The application ID has a development suffix; the service class does not.
        relative = recovery.APP + "/.InputReceivingService"
        for own in (recovery.OWN_SERVICE,):
            self.assertEqual(recovery.without_own_service("other.pkg/.Service:" + own + ":third.pkg/FullName"),
                             "other.pkg/.Service:third.pkg/FullName")
            self.assertEqual(recovery.without_own_service(own), "")
        for raw in (None, "", "other.pkg/.Service", recovery.OWN_SERVICE + ":" + recovery.OWN_SERVICE, relative,
                    "malformed:" + recovery.OWN_SERVICE):
            with self.assertRaises(RuntimeError): recovery.without_own_service(raw)

    def test_lifecycle_flag_is_written_before_source_process_launch(self):
        with tempfile.TemporaryDirectory() as folder:
            runner = recovery.PermissionRecoveryDriver("unused", Path(folder), Path("unused"), "a" * 64)
            runner.wait_ui = lambda label: []
            runner.ui = lambda *args: [{"text": "1234-abcd-5678-ef90"},
                                      {"description": "Pairing address 192.0.2.8:48179"}]
            runner.public_state = lambda: {"availability": True, "input_receiving": True}
            def launch(*args, **kwargs):
                fixture = json.loads((Path(folder) / "private-fixture.json").read_text())
                self.assertTrue(fixture["lifecycle"])
                self.assertEqual(fixture["peer"], "a" * 64)
                self.assertEqual((Path(folder) / "private-fixture.json").stat().st_mode & 0o777, 0o600)
                raise RuntimeError("Stop before launching any process")
            with patch("test_input_receiving_ui.subprocess.Popen", side_effect=launch):
                with self.assertRaisesRegex(RuntimeError, "Stop before"):
                    runner.pair(None, lifecycle=True)

    def test_denial_requires_exact_peer_reason_and_false_text_capability(self):
        state = {"peer": "b" * 64, "state": "denied", "denial": "remote-consent-denied", "text_supported": False}
        recovery.assert_denied(state, "b" * 64)
        for change in ({"peer": "c" * 64}, {"state": "disconnected"}, {"denial": "timeout"},
                       {"text_supported": True}, {"text_supported": None}):
            with self.assertRaises(RuntimeError): recovery.assert_denied({**state, **change}, "b" * 64)

    def test_restore_attempts_all_keys_even_after_first_failure_and_verifies(self):
        with tempfile.TemporaryDirectory() as folder:
            runner = recovery.PermissionRecoveryDriver("unused", Path(folder), Path("unused"), "a" * 64)
            runner.secure_baseline = {key: "original-" + key for key in recovery.SECURE_KEYS}
            current = {key: "changed" for key in recovery.SECURE_KEYS}
            attempts = []
            runner.secure_get = lambda key: current[key]
            def put(key, value):
                attempts.append(key)
                if key == recovery.SECURE_KEYS[0]: raise RuntimeError("simulated failure")
                current[key] = value
            runner.secure_set = put
            errors = runner.restore_preferences()
            self.assertEqual(attempts, list(recovery.SECURE_KEYS))
            self.assertIn("restore-enabled_accessibility_services-failed", errors)
            self.assertIn("restore-enabled_accessibility_services-mismatch", errors)
            self.assertEqual(current["default_input_method"], runner.secure_baseline["default_input_method"])
            self.assertFalse(any("awake" in key for key in attempts))

    def test_failure_evidence_precedes_cleanup_and_restore_even_when_cleanup_throws(self):
        with tempfile.TemporaryDirectory() as folder:
            runner = recovery.PermissionRecoveryDriver("unused", Path(folder), Path("unused"), "a" * 64)
            events = []
            runner.wait_ui = lambda label: []
            runner.capture_preferences = lambda: events.append("capture-preferences")
            def pair(address, lifecycle=False):
                self.assertTrue(lifecycle)
                events.append("pair-lifecycle")
                raise RuntimeError("deliberate pairing failure")
            runner.pair = pair
            runner.capture_failure = lambda: events.append("failure-evidence") or {"captured_before_cleanup": True}
            runner.secure_get = lambda key: "baseline"
            def cleanup():
                events.append("cleanup")
                raise RuntimeError("deliberate cleanup failure")
            runner.cleanup = cleanup
            runner.restore_preferences = lambda: events.append("restore") or []
            result = recovery.run(runner, None, [])
            self.assertEqual(events, ["capture-preferences", "pair-lifecycle", "failure-evidence", "cleanup", "restore"])
            self.assertEqual(result["status"], "failed")
            self.assertEqual(result["cleanup_errors"], ["source-cleanup-failed"])
            self.assertTrue((Path(folder) / "failure-system-preferences.json").is_file())
            self.assertEqual((Path(folder) / "result.json").stat().st_mode & 0o777, 0o600)

    def test_recovery_only_requires_native_balanced_contact_before_mutation(self):
        with tempfile.TemporaryDirectory() as folder:
            runner = recovery.PermissionRecoveryDriver("unused", Path(folder), Path("unused"), "a" * 64)
            events = []
            runner.start_target = lambda: None
            runner.send = lambda command, expected=None: events.append(command["kind"]) or {
                "text_supported": True, "unicode_fixture": recovery.UNICODE_TEXT,
                "unicode_utf16_units": len(recovery.UNICODE_TEXT.encode("utf-16-le")) // 2}
            runner.click_target = lambda label, geometry: events.append("actual-click")
            runner.wait_target_label = lambda label: None
            native = {"touch": "released"}
            def wait(predicate):
                if not predicate(native): raise RuntimeError("Native contact not released")
                return native
            runner.wait_target_state = wait
            runner.target_state = lambda: native
            def revoke():
                events.append("revoke")
                self.assertTrue((Path(folder) / "native-balanced-click-before-revoke.json").exists())
                raise RuntimeError("Stop before permission mutation")
            runner.remove_permission = revoke
            with self.assertRaisesRegex(RuntimeError, "Stop before"):
                runner.exercise_recovery(without_held_contact=True)
            self.assertEqual(events, ["connect", "actual-click", "revoke"])
            events.clear()
            native["touch"] = "held"
            with self.assertRaisesRegex(RuntimeError, "Native contact not released"):
                runner.exercise_recovery(without_held_contact=True)
            self.assertNotIn("revoke", events)
            self.assertNotIn("hold", events)

    def test_mode_results_never_claim_held_release_for_recovery_only(self):
        for without_held in (False, True):
            with tempfile.TemporaryDirectory() as folder:
                runner = recovery.PermissionRecoveryDriver("unused", Path(folder), Path("unused"), "a" * 64)
                runner.wait_ui = lambda label: []
                runner.capture_preferences = lambda: None
                runner.pair = lambda address, lifecycle=False: None
                observed_modes = []
                runner.exercise_recovery = lambda without_held_contact=False: observed_modes.append(without_held_contact)
                runner.cleanup = lambda: []
                runner.restore_preferences = lambda: []
                result = recovery.run(runner, None, [], without_held_contact=without_held)
                self.assertEqual(observed_modes, [without_held])
                self.assertEqual(result["status"], "passed")
                self.assertEqual(result["held_revocation_accepted"], not without_held)
                self.assertEqual(result["mode"], "recovery-only" if without_held else "strict-held-contact")
                self.assertEqual("native-contact-release" in result["checks"], not without_held)
                self.assertEqual("native-held-contact" in result["checks"], not without_held)
                self.assertEqual("native-balanced-click-before-revoke" in result["checks"], without_held)
                self.assertIn("same-identity-exact-reconnect-denial", result["checks"])

    def test_scoped_removal_never_disables_global_accessibility_or_ime(self):
        with tempfile.TemporaryDirectory() as folder:
            runner = recovery.PermissionRecoveryDriver("unused", Path(folder), Path("unused"), "a" * 64)
            current = {key: "baseline" for key in recovery.SECURE_KEYS}
            current["enabled_accessibility_services"] = "other.pkg/.Service:" + recovery.OWN_SERVICE
            runner.secure_baseline = current.copy()
            writes = []
            runner.secure_get = lambda key: current[key]
            def put(key, value): writes.append((key, value)); current[key] = value
            runner.secure_set = put
            runner.remove_permission()
            self.assertEqual(writes, [("enabled_accessibility_services", "other.pkg/.Service")])


if __name__ == "__main__": unittest.main()
