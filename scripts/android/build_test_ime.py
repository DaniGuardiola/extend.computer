#!/usr/bin/env python3
"""Build only the separate Unicode test IME; never installs or rebuilds the app."""
import argparse
from pathlib import Path
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[2]


def build(sdk, java_bin, output):
    sdk, java_bin, output = Path(sdk).resolve(), Path(java_bin).resolve(), Path(output).resolve()
    tools, jar = sdk / 'build-tools/36.0.0', sdk / 'platforms/android-36/android.jar'
    if not jar.is_file() or not (tools / 'lib/d8.jar').is_file():
        raise RuntimeError('Android platform/build-tools 36 are required')
    if not (java_bin / 'javac').is_file():
        raise RuntimeError('--java-bin must point to a JDK bin directory')
    output.mkdir(parents=True, exist_ok=False)
    classes, dex = output / 'classes', output / 'dex'
    classes.mkdir(); dex.mkdir()
    def run(command):
        subprocess.run([str(item) for item in command], check=True)
    source = ROOT / 'native/android/tests/unicode-ime'
    sources = sorted(source.glob('src/**/*.java'))
    run([java_bin / 'javac', '-source', '8', '-target', '8', '-classpath', jar, '-d', classes, *sources])
    run([java_bin / 'java', '-cp', tools / 'lib/d8.jar', 'com.android.tools.r8.D8', '--min-api', '29', '--lib', jar, '--output', dex, *sorted(classes.glob('**/*.class'))])
    resources = output / 'resources.zip'
    run([tools / 'aapt2', 'compile', '--dir', source / 'res', '-o', resources])
    unsigned, aligned, apk = output / 'unsigned.apk', output / 'aligned.apk', output / 'unicode-ime-test.apk'
    run([tools / 'aapt2', 'link', '-I', jar, '--manifest', source / 'AndroidManifest.xml', '-o', unsigned, resources])
    with zipfile.ZipFile(unsigned, 'a') as archive:
        archive.write(dex / 'classes.dex', 'classes.dex')
    run([tools / 'zipalign', '-f', '4', unsigned, aligned])
    run([java_bin / 'java', '-jar', tools / 'lib/apksigner.jar', 'sign', '--ks', ROOT / 'mobile/android/app/debug.keystore', '--ks-key-alias', 'androiddebugkey', '--ks-pass', 'pass:android', '--out', apk, aligned])
    run([java_bin / 'java', '-jar', tools / 'lib/apksigner.jar', 'verify', apk])
    print(apk)
    return apk


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sdk', type=Path, required=True)
    parser.add_argument('--java-bin', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True, help='Fresh output directory; existing files are never removed')
    args = parser.parse_args()
    build(args.sdk, args.java_bin, args.output)
