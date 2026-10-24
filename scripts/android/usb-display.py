#!/usr/bin/env python3
"""USB development bridge: existing macOS capture helper -> native Android decoder.

The listener is loopback-only and a fresh capability token authorizes one session.
ADB supplies the USB tunnel. This is not the engine's LAN/account protocol.
"""
import argparse
import hmac
import json
from pathlib import Path
import queue
import secrets
import shlex
import signal
import socket
import struct
import subprocess
import threading
import time

ROOT = Path(__file__).resolve().parents[2]
MAX_PACKET = 8 * 1024 * 1024
HEADER = struct.Struct(">BI")


def exact(reader, count):
    data = bytearray()
    while len(data) < count:
        chunk = reader.read(count - len(data))
        if not chunk:
            raise EOFError("Display stream ended")
        data.extend(chunk)
    return bytes(data)


def read_packet(reader, maximum=MAX_PACKET):
    kind, length = HEADER.unpack(exact(reader, HEADER.size))
    if length > maximum:
        raise ValueError("Oversized display packet")
    return kind, exact(reader, length)


def packet(kind, payload):
    if len(payload) > MAX_PACKET:
        raise ValueError("Oversized display packet")
    return HEADER.pack(kind, len(payload)) + payload


def control(reader):
    kind, payload = read_packet(reader, 4096)
    if kind != 2:
        raise ValueError("Expected display control packet")
    message = json.loads(payload)
    if not isinstance(message, dict):
        raise ValueError("Invalid display control object")
    return message


def authenticate(reader, token):
    hello = control(reader)
    offered = hello.get("token")
    if hello.get("type") != "hello" or hello.get("version") != 1 \
            or not isinstance(offered, str) or not hmac.compare_digest(offered, token):
        raise ValueError("Unauthorized USB display client")


def adb(serial, *arguments, timeout=30, quiet=False):
    # ADB joins shell arguments remotely; quote them before that join.
    if arguments and arguments[0] == "shell":
        arguments = ("shell", shlex.join(map(str, arguments[1:])))
    result = subprocess.run(["adb", "-s", serial, *map(str, arguments)], check=True,
                            capture_output=True, text=True, timeout=timeout)
    if not quiet and result.stdout.strip():
        print(result.stdout.strip(), flush=True)
    return result.stdout.strip()


def select_device(serial):
    result = subprocess.run(["adb", "devices", "-l"], check=True, capture_output=True, text=True)
    devices = [fields[0] for line in result.stdout.splitlines()[1:]
               if len(fields := line.split()) >= 2 and fields[1] == "device"
               and any(field.startswith("usb:") for field in fields[2:])]
    if serial:
        if serial not in devices:
            raise RuntimeError("Selected tablet is not authorized through physical USB ADB")
        return serial
    if len(devices) != 1:
        raise RuntimeError("Connect one authorized USB tablet or specify --serial")
    return devices[0]


