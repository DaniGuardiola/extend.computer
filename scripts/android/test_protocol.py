"""Development transport framing and authorization boundary tests."""
import importlib.util
import io
import json
from pathlib import Path
import struct
import unittest
from unittest.mock import patch
from subprocess import CompletedProcess

spec = importlib.util.spec_from_file_location("usb_display", Path(__file__).with_name("usb-display.py"))
bridge = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bridge)


class Fragmented(io.BytesIO):
    def read(self, count=-1):
        return super().read(min(count, 3))


class ProtocolTests(unittest.TestCase):
    def test_usb_selection_excludes_wireless_adb(self):
        listing = ("List of devices attached\n"
                   "tablet device usb:123 product:tablet transport_id:1\n"
                   "10.0.0.4:5555 device product:tablet transport_id:2\n"
                   "other unauthorized usb:456 transport_id:3\n")
        with patch.object(bridge.subprocess, "run", return_value=CompletedProcess([], 0, listing)):
            self.assertEqual(bridge.select_device(None), "tablet")
            self.assertEqual(bridge.select_device("tablet"), "tablet")
            for serial in ("10.0.0.4:5555", "other"):
                with self.assertRaises(RuntimeError):
                    bridge.select_device(serial)

    def test_fragmented_packet(self):
        payload = b"a" * 97
        self.assertEqual(bridge.read_packet(Fragmented(bridge.packet(1, payload))), (1, payload))

    def test_truncated_packet(self):
        for data in (b"\x01", bridge.HEADER.pack(1, 5) + b"abc"):
            with self.assertRaises(EOFError):
                bridge.read_packet(io.BytesIO(data))

    def test_size_rejected_before_payload_read(self):
        for size in (bridge.MAX_PACKET + 1, 0xFFFFFFFF):
            with self.assertRaises(ValueError):
                bridge.read_packet(io.BytesIO(struct.pack(">BI", 1, size)))

    def test_control_requires_object_and_correct_kind(self):
        for encoded in (bridge.packet(1, b"{}"), bridge.packet(2, b"[]")):
            with self.assertRaises(ValueError):
                bridge.control(io.BytesIO(encoded))

    def test_capability_token_and_version_required(self):
        token = "a" * 64
        valid = {"type": "hello", "version": 1, "token": token}
        bridge.authenticate(io.BytesIO(bridge.packet(2, json.dumps(valid).encode())), token)
        for replacement in ({"token": "b" * 64}, {"token": None}, {"version": 2}, {"type": "ready"}):
            value = {**valid, **replacement}
            with self.assertRaises(ValueError):
                bridge.authenticate(io.BytesIO(bridge.packet(2, json.dumps(value).encode())), token)


if __name__ == "__main__":
    unittest.main()
