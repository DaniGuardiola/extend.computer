#!/usr/bin/env python3
"""Run the approved native helper in the user's desktop session, not SSH's TCC context.
Private FIFOs carry cursor coordinates; launchd job and files live for one invocation.
"""
import os
from pathlib import Path
import plistlib
import select
import subprocess
import sys
import tempfile
import time
import uuid


def main():
    if sys.argv[1:] not in (['inject'], ['inject-input'], ['inject-control']):
        raise SystemExit('desktop helper supports inject only')
    native = Path(__file__).resolve().parents[1] / 'target/extend.computer Cursor.app/Contents/MacOS/ExtendComputerCursor'
    label = 'computer.extend.prototype.cursor-session.' + uuid.uuid4().hex
    domain = f'gui/{os.getuid()}'
    with tempfile.TemporaryDirectory(prefix='extend-computer-cursor-') as directory:
        root = Path(directory)
        source = root / 'input'; sink = root / 'output'; errors = root / 'errors'
        os.mkfifo(source, 0o600); os.mkfifo(sink, 0o600)
        incoming = os.open(source, os.O_RDWR | os.O_NONBLOCK)
        outgoing = os.open(sink, os.O_RDWR | os.O_NONBLOCK)
        config = root / 'job.plist'
        config.write_bytes(plistlib.dumps({'Label': label, 'ProgramArguments': [str(native), sys.argv[1]], 'RunAtLoad': True, 'StandardInPath': str(source), 'StandardOutPath': str(sink), 'StandardErrorPath': str(errors)}))
        started = False
        try:
            subprocess.run(['launchctl', 'bootstrap', domain, str(config)], check=True)
            started = True
            deadline = float("inf") if sys.argv[1] == "inject-control" else time.monotonic() + 35
            while time.monotonic() < deadline:
                if errors.exists() and errors.stat().st_size:
                    sys.stderr.write(errors.read_text())
                    return 1
                ready, _, _ = select.select([0, outgoing], [], [], .1)
                for descriptor in ready:
                    packet = os.read(descriptor, 4096)
                    if not packet:
                        return 0
                    destination = incoming if descriptor == 0 else 1
                    # Protocol sends a single short position, then waits for OK.
                    while packet:
                        written = os.write(destination, packet)
                        packet = packet[written:]
            sys.stderr.write('Desktop cursor helper timed out.\n')
            return 1
        finally:
            if started:
                subprocess.run(['launchctl', 'bootout', f'{domain}/{label}'], capture_output=True)
            os.close(incoming); os.close(outgoing)


if __name__ == '__main__':
    raise SystemExit(main())
