#!/usr/bin/env python3
"""Drive explicitly named Android UI through app-signed debug instrumentation.

Requires the development app and engine-test APK on the same authorized device.
Uses accessibility actions and explicit test-only touchscreen gestures. Password nodes are
excluded from output. The driver is test-only and absent from the mobile APK.
"""
import argparse
import json
import shlex
import subprocess
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--serial', required=True)
    parser.add_argument('--package', required=True, help='Only inspect or match nodes from this app')
    parser.add_argument('--action', choices=['dump', 'click', 'scroll', 'setText', 'gesture'], default='dump')
    parser.add_argument('--text', default='', help='Exact visible label or accessibility description')
    parser.add_argument('--value', default='', help='Replacement text for an editable control')
    parser.add_argument('--preserve-services', action='store_true', help='Use separate observer APK; retain accessibility services and target process')
    parser.add_argument('--gesture', choices=['tap', 'double-tap', 'hold-drag', 'two-finger-scroll', 'two-finger-tap', 'cancel-drag', 'home-drag'])
    parser.add_argument('--start', nargs=2, type=float, metavar=('X', 'Y'), help='Observed foreground-window touchscreen pixels')
    parser.add_argument('--end', nargs=2, type=float, metavar=('X', 'Y'))
    parser.add_argument('--gap', type=float, default=80, help='Second finger horizontal separation in pixels')
    parser.add_argument('--hold-ms', type=int, help='Hold before drag; defaults to platform long-press timeout + 150 ms')
    parser.add_argument('--duration-ms', type=int, default=350)
    parser.add_argument('--capture-dir', type=Path, help='New directory for before/after PNG and injection receipt; not remote-input evidence')
    args = parser.parse_args()
    if args.action in ('click', 'setText') and not args.text:
        parser.error('--text is required for click/setText')
    if args.action == 'gesture' and (not args.preserve_services or not args.gesture or not args.start):
        parser.error('gesture requires --preserve-services, --gesture and --start X Y')
    values = {'action': args.action, 'package': args.package, 'text': args.text, 'value': args.value}
    if args.action == 'gesture':
        values.update(gesture=args.gesture, x=str(args.start[0]), y=str(args.start[1]), gap=str(args.gap), durationMs=str(args.duration_ms))
        if args.end: values.update(endX=str(args.end[0]), endY=str(args.end[1]))
        if args.hold_ms is not None: values['holdMs'] = str(args.hold_ms)
    base = ['adb', '-s', args.serial]
    if args.capture_dir:
        args.capture_dir.mkdir(parents=True, exist_ok=False)
        with (args.capture_dir / 'before.png').open('wb') as image:
            subprocess.run(base + ['exec-out', 'screencap', '-p'], stdout=image, check=True, timeout=15)
    command = ['am', 'instrument', '-w']
    for key, value in values.items():
        command.extend(['-e', key, value])
    command.append('computer.extend.android.observer/computer.extend.android.tests.ObserverInstrumentation' if args.preserve_services else 'computer.extend.android.enginetest/computer.extend.android.tests.RobotInstrumentation')
    receipt = subprocess.check_output(['adb', '-s', args.serial, 'shell', shlex.join(command)], text=True, timeout=30)
    if args.capture_dir:
        (args.capture_dir / 'instrumentation.txt').write_text(receipt)
        with (args.capture_dir / 'after.png').open('wb') as image:
            subprocess.run(base + ['exec-out', 'screencap', '-p'], stdout=image, check=True, timeout=15)
    fields = dict(line.removeprefix('INSTRUMENTATION_RESULT: ').split('=', 1)
                  for line in receipt.splitlines() if line.startswith('INSTRUMENTATION_RESULT: ') and '=' in line)
    if args.capture_dir:
        (args.capture_dir / 'result.json').write_text(json.dumps(fields, indent=2) + '\n')
    if fields.get('status') != 'passed':
        raise RuntimeError(fields.get('error', 'Expected UI action was not available'))
    if args.action == 'dump':
        print(json.dumps(json.loads(fields['nodes']), indent=2))
    elif args.action == 'gesture':
        print(json.dumps({**fields, 'gestureEvents': json.loads(fields['gestureEvents'])}, indent=2))
    else:
        print('PASS: ' + args.action)


if __name__ == '__main__':
    main()
