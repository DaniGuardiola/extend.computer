#!/usr/bin/env python3
"""Check display-helper pipe framing without moving the pointer or injecting keys.

The target JSON must contain freshly observed display_id, bounds, canvas and
content. The signed helper must already have Accessibility permission. Only
alive and release commands are sent; no new permission is requested.
"""
import argparse
import json
import os
from pathlib import Path
import select
import subprocess
import time
import uuid


def line(process, buffer, timeout):
    deadline = time.monotonic() + timeout
    while b"\n" not in buffer:
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not select.select([process.stdout], [], [], remaining)[0]:
            raise TimeoutError("Helper did not acknowledge a complete command")
        chunk = os.read(process.stdout.fileno(), 512)
        if not chunk:
            raise RuntimeError("Helper closed before acknowledging the command")
        buffer.extend(chunk)
        if len(buffer) > 4096:
            raise RuntimeError("Oversized helper reply")
    end = buffer.index(b"\n")
    value = bytes(buffer[:end]).decode()
    del buffer[:end + 1]
    return value


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--helper", required=True)
    parser.add_argument("--target", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    target = json.loads(args.target.read_text())
    descriptor = {key: target[key] for key in ("display_id", "bounds", "canvas", "content")}
    descriptor.update(session=uuid.uuid4().hex, epoch=1, revision=0)
    binding = {key: descriptor[key] for key in ("session", "epoch", "revision")}
    receipt = {"status": "failed", "single_line_ack_ms": [], "commands": ["alive", "release"]}
    process = subprocess.Popen(
        [str(Path(args.helper).resolve()), "inject-display", json.dumps(descriptor)],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    buffer = bytearray()
    try:
        ready = line(process, buffer, 3)
        if not ready.startswith("READY ") or json.loads(ready[6:]) != descriptor:
            raise RuntimeError("Helper accepted a different display target")
        for sequence, kind in enumerate(receipt["commands"], 1):
            command = {**binding, "sequence": sequence, "event": {"kind": kind}}
            encoded = json.dumps(command).encode() + b"\n"
            if len(encoded) >= 256:
                raise RuntimeError("Fixture must fit below the old blocking read size")
            started = time.monotonic()
            process.stdin.write(encoded)
            process.stdin.flush()
            reply = line(process, buffer, 0.4)
            elapsed = (time.monotonic() - started) * 1000
            if not reply.startswith("APPLIED ") or json.loads(reply[8:]) != {**binding, "sequence": sequence}:
                raise RuntimeError("Helper returned the wrong acknowledgement")
            receipt["single_line_ack_ms"].append(elapsed)
        process.stdin.close()
        process.wait(timeout=2)
        if process.returncode != 0:
            raise RuntimeError("Helper failed to stop after EOF")
        receipt["status"] = "passed"
    except Exception as error:
        receipt["error"] = str(error)
        raise
    finally:
        if not process.stdin.closed:
            process.stdin.close()
        if process.poll() is None:
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                process.terminate()
                try:
                    process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
        receipt["exit_code"] = process.returncode
        receipt["stderr"] = process.stderr.read().decode(errors="replace")
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(receipt, indent=2) + "\n")
        print(json.dumps(receipt))


if __name__ == "__main__":
    main()
