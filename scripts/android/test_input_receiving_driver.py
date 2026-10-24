"""Host-only receiving fixture invariants; never calls ADB or launches a target."""
import importlib.util
import json
from pathlib import Path
import re
import tempfile
import os
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET

spec = importlib.util.spec_from_file_location("input_receiving_ui", Path(__file__).with_name("test_input_receiving_ui.py"))
receiver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(receiver)


class ReceiverDriverTests(unittest.TestCase):
    def test_editor_receipt_requires_exact_unicode_and_actual_utf16_delivery(self):
        expected = "extend" + receiver.UNICODE_TEXT
        units = len(expected.encode("utf-16-le")) // 2
        state = {"text": expected, "text-utf16": units, "text-changes": 7}
        receiver.assert_editor_text(state, expected)
        self.assertGreater(units, 20)
        self.assertGreater(units, len(expected))
        for changed in ({"text": expected[:20]}, {"text-utf16": len(expected)}, {"text-changes": 0}):
            with self.assertRaises(RuntimeError): receiver.assert_editor_text({**state, **changed}, expected)
        native = (receiver.ROOT / "native/android/engine/examples/tablet_input_source.rs").read_text()
        self.assertEqual(re.search(r'const UNICODE_TEXT: &str = "([^"]+)";', native).group(1), receiver.UNICODE_TEXT)

    def test_disposable_editor_xml_retains_unicode_and_types(self):
        root = ET.Element("map")
        ET.SubElement(root, "string", name="text").text = receiver.UNICODE_TEXT
        ET.SubElement(root, "int", name="text-utf16", value=str(len(receiver.UNICODE_TEXT.encode("utf-16-le")) // 2))
        ET.SubElement(root, "int", name="text-changes", value="1")
        ET.SubElement(root, "boolean", name="focused", value="true")
        parsed = receiver.target_preferences(ET.tostring(root, encoding="unicode"))
        receiver.assert_editor_text(parsed, receiver.UNICODE_TEXT)
        self.assertTrue(parsed["focused"])

    def test_code_address_selection_uses_only_current_observed_nodes(self):
        nodes = [{"text": "1234-abcd-5678-ef90"}, {"description": "Pairing address [::1]:12345"},
                 {"description": "Pairing address 192.168.1.20:12345"}]
        self.assertEqual(receiver.fixture_fields(nodes), ("1234-abcd-5678-ef90", "192.168.1.20:12345"))
        self.assertEqual(receiver.fixture_fields(nodes, "[::1]:12345")[1], "[::1]:12345")
        with self.assertRaises(RuntimeError): receiver.fixture_fields(nodes, "192.168.1.99:12345")
        with self.assertRaises(RuntimeError): receiver.fixture_fields(nodes + [{"text": "abcd-0123-4567-8901"}])

    def test_same_stage_old_receipt_cannot_ack_replacement_command(self):
        with tempfile.TemporaryDirectory() as directory:
            run = receiver.ReceiverDriver("not-used", Path(directory), Path(directory) / "not-used", "a" * 64)
            run.peer = "b" * 64
            run.process = type("NoProcess", (), {"poll": lambda self: None})()
            old = {"state": "click", "peer": run.peer, "command_sequence": 1, "text_supported": True}
            run.receipt_path.write_text(json.dumps(old))
            def update(_delay): run.receipt_path.write_text(json.dumps({**old, "command_sequence": 2}))
            with patch.object(receiver.time, "sleep", side_effect=update):
                result = run.source_state("click", sequence=2)
            self.assertEqual(result["command_sequence"], 2)
            self.assertTrue(result["text_supported"])

    def test_fast_start_preserves_identity_without_requiring_transient_waiting(self):
        with tempfile.TemporaryDirectory() as directory:
            run = receiver.ReceiverDriver("not-used", Path(directory), Path(directory) / "not-used", "a" * 64)
            run.baseline = {"trust": {"peers": {}}}
            run.process = type("NoProcess", (), {"poll": lambda self: None})()
            state = {"state": "paired", "stage": "paired-waiting-target", "peer": "b" * 64,
                     "name": "Input test bbbbbbbb", "command_sequence": 0}
            run.receipt_path.write_text(json.dumps(state))
            self.assertEqual(run.source_state(None), state)
            self.assertEqual(run.peer, state["peer"])
            self.assertEqual(run.name, state["name"])

    def test_failure_receipt_reports_boundary_address_not_arbitrary_private_errors(self):
        with tempfile.TemporaryDirectory() as directory:
            run = receiver.ReceiverDriver("not-used", Path(directory), Path(directory) / "not-used", "a" * 64)
            run.baseline = {"trust": {"peers": {}}}
            # Final receipt must be read even after the process has exited.
            run.process = type("DeadProcess", (), {"poll": lambda self: 1})()
            state = {"state": "failed", "stage": "failed", "failure_stage": "pairing-handshake-ready",
                     "peer": "b" * 64, "name": "Input test bbbbbbbb", "command_sequence": 0,
                     "selected_pairing_address": "10.0.100.201:40983", "error_kind": "TimedOut",
                     "error": "private pairing code must never escape"}
            run.receipt_path.write_text(json.dumps(state))
            with self.assertRaises(RuntimeError) as error: run.source_state(None)
            self.assertIn("pairing-handshake-ready", str(error.exception))
            self.assertIn("10.0.100.201:40983", str(error.exception))
            self.assertIn("TimedOut", str(error.exception))
            self.assertNotIn("private pairing code", str(error.exception))
            self.assertEqual(run.last_source, state)
            self.assertEqual(run.peer, state["peer"])
            malformed = {**state, "failure_stage": "1234-abcd-5678-ef90", "selected_pairing_address": "private-code"}
            self.assertNotIn("1234-abcd", receiver.source_failure(malformed))
            self.assertNotIn("private-code", receiver.source_failure(malformed))

    def test_editor_focus_requires_exact_editor_input_focus_then_saves_actual_snapshot(self):
        editor = {"description": "Remote typing target", "focused": False, "text": ""}
        self.assertFalse(receiver.editor_focused([editor, {"description": "other", "focused": True}]))
        self.assertFalse(receiver.editor_focused([{**editor, "focused": "true"}]))
        self.assertFalse(receiver.editor_focused([]))
        with self.assertRaises(RuntimeError): receiver.editor_focused([editor, editor])
        with tempfile.TemporaryDirectory() as directory:
            run = receiver.ReceiverDriver("not-used", Path(directory), Path(directory) / "not-used", "a" * 64)
            run.process = type("NoProcess", (), {"poll": lambda self: None})()
            snapshots = iter([[editor], [{**editor, "focused": True}]])
            run.read_source = lambda: None
            run.observe_target = lambda: next(snapshots)
            with patch.object(receiver.time, "sleep"):
                run.wait_editor_focus()
            path = Path(directory) / "editor-focused-before-type.json"
            self.assertTrue(receiver.editor_focused(json.loads(path.read_text())))
            self.assertEqual(os.stat(path).st_mode & 0o777, 0o600)

    def test_failure_capture_keeps_native_focus_and_input_preference_delta_without_restoring(self):
        with tempfile.TemporaryDirectory() as directory:
            run = receiver.ReceiverDriver("not-used", Path(directory), Path(directory) / "not-used", "a" * 64)
            run.peer = "b" * 64
            run.baseline = {"trust": {"peers": {}}, "names": {}, "saved": {}, "routes": {},
                            "availability": True, "input_receiving": True}
            observed = {**run.baseline, "input_receiving": False}
            run.public_state = lambda: observed
            run.target_state = lambda: {"touch": "released"}
            run.observe_target = lambda: [{"description": "Remote typing target", "focused": False}]
            terminal = {"state": "failed", "peer": run.peer, "stage": "failed", "failure_stage": "control-active"}
            run.receipt_path.write_text(json.dumps(terminal))
            evidence = run.capture_failure()
            self.assertEqual(evidence["status"], "failed")
            self.assertTrue(evidence["captured_before_cleanup"])
            self.assertEqual(evidence["input_receiving"], {"before": True, "observed": False})
            self.assertFalse(evidence["baseline_preserved_allowing_only_fixture"])
            self.assertFalse(evidence["editor_focused"])
            self.assertEqual(evidence["source"], terminal)
            self.assertEqual(json.loads((Path(directory) / "failure-target-native.json").read_text()), {"touch": "released"})
            self.assertEqual(observed["input_receiving"], False)


if __name__ == "__main__":
    unittest.main()
