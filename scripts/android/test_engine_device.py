#!/usr/bin/env python3
"""Build and run app-signed instrumentation against the installed development APK.

Never exports the device private key. A temporary ciphertext-tamper check restores
the original record in finally. Run only while display sessions are stopped.
"""
import argparse
import json
from pathlib import Path
import subprocess
import shlex
import shutil
import zipfile

ROOT = Path(__file__).resolve().parents[2]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--sdk', required=True, type=Path)
    p.add_argument('--java-bin', required=True, type=Path)
    p.add_argument('--serial', required=True)
    p.add_argument('--pair-fixture', type=Path, help='Private JSON emitted by tablet_pairing_peer example')
    p.add_argument('--discovery-name', help='Require native NSD discovery of this advertised peer; no ADB reverse tunnel')
    p.add_argument('--build-only', action='store_true', help='Build app-signed tests without installing or running')
    args = p.parse_args()
    fixture = json.loads(args.pair_fixture.read_text()) if args.pair_fixture else None
    if args.discovery_name and not fixture:
        p.error('--discovery-name requires --pair-fixture')
    tools = args.sdk / 'build-tools/36.0.0'
    jar = args.sdk / 'platforms/android-36/android.jar'
    output = ROOT / 'native/android/.local/engine-test'
    output.mkdir(parents=True, exist_ok=True)
    classes, dex = output / 'classes', output / 'dex'
    for generated in (classes, dex):
        if generated.exists():
            shutil.rmtree(generated)
    classes.mkdir(exist_ok=True); dex.mkdir(exist_ok=True)

    def run(command):
        subprocess.run([str(value) for value in command], check=True)

    manifest = output / 'AndroidManifest.xml'
    manifest.write_text('''<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="computer.extend.android.enginetest">
<uses-sdk android:minSdkVersion="29" android:targetSdkVersion="36"/>
<application android:label="Extend engine tests" android:testOnly="true" android:debuggable="true">
<activity android:name="computer.extend.android.tests.ReceiverTargetActivity" android:exported="true"/>
</application>
<instrumentation android:name="computer.extend.android.tests.EngineInstrumentation" android:targetPackage="computer.extend.android.development"/>
<instrumentation android:name="computer.extend.android.tests.RobotInstrumentation" android:targetPackage="computer.extend.android.development"/>
<instrumentation android:name="computer.extend.android.tests.ControlInstrumentation" android:targetPackage="computer.extend.android.development"/>
<instrumentation android:name="computer.extend.android.tests.ScreenSharingOptionsInstrumentation" android:targetPackage="computer.extend.android.development"/>
</manifest>''')
    run([args.java_bin/'javac', '-source', '8', '-target', '8', '-classpath', jar, '-d', classes,
         *sorted((ROOT/'native/android/tests').glob('*.java'))])
    run([args.java_bin/'java', '-cp', tools/'lib/d8.jar', 'com.android.tools.r8.D8', '--min-api', '29',
         '--lib', jar, '--output', dex, *sorted(classes.glob('**/*.class'))])
    run([tools/'aapt2', 'link', '-I', jar, '--manifest', manifest, '-o', output/'unsigned.apk'])
    with zipfile.ZipFile(output/'unsigned.apk', 'a') as archive:
        archive.write(dex/'classes.dex', 'classes.dex')
    run([tools/'zipalign', '-f', '4', output/'unsigned.apk', output/'aligned.apk'])
    apk = output/'engine-test.apk'
    run([args.java_bin/'java', '-jar', tools/'lib/apksigner.jar', 'sign', '--ks', ROOT/'mobile/android/app/debug.keystore',
         '--ks-key-alias', 'androiddebugkey', '--ks-pass', 'pass:android', '--out', apk, output/'aligned.apk'])
    # Separate package keeps observation from restarting the application or test target.
    observer_manifest=output/'ObserverManifest.xml'
    observer_manifest.write_text('''<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="computer.extend.android.observer">
<uses-sdk android:minSdkVersion="29" android:targetSdkVersion="36"/>
<application android:label="Extend UI observer" android:testOnly="true"/>
<instrumentation android:name="computer.extend.android.tests.ObserverInstrumentation" android:targetPackage="computer.extend.android.observer"/>
</manifest>''')
    run([tools/'aapt2','link','-I',jar,'--manifest',observer_manifest,'-o',output/'observer-unsigned.apk'])
    with zipfile.ZipFile(output/'observer-unsigned.apk','a') as archive: archive.write(dex/'classes.dex','classes.dex')
    run([tools/'zipalign','-f','4',output/'observer-unsigned.apk',output/'observer-aligned.apk'])
    run([args.java_bin/'java','-jar',tools/'lib/apksigner.jar','sign','--ks',ROOT/'mobile/android/app/debug.keystore',
         '--ks-key-alias','androiddebugkey','--ks-pass','pass:android','--out',output/'observer-test.apk',output/'observer-aligned.apk'])
    if args.build_only:
        print('Built app-signed test APK: ' + str(apk))
        return
    run(['adb', '-s', args.serial, 'install', '--no-streaming', '--no-incremental', '-r', '-t', apk])
    receipts = []
    for phase in ("first", "second"):
        run(['adb', '-s', args.serial, 'shell', 'am', 'force-stop', 'computer.extend.android.development'])
        command = ['adb', '-s', args.serial, 'shell', 'am', 'instrument', '-w']
        if fixture:
            for key, value in {'pairAddress': fixture['address'], 'pairCode': fixture['code'], 'pairPeer': fixture['peer'], 'pairPhase': phase}.items():
                command.extend(['-e', key, value])
        if args.discovery_name:
            command.extend(['-e', 'discoveryName', args.discovery_name])
        command.append('computer.extend.android.enginetest/computer.extend.android.tests.EngineInstrumentation')
        receipt = subprocess.check_output(command[:4] + [shlex.join(command[4:])], text=True, timeout=60)
        print(receipt)
        fields = dict(line.removeprefix('INSTRUMENTATION_RESULT: ').split('=', 1)
                      for line in receipt.splitlines() if line.startswith('INSTRUMENTATION_RESULT: ') and '=' in line)
        if fields.get('status') != 'passed':
            raise RuntimeError('Device engine instrumentation failed')
        if fixture and fields.get('pairing') != 'passed-' + phase:
            raise RuntimeError('Physical pairing interoperability check failed')
        if args.discovery_name and fields.get('discovery') != 'passed':
            raise RuntimeError('Native discovery verification failed')
        receipts.append(fields)
    if receipts[0]['fingerprint'] != receipts[1]['fingerprint']:
        raise RuntimeError('Identity did not survive a fresh application process')
    (output/'device-result.json').write_text(json.dumps({'status': 'passed', 'persistent_identity_verified': True,
        'receipts': receipts}, indent=2))
    print('PASS: installed native engine, Keystore tamper rejection and process-restart identity durability.')


if __name__ == '__main__':
    main()
