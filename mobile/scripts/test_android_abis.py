"""Archive/ELF rejection regressions; fixtures are never loaded or executed."""
import importlib.util
from pathlib import Path
import struct
import tempfile
import unittest
import zipfile

spec = importlib.util.spec_from_file_location("verify", Path(__file__).with_name("verify-android-abis.py"))
verify = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verify)


def library(machine=183, public=True, defined=True, alignment=16384):
    data = bytearray(512)
    data[:16] = b"\x7fELF\x02\x01\x01" + b"\0" * 9
    struct.pack_into("<HHIQQQIHHHHHH", data, 16, 3, machine, 1, 0, 64, 128, 0, 64, 56, 1, 64, 3, 0)
    struct.pack_into("<IIQQQQQQ", data, 64, 1, 5, 0, 0, 0, 512, 512, alignment)
    struct.pack_into("<IIQQQQIIQQ", data, 192, 0, 11, 0, 0, 320, 24, 2, 0, 8, 24)
    names = b"\0" + verify.EXPORT.encode() + b"\0"
    struct.pack_into("<IIQQQQIIQQ", data, 256, 0, 3, 0, 0, 352, len(names), 0, 0, 1, 0)
    struct.pack_into("<IBBHQQ", data, 320, 1, 18, 0 if public else 2, 1 if defined else 0, 0, 0)
    data[352:352 + len(names)] = names
    return bytes(data)


class AbiTests(unittest.TestCase):
    def archive(self, entries, expected):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.apk"
            with zipfile.ZipFile(path, "w") as archive:
                for name, data in entries.items():
                    archive.writestr(name, data)
            return verify.verify_apk(path, expected)

    def test_both_architectures_have_real_dynamic_export(self):
        result = self.archive({"lib/arm64-v8a/libextend_android_engine.so": library(),
                               "lib/x86_64/libextend_android_engine.so": library(62)}, ["arm64-v8a", "x86_64"])
        self.assertEqual([row["elf_machine"] for row in result], [183, 62])

    def test_rn_present_but_engine_missing_rejected(self):
        with self.assertRaisesRegex(ValueError, "Missing engine"):
            self.archive({"lib/arm64-v8a/libextend_android_engine.so": library(),
                          "lib/x86_64/libreactnative.so": library(62)}, ["arm64-v8a", "x86_64"])

    def test_wrong_machine_rejected_even_under_right_zip_path(self):
        with self.assertRaisesRegex(ValueError, "architecture"):
            self.archive({"lib/x86_64/libextend_android_engine.so": library()}, ["x86_64"])

    def test_extra_packaged_abi_rejected(self):
        with self.assertRaisesRegex(ValueError, "ABI mismatch"):
            self.archive({"lib/arm64-v8a/libextend_android_engine.so": library(),
                          "lib/x86/libreactnative.so": b"ignored"}, ["arm64-v8a"])

    def test_undefined_or_hidden_symbol_not_delivery_proof(self):
        for data in (library(public=False), library(defined=False)):
            with self.subTest(data=data), self.assertRaisesRegex(ValueError, "export"):
                verify.verify_elf(data, "arm64-v8a")

    def test_truncation_and_4k_load_alignment_rejected(self):
        for data in (library()[:100], library(alignment=4096)):
            with self.subTest(data=data), self.assertRaises(ValueError):
                verify.verify_elf(data, "arm64-v8a")

    def test_requested_unsupported_or_duplicate_abi_rejected(self):
        for value in ("x86", "arm64-v8a,x86", "", "x86_64,x86_64"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                verify.architectures(value)


if __name__ == "__main__":
    unittest.main()
