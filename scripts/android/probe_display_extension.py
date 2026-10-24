#!/usr/bin/env python3
"""Build an ordinary-UID display probe; device execution is explicit and separate.

Uses only PUBLIC | OWN_CONTENT_ONLY, an app-owned ImageReader surface and a
disposable activity from another test package. No projection or shell permission
adoption. A successful probe establishes platform feasibility, not product parity.
"""
import argparse
import json
from pathlib import Path
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--sdk', type=Path, required=True)
parser.add_argument('--java-bin', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
args = parser.parse_args()
out = args.output.resolve()
out.mkdir(parents=True, exist_ok=False)
classes, dex = out/'classes', out/'dex'
classes.mkdir(); dex.mkdir()
tools = args.sdk/'build-tools/36.0.0'
jar = args.sdk/'platforms/android-36/android.jar'
manifest = out/'AndroidManifest.xml'
manifest.write_text('''<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="computer.extend.android.displayprobe">
<uses-sdk android:minSdkVersion="29" android:targetSdkVersion="36"/>
<queries><package android:name="computer.extend.android.enginetest"/></queries>
<application android:label="Extend display probe" android:testOnly="true">
<activity android:name="computer.extend.android.displayprobe.ProbeActivity" android:exported="false"/>
</application>
<instrumentation android:name="computer.extend.android.displayprobe.ProbeInstrumentation" android:targetPackage="computer.extend.android.displayprobe"/>
</manifest>''')
def run(command):
    subprocess.run([str(item) for item in command], check=True)
run([args.java_bin/'javac', '-source', '8', '-target', '8', '-classpath', jar, '-d', classes, *sorted((ROOT/'native/android/tests/display-probe').glob('*.java'))])
run([args.java_bin/'java', '-cp', tools/'lib/d8.jar', 'com.android.tools.r8.D8', '--min-api', '29', '--lib', jar, '--output', dex, *sorted(classes.glob('**/*.class'))])
run([tools/'aapt2', 'link', '-I', jar, '--manifest', manifest, '-o', out/'unsigned.apk'])
with zipfile.ZipFile(out/'unsigned.apk', 'a') as archive:
    archive.write(dex/'classes.dex', 'classes.dex')
run([tools/'zipalign', '-f', '4', out/'unsigned.apk', out/'aligned.apk'])
apk = out/'display-probe.apk'
run([args.java_bin/'java', '-jar', tools/'lib/apksigner.jar', 'sign', '--ks', ROOT/'mobile/android/app/debug.keystore', '--ks-key-alias', 'androiddebugkey', '--ks-pass', 'pass:android', '--out', apk, out/'aligned.apk'])
(out/'build.json').write_text(json.dumps({'status':'built', 'apk':str(apk), 'permissions':[], 'scope':'ordinary-UID feasibility only'}, indent=2))
print(apk)
