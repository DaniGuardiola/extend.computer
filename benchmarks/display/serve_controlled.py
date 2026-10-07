#!/usr/bin/env python3
"""Serve a fixed local display workload and retain only allowed numeric metadata."""
import argparse
import datetime
import hashlib
import json
import math
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Lock
import re
import ipaddress
import time

ROOT = Path(__file__).resolve().parent
SCENES = {"static", "scroll", "panel", "motion"}
NUMBER_FIELDS = {"viewport_width", "viewport_height", "canvas_pixel_width", "canvas_pixel_height", "device_pixel_ratio", "elapsed_seconds", "source_callbacks", "source_callback_hz", "source_interval_p50_ms", "source_interval_p95_ms", "source_interval_max_ms", "source_intervals_over_33_33_ms"}
BOOL_FIELDS = {"measured", "hidden_during_phase", "resized_during_phase", "cancelled"}
TEXT_FIELDS = {"run_id", "scene", "started_utc"}
NULL_FIELDS = {"source_callback_hz", "source_interval_p50_ms", "source_interval_p95_ms", "source_interval_max_ms"}

def validate(body):
    if not isinstance(body, dict) or set(body) != {"kind", "data"} or body["kind"] not in {"phase_start", "phase_end"}:
        raise ValueError("Invalid event")
    data = body["data"]
    if not isinstance(data, dict) or set(data) - NUMBER_FIELDS - BOOL_FIELDS - TEXT_FIELDS:
        raise ValueError("Unexpected fields")
    required = {"run_id", "scene", "measured", "started_utc", "viewport_width", "viewport_height", "canvas_pixel_width", "canvas_pixel_height", "device_pixel_ratio"}
    if body["kind"] == "phase_end":
        required |= NUMBER_FIELDS | BOOL_FIELDS
    if required - set(data):
        raise ValueError("Missing fields")
    if not re.fullmatch(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}", data["run_id"]):
        raise ValueError("Invalid run ID")
    if data["scene"] not in SCENES or not re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z", data["started_utc"]):
        raise ValueError("Invalid phase metadata")
    for key in BOOL_FIELDS & data.keys():
        if not isinstance(data[key], bool):
            raise ValueError("Invalid flag")
    for key in NUMBER_FIELDS & data.keys():
        value = data[key]
        if value is None and key in NULL_FIELDS:
            continue
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or not 0 <= value <= 1e9:
            raise ValueError("Invalid numeric metadata")
    return body

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--host", default="127.0.0.1", help="Local IP to bind; loopback by default")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    address = ipaddress.ip_address(args.host)
    if address.version != 4 or not address.is_loopback:
        parser.error("Controlled workload server must bind loopback IPv4")
    parser.error("Invalid port") if not 0 < args.port < 65536 else None
    args.output.parent.mkdir(parents=True, exist_ok=True)
    workload = (ROOT / "workloads" / "display-baseline-v2.html").read_bytes()
    workload_hash = hashlib.sha256(workload).hexdigest()
    output = args.output.open("x", encoding="utf-8")
    lock = Lock()
    last_poll = [0.0]
    state = {"status": "idle", "run_id": None, "duration_s": 60, "completed": [], "ready": False, "stop": False}
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass  # Do not retain request URLs, headers, or addresses.
        def reply(self, code, data, content_type):
            self.send_response(code)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Cache-Control", "no-store")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.end_headers()
            self.wfile.write(data)
        def do_GET(self):
            if self.path in {"/", "/workload.html"}:
                self.reply(200, workload, "text/html; charset=utf-8")
            elif self.path == "/status":
                with lock: data = json.dumps({**state, "ready": time.monotonic() - last_poll[0] < 2}).encode()
                self.reply(200, data, "application/json")
            elif self.path == "/command":
                with lock:
                    state["ready"] = True
                    last_poll[0] = time.monotonic()
                    data = json.dumps({"stop": state["stop"], **({"run_id": state["run_id"], "duration_s": state["duration_s"]} if state["status"] == "pending" else {})}).encode()
                self.reply(200, data, "application/json")
            elif self.path in {"/collect.py", "/benchmark_core.py", "/summarize.py"}:
                self.reply(200, (ROOT / "sensors" / self.path[1:]).read_bytes(), "text/plain; charset=utf-8")
            else:
                self.reply(404, b"Not found", "text/plain")
        def do_POST(self):
            if self.path in {"/start", "/stop"}:
                try:
                    length = int(self.headers.get("Content-Length", "0"))
                    if not 0 < length <= 1024: raise ValueError("Invalid request size")
                    body = json.loads(self.rfile.read(length))
                    with lock:
                        if self.path == "/stop":
                            if body != {}: raise ValueError("Invalid stop")
                            state["stop"] = True
                            if state["status"] == "pending": state["status"] = "cancelled"
                        else:
                            if set(body) != {"run_id", "duration_s"}: raise ValueError("Invalid start")
                            if not isinstance(body["run_id"], str) or not re.fullmatch(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}", body["run_id"]): raise ValueError("Invalid run ID")
                            if isinstance(body["duration_s"], bool) or not isinstance(body["duration_s"], int) or not 1 <= body["duration_s"] <= 600: raise ValueError("Invalid duration")
                            if time.monotonic() - last_poll[0] >= 2 or state["status"] in {"pending", "running"} or body["run_id"] == state["run_id"]:
                                self.reply(409, b"Workload unavailable", "text/plain"); return
                            state.update(body, status="pending", completed=[], stop=False)
                    self.reply(200, b"OK", "text/plain")
                except (ValueError, TypeError, KeyError): self.reply(400, b"Invalid control request", "text/plain")
                return
            if self.path != "/metrics":
                self.reply(404, b"Not found", "text/plain")
                return
            try:
                length = int(self.headers.get("Content-Length", "0"))
                if not 0 < length <= 8192:
                    raise ValueError("Invalid length")
                record = validate(json.loads(self.rfile.read(length)))
                record = {"workload_version": "display-baseline-v2", "workload_sha256": workload_hash,
                          "received_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(), **record}
                with lock:
                    data = record["data"]
                    if state["run_id"] and data["run_id"] != state["run_id"]:
                        raise ValueError("Unexpected workload run")
                    if record["kind"] == "phase_start": state["status"] = "running"
                    if record["kind"] == "phase_end":
                        if data["cancelled"]: state["status"] = "cancelled"
                        elif data["measured"]:
                            if data["scene"] in state["completed"]: raise ValueError("Duplicate measured phase")
                            state["completed"].append(data["scene"])
                            if len(state["completed"]) == 4: state["status"] = "complete"
                    output.write(json.dumps(record, allow_nan=False) + "\n")
                    output.flush()
                self.reply(200, b"OK", "text/plain")
            except (ValueError, TypeError, KeyError, json.JSONDecodeError):
                self.reply(400, b"Invalid metadata", "text/plain")
    server = ThreadingHTTPServer((args.host, args.port), Handler)
    print(f"Workload: http://{args.host}:{args.port}/workload.html", flush=True)
    print(f"SHA-256: {workload_hash}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
        output.close()

if __name__ == "__main__":
    main()
