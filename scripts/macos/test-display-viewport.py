#!/usr/bin/env python3
"""Verify a native viewer reports settled fullscreen content geometry.

Opens one synthetic viewer, without screen capture or a network connection,
then closes only that owned process. Run while the desktop is idle.
"""
import argparse
import json
import selectors
import struct
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('helper')
    args = parser.parse_args()
    child = subprocess.Popen(
        [args.helper, 'viewer', '1280', '800', '60', '20000000', 'retina', '0', 'hevc', 'cursor'],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    try:
        deadline = time.monotonic() + 8
        with selectors.DefaultSelector() as selector:
            selector.register(child.stdout, selectors.EVENT_READ)
            sequence = 0
            while time.monotonic() < deadline:
                sequence += 1
                payload = json.dumps({'type': 'viewport', 'id': sequence}).encode()
                child.stdin.write(bytes([2]) + struct.pack('>I', len(payload)) + payload)
                child.stdin.flush()
                while selector.select(timeout=max(0, deadline - time.monotonic())):
                    header = child.stdout.read(5)
                    if len(header) != 5:
                        raise RuntimeError('Viewer exited before reporting geometry')
                    count = struct.unpack('>I', header[1:])[0]
                    if not 0 < count <= 65536:
                        raise RuntimeError('Invalid native control packet')
                    message = json.loads(child.stdout.read(count))
                    if message.get('type') != 'viewport' or message.get('id') != sequence:
                        continue
                    if message['fullscreen']:
                        assert 640 <= message['width'] <= 16384
                        assert 480 <= message['height'] <= 16384
                        assert 0 < message['scale'] <= 4
                        print(json.dumps(message, sort_keys=True))
                        return
                    break
                time.sleep(.05)
        raise RuntimeError('Viewer never reported settled fullscreen geometry')
    finally:
        child.terminate()
        try:
            child.wait(timeout=3)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait()


if __name__ == '__main__':
    main()