def stream(connection, helper, args, report):
    connection.settimeout(15)
    reader = connection.makefile("rb", buffering=0)
    capture = None
    stopped = threading.Event()
    frames = queue.Queue(maxsize=2)
    failures = queue.Queue()
    input_lock = threading.Lock()
    counters = {"encoded": 0, "sent": 0, "dropped": 0}
    workers = []

    def helper_command(value):
        with input_lock:
            if capture and capture.poll() is None:
                capture.stdin.write((json.dumps(value) + "\n").encode())
                capture.stdin.flush()

    def source():
        waiting_keyframe = False
        try:
            while not stopped.is_set():
                kind, payload = read_packet(capture.stdout)
                if kind == 1:
                    if len(payload) <= 9 or payload[8] > 1:
                        raise ValueError("Malformed capture frame")
                    counters["encoded"] += 1
                    if waiting_keyframe and payload[8] == 0:
                        counters["dropped"] += 1
                        continue
                    try:
                        frames.put_nowait(payload)
                        waiting_keyframe = False
                    except queue.Full:
                        counters["dropped"] += 1
                        waiting_keyframe = True
                        helper_command({"type": "keyframe"})
                elif kind == 0:
                    print("Mac capture ready; virtual display active." if args.mode == "extend"
                          else "Mac screen capture ready.", flush=True)
                elif kind != 2:
                    raise ValueError("Unexpected capture packet")
        except Exception as error:
            if not stopped.is_set():
                failures.put(str(error))
                stopped.set()

    def feedback():
        try:
            while not stopped.is_set():
                message = control(reader)
                kind = message.get("type")
                if kind not in {"decoder", "rendered", "stats"}:
                    raise ValueError("Unexpected receiver feedback")
                report["receiver"] = message
                if kind == "decoder":
                    report["decoder"] = message
                    print("Android decoder: " + str(message.get("name")), flush=True)
                if kind == "rendered" and message.get("frames") == 1:
                    print("PASS: first Android decoder frame-rendered callback received.", flush=True)
                if args.report:
                    args.report.write_text(json.dumps(report, indent=2) + "\n")
        except Exception as error:
            if not stopped.is_set():
                failures.put(str(error))
                stopped.set()

    def diagnostics():
        # Drain stderr continuously, keeping only a bounded diagnostic tail.
        tail = bytearray()
        while True:
            chunk = capture.stderr.read(1024)
            if not chunk:
                break
            tail.extend(chunk)
            del tail[:-8192]
            report["capture_diagnostic"] = tail.decode(errors="replace").strip()

    try:
        authenticate(reader, args.token)
        config = {"width": args.width, "height": args.height, "fps": args.fps, "codec": args.codec}
        connection.sendall(packet(0, json.dumps(config).encode()))
        ready = control(reader)
        if ready.get("type") != "ready":
            raise RuntimeError("Android did not accept display configuration")
        report["config"] = {**config, "mode": args.mode, "bitrate": args.bitrate}
        report["ready"] = ready
        capture = subprocess.Popen([str(helper), args.mode, str(args.width), str(args.height),
                                    str(args.fps), str(args.bitrate), "standard", "0", args.codec, "embedded"],
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   bufsize=0)
        for job in (source, feedback, diagnostics):
            worker = threading.Thread(target=job, daemon=True)
            worker.start()
            workers.append(worker)
        started = time.monotonic()
        last_ping = 0
        while not stopped.is_set():
            elapsed = time.monotonic() - started
            if args.seconds and elapsed >= args.seconds:
                break
            if elapsed - last_ping >= 1:
                connection.sendall(packet(2, b'{"type":"ping"}'))
                last_ping = elapsed
            try:
                frame = frames.get(timeout=0.01)
            except queue.Empty:
                continue
            connection.sendall(packet(1, frame))
            counters["sent"] += 1
        if not failures.empty():
            raise RuntimeError(failures.get())
        connection.sendall(packet(2, b'{"type":"close"}'))
    finally:
        stopped.set()
        try:
            connection.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        connection.close()
        if capture:
            try:
                helper_command({"type": "stop"})
                capture.stdin.close()
                capture.wait(timeout=3)
            except (OSError, subprocess.TimeoutExpired):
                capture.kill()
                capture.wait()
            for pipe in (capture.stdout, capture.stderr):
                pipe.close()
        reader.close()
        for worker in workers:
            worker.join(timeout=1)
        report["source"] = counters
        if args.report:
            args.report.write_text(json.dumps(report, indent=2) + "\n")
        print("Display stopped; capture helper released.", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial")
    parser.add_argument("--receiver", choices=("react-native", "native"), default="react-native")
    parser.add_argument("--apk", type=Path)
    parser.add_argument("--peer-name", default=socket.gethostname().removesuffix(".local"))
    parser.add_argument("--helper", type=Path, default=ROOT / "target/extend.computer Display.app/Contents/MacOS/ExtendComputerDisplay")
    parser.add_argument("--mode", choices=("extend", "mirror"), default="extend")
    parser.add_argument("--width", type=int, default=1280)
    parser.add_argument("--height", type=int, default=800)
    parser.add_argument("--fps", type=int, choices=(30, 60, 120), default=60)
    parser.add_argument("--codec", choices=("hevc", "h264"), default="hevc")
    parser.add_argument("--bitrate", type=int, default=12_000_000)
    parser.add_argument("--seconds", type=int, default=0, help="0 runs until stopped")
    parser.add_argument("--report", type=Path, help="Numeric diagnostic JSON, not a physical latency benchmark")
    parser.add_argument("--skip-install", action="store_true")
    args = parser.parse_args()
    if args.apk is None:
        args.apk = ROOT / ("mobile/android/app/build/outputs/apk/release/app-release.apk"
                           if args.receiver == "react-native" else "native/android/.local/extend.computer.apk")
    activity = ("computer.extend.android.development/.MainActivity" if args.receiver == "react-native"
                else "computer.extend.android/.DisplayActivity")
    if not (640 <= args.width <= 4096 and 480 <= args.height <= 4096
            and args.width % 2 == 0 and args.height % 2 == 0
            and 1_000_000 <= args.bitrate <= 80_000_000 and args.seconds >= 0):
        parser.error("Invalid display configuration")
    if not args.helper.is_file() or not args.apk.is_file():
        parser.error("Build the native macOS display helper and Android APK first")
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        # Avoid overwriting evidence from another run.
        with args.report.open("x") as file:
            file.write("{}\n")
    serial = select_device(args.serial)
    if not args.skip_install:
        adb(serial, "install", "--no-incremental", "-r", args.apk, timeout=120)
    args.token = secrets.token_hex(32)
    report = {"transport": "adb-usb-loopback", "physical_benchmark": False, "status": "connecting"}
    mapping = None
    try:
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen(1)
            listener.settimeout(60)
            port = listener.getsockname()[1]
            selected_mapping = "tcp:" + str(port)
            adb(serial, "reverse", "--no-rebind", selected_mapping, selected_mapping, quiet=True)
            mapping = selected_mapping
            adb(serial, "shell", "am", "start", "-S", "-n", activity,
                "--ei", "port", port, "--es", "token", args.token,
                "--es", "peer", args.peer_name, "--es", "mode", args.mode, quiet=True)
            print("Waiting for authorized Android receiver over USB…", flush=True)
            connection, _ = listener.accept()
            report["status"] = "running"
            stream(connection, args.helper.resolve(), args, report)
            report["status"] = "completed"
    except KeyboardInterrupt:
        report["status"] = "interrupted"
        raise
    except Exception as error:
        report.update(status="failed", error=str(error))
        raise
    finally:
        if mapping:
            try:
                adb(serial, "reverse", "--remove", mapping, quiet=True)
            except (subprocess.SubprocessError, OSError):
                report["reverse_mapping_removed"] = False
            else:
                report["reverse_mapping_removed"] = True
        if args.report:
            args.report.write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, lambda *_: (_ for _ in ()).throw(KeyboardInterrupt()))
    try:
        main()
    except KeyboardInterrupt:
        pass
