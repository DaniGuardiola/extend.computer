#!/usr/bin/env python3
"""Run diagnostic-only consent, reconnect and revocation checks over a real LAN."""
import argparse
import queue
import re
import shlex
import subprocess
import threading
import time
import uuid


def lines(process):
    output = queue.Queue()
    def read():
        for line in process.stdout:
            output.put(line.rstrip())
        output.put(None)
    threading.Thread(target=read, daemon=True).start()
    return output


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--ssh-host", required=True)
    parser.add_argument("--peer-host", required=True)
    parser.add_argument("--key", required=True)
    parser.add_argument("--known-hosts", required=True)
    parser.add_argument("--remote-dir", required=True)
    parser.add_argument("--local-bin", required=True)
    parser.add_argument("--driver", required=True)
    args = parser.parse_args()
    ssh = ["ssh", "-o", "BatchMode=yes", "-o", "IdentitiesOnly=yes", "-o",
           "StrictHostKeyChecking=yes", "-o", f"UserKnownHostsFile={args.known_hosts}",
           "-o", "ConnectTimeout=8", "-i", args.key, args.ssh_host]
    state = ".test-state-" + uuid.uuid4().hex
    command = (f"cd {shlex.quote(args.remote_dir)} && echo EXTEND_COMPUTER_PID=$$ && exec "
               f"./target/debug/extend-computer --ephemeral --state {state} serve "
               "--bind 0.0.0.0:48177 --pair --advertise")
    server = subprocess.Popen(ssh + ["sh -c " + shlex.quote(command)], stdin=subprocess.PIPE,
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
    output = lines(server)
    pid = None
    client = None
    try:
        code = None
        deadline = time.monotonic() + 30
        while not code:
            line = output.get(timeout=max(0.01, deadline - time.monotonic()))
            if line is None:
                raise RuntimeError("remote server exited before opening pairing window")
            if line.startswith("EXTEND_COMPUTER_PID="):
                pid = int(line.split("=", 1)[1])
            match = re.search(r"Pair code: ([a-f0-9-]+)", line)
            if match:
                code = match.group(1)
            elif "Pair code" not in line:
                print("server:", line, flush=True)
        # Deliberate pairing with the selected test device. No production bypass flag.
        server.stdin.write("pair\n")
        server.stdin.flush()
        discovery = subprocess.run([args.local_bin, "discover", "--seconds", "8"],
                                   capture_output=True, text=True, timeout=15)
        print("discovery:", discovery.stdout.strip() or discovery.stderr.strip() or "no candidates", flush=True)
        assert discovery.returncode == 0 and "port=48177" in discovery.stdout, "discovery failed"
        client = subprocess.Popen([args.driver, f"{args.peer_host}:48177"], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
        client.stdin.write(code + "\n")
        client.stdin.flush()
        code = None  # Never persist or print the pairing code.
        client_output = lines(client)
        client_id = None
        passed = False
        deadline = time.monotonic() + 45
        while True:
            line = client_output.get(timeout=max(0.01, deadline - time.monotonic()))
            if line is None:
                break
            print(line, flush=True)
            if line.startswith("CLIENT_ID="):
                client_id = line.split("=", 1)[1]
                assert re.fullmatch(r"[0-9a-f]{64}", client_id)
            if line == "WAIT_REVOKE":
                assert client_id
                revoke = (f"cd {shlex.quote(args.remote_dir)} && ./target/debug/extend-computer "
                          f"--state {state} revoke {client_id}")
                subprocess.run(ssh + ["sh -c " + shlex.quote(revoke)], check=True, timeout=10)
                client.stdin.write("revoked\n")
                client.stdin.flush()
            if line.startswith("PASS:"):
                passed = True
        assert client.wait(timeout=5) == 0 and passed, "cross-Mac test failed"
    finally:
        if client and client.poll() is None:
            client.terminate()
            client.wait(timeout=5)
        if pid:
            subprocess.run(ssh + [f"kill -TERM {pid}"], timeout=10, check=False)
        if server.poll() is None:
            server.terminate()
        server.wait(timeout=5)


if __name__ == "__main__":
    main()
