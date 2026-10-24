"""Brightness rollback covers failures after partial device configuration."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import brightness


class BrightnessGuardTests(unittest.TestCase):
    def test_android_async_settings_must_settle_before_capture(self):
        original = {"mac": None, "android": [{"serial": "tablet"}]}
        with patch.object(brightness, "android", side_effect=["", "", "1", "8", "", "", "0", "255"]), patch.object(brightness.time, "sleep") as sleep:
            brightness.apply(original)
            sleep.assert_called_once_with(.1)

    def test_restores_after_command_failure(self):
        original = {"android": [], "mac": None}
        with tempfile.TemporaryDirectory() as directory:
            recovery = Path(directory) / "original.json"
            with patch.object(brightness, "snapshot", return_value=original), \
                    patch.object(brightness, "apply"), patch.object(brightness, "restore") as restore:
                with self.assertRaisesRegex(RuntimeError, "capture failed"):
                    with brightness.maximum([], False, recovery):
                        raise RuntimeError("capture failed")
                restore.assert_called_once_with(original)
                self.assertTrue(recovery.with_suffix(".restored.json").exists())

    def test_restores_partial_setup_failure(self):
        original = {"android": [], "mac": None}
        with tempfile.TemporaryDirectory() as directory:
            recovery = Path(directory) / "original.json"
            with patch.object(brightness, "snapshot", return_value=original), \
                    patch.object(brightness, "apply", side_effect=RuntimeError("second device failed")), \
                    patch.object(brightness, "restore") as restore:
                with self.assertRaisesRegex(RuntimeError, "second device failed"):
                    with brightness.maximum([], False, recovery):
                        self.fail("Benchmark must not start after setup failure")
                restore.assert_called_once_with(original)

    def test_original_snapshot_cannot_be_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            recovery = Path(directory) / "original.json"
            recovery.write_text("previous evidence")
            with patch.object(brightness, "snapshot", return_value={}), patch.object(brightness, "apply") as apply:
                with self.assertRaises(FileExistsError):
                    with brightness.maximum([], False, recovery):
                        pass
                apply.assert_not_called()
                self.assertEqual(recovery.read_text(), "previous evidence")


if __name__ == "__main__":
    unittest.main()
