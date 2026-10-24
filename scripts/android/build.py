#!/usr/bin/env python3
"""Build the local Android display receiver with an Android SDK and JDK (no Gradle)."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "native/android"
OUTPUT = SOURCE / ".local"


def build(sdk, java):
    sdk, java = Path(sdk).resolve(), Path(java).resolve()
    tools = sdk / "build-tools/35.0.0"
    if not tools.is_dir():
        tools = sdk / "android-15"
    jar = sdk / "platforms/android-35/android.jar"
    if not jar.is_file():
        jar = sdk / "android-35/android.jar"
    if not jar.is_file() or not (tools / "lib/d8.jar").is_file():
        raise RuntimeError("Android platform 35 and build-tools 35.0.0 are required.")
    if not (java / "javac").is_file():
        raise RuntimeError("--java-bin must point to a JDK bin directory.")
    OUTPUT.mkdir(parents=True, exist_ok=True)
    for name in ("classes", "dex"):
        shutil.rmtree(OUTPUT / name, ignore_errors=True)
        (OUTPUT / name).mkdir()

    def run(args):
        subprocess.run([str(arg) for arg in args], check=True)

    for executable in ("aapt2", "zipalign"):
        (tools / executable).chmod((tools / executable).stat().st_mode | 0o100)
    run([java / "javac", "-source", "8", "-target", "8", "-classpath", jar,
         "-d", OUTPUT / "classes", *sorted(SOURCE.glob("src/**/*.java"))])
    run([java / "java", "-cp", tools / "lib/d8.jar", "com.android.tools.r8.D8",
         "--min-api", "29", "--lib", jar, "--output", OUTPUT / "dex",
         *sorted(OUTPUT.glob("classes/**/*.class"))])
    run([tools / "aapt2", "link", "-I", jar, "--manifest", SOURCE / "AndroidManifest.xml",
         "-o", OUTPUT / "unsigned.apk"])
    with zipfile.ZipFile(OUTPUT / "unsigned.apk", "a") as archive:
        archive.write(OUTPUT / "dex/classes.dex", "classes.dex")
    run([tools / "zipalign", "-f", "4", OUTPUT / "unsigned.apk", OUTPUT / "aligned.apk"])
    key = OUTPUT / "debug.keystore"
    if not key.exists():
        run([java / "keytool", "-genkeypair", "-keystore", key, "-storepass", "android",
             "-keypass", "android", "-alias", "local", "-keyalg", "RSA", "-validity", "3650",
             "-dname", "CN=extend.computer Android Development"])
        key.chmod(0o600)
    apk = OUTPUT / "extend.computer.apk"
    run([java / "java", "-jar", tools / "lib/apksigner.jar", "sign", "--ks", key,
         "--ks-key-alias", "local", "--ks-pass", "pass:android", "--out", apk, OUTPUT / "aligned.apk"])
    run([java / "java", "-jar", tools / "lib/apksigner.jar", "verify", apk])
    print(apk)
    return apk


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", required=True, type=Path)
    parser.add_argument("--java-bin", required=True, type=Path)
    args = parser.parse_args()
    build(args.sdk, args.java_bin)
