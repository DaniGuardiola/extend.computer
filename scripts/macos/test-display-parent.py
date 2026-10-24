#!/usr/bin/env python3
"""Verify native helper cleanup after both graceful and abrupt parent death."""
import argparse
import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import sys
import time


def running(pid):
    result = subprocess.run(['/bin/ps', '-p', str(pid), '-o', 'stat='],
                            capture_output=True, text=True)
    return result.returncode == 0 and not result.stdout.strip().startswith('Z')


def verify(helper, termination):
    # The child does not read stdin in this mode: EOF cannot masquerade as
    # parent-death detection. Readiness proves the monitor is armed first.
    code = '''import json,signal,subprocess,sys
child=subprocess.Popen([sys.argv[1],'parent-exit-test'],stdout=subprocess.PIPE,text=True)
if child.stdout.readline().strip()!='ready':raise RuntimeError('Helper not ready')
print(json.dumps({'pid':child.pid}),flush=True)
signal.pause()
'''
    parent = subprocess.Popen([sys.executable, '-c', code, str(helper)],
                              stdout=subprocess.PIPE, text=True)
    child = None
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(parent.stdout, selectors.EVENT_READ)
            if not selector.select(timeout=10):
                raise RuntimeError('Parent-exit fixture failed to become ready')
        child = json.loads(parent.stdout.readline())['pid']
        time.sleep(.6)
        if not running(child):
            raise RuntimeError('Helper exited while its parent was alive')
        started = time.monotonic()
        os.kill(parent.pid, termination)
        parent.wait(timeout=3)
        deadline = started + 2
        while running(child) and time.monotonic() < deadline:
            time.sleep(.02)
        if running(child):
            raise RuntimeError('Helper remained alive after parent death')
        elapsed = time.monotonic() - started
        print(f'{signal.Signals(termination).name}: helper exited in {elapsed:.3f}s')
    finally:
        if parent.poll() is None:
            parent.kill()
            parent.wait(timeout=3)
        if child and running(child):
            os.kill(child, signal.SIGKILL)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--helper', type=Path, default=Path(__file__).resolve().parents[2]
                        / 'target/extend.computer Display.app/Contents/MacOS/ExtendComputerDisplay')
    args = parser.parse_args()
    if not args.helper.is_file():
        parser.error('Build the native display helper first')
    for termination in (signal.SIGTERM, signal.SIGKILL):
        verify(args.helper, termination)
    print('Display parent-exit checks passed.')


if __name__ == '__main__':
    main()
