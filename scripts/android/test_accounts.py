#!/usr/bin/env python3
"""Compile the native Android account client and test real loopback HTTP on a host JDK. No APK/device actions."""
import argparse
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
JSON_URL = "https://repo.maven.apache.org/maven2/org/json/json/20250517/json-20250517.jar"
JSON_SHA256 = "3ea61b2a06e31edf1c91134fe9106b0ebb16628be169f3db75bc7a2b06b45796"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", required=True, type=Path)
    parser.add_argument("--java-bin", required=True, type=Path)
    parser.add_argument("--output", type=Path, default=ROOT / "native/android/.local/account-host-tests")
    parser.add_argument("--test", choices=["accounts", "device-settings", "peer-address", "decoded-frame", "cursor-trace"], default="accounts")
    args = parser.parse_args()
    sdk, java, output = args.sdk.resolve(), args.java_bin.resolve(), args.output.resolve()
    platforms = sorted(sdk.glob("platforms/android-*/android.jar"), key=lambda jar: int(jar.parent.name.removeprefix("android-")))
    if not platforms or not (java / "javac").is_file() or not (java / "java").is_file():
        raise RuntimeError("An Android SDK platform and JDK 17 bin directory are required.")
    output.mkdir(parents=True, exist_ok=True)
    library = output / "json-20250517.jar"
    if not library.is_file():
        with urllib.request.urlopen(JSON_URL, timeout=30) as response:
            data = response.read(1024 * 1024 + 1)
        if hashlib.sha256(data).hexdigest() != JSON_SHA256:
            raise RuntimeError("Host-only JSON library checksum did not match.")
        library.write_bytes(data)
    if hashlib.sha256(library.read_bytes()).hexdigest() != JSON_SHA256:
        raise RuntimeError("Host-only JSON library checksum did not match.")
    with tempfile.TemporaryDirectory(prefix="classes-", dir=output) as temporary:
        classes = Path(temporary)
        sources = sorted((ROOT / "native/android/src").glob("**/*.java"))
        subprocess.run([str(java / "javac"), "-classpath", str(platforms[-1]), "-d", str(classes), *map(str, sources)], check=True)
        classpath = os.pathsep.join(map(str, [library, classes, platforms[-1]]))
        name = {"accounts": "AccountTest", "device-settings": "DeviceSettingsTest", "peer-address": "PeerAddressTest", "decoded-frame": "DecodedFrameGateTest", "cursor-trace": "CursorTraceTest"}[args.test]
        test = ROOT / f"native/android/test/computer/extend/android/{name}.java"
        tests = [test]
        if args.test == "peer-address": tests.append(ROOT / "native/android/test/computer/extend/android/DeviceAvailabilityTest.java")
        subprocess.run([str(java / "javac"), "--add-modules", "jdk.httpserver", "-classpath", classpath, "-d", str(classes), *map(str, tests)], check=True)
        subprocess.run([str(java / "java"), "--add-modules", "jdk.httpserver", "-classpath", classpath, f"computer.extend.android.{name}"], check=True)


if __name__ == "__main__":
    main()
