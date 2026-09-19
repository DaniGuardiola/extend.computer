#!/usr/bin/env python3
"""Explicitly authorized development test; --input enables full control. No saved grants."""
import argparse
import os
from pathlib import Path
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
    for arg in ('ssh-host', 'peer-host', 'key', 'known-hosts', 'remote-dir', 'driver', 'helper'):
        parser.add_argument('--' + arg, required=True)
    parser.add_argument('--edge', choices=['left', 'right'], default='left')
    parser.add_argument('--offset-y', type=float, default=0)
    parser.add_argument('--low-jitter', action='store_true', help='Enable session-managed leases for synthetic diagnostic')
    parser.add_argument('--synthetic', action='store_true', help='Use dummy receiver; no native input APIs')
    parser.add_argument('--trace-dir', help='Save timing-only CSV files from both Macs')
    parser.add_argument('--input', action='store_true', help='Explicitly authorize mouse and keyboard control for this development test')
    parser.add_argument('--session', action='store_true', help='Full control until stopped; implies --input')
    parser.add_argument('--seconds', type=int, help='Optional run limit for --session')
    args = parser.parse_args()
    if args.seconds is not None and (not args.session or not 1 <= args.seconds <= 86400): parser.error('--seconds requires --session and 1..86400')
    if args.session: args.input = True
    if args.input and args.synthetic: parser.error('--input cannot use cursor-only synthetic driver')
    ssh = ['ssh', '-o', 'BatchMode=yes', '-o', 'IdentitiesOnly=yes', '-o', 'StrictHostKeyChecking=yes', '-o', f'UserKnownHostsFile={args.known_hosts}', '-o', 'ConnectTimeout=8', '-i', args.key, args.ssh_host]
    state = '.test-state-cursor-' + uuid.uuid4().hex
    helper = args.remote_dir + '/scripts/desktop-cursor-helper.py'
    trace_remote = args.remote_dir + '/' + state + '-timing.csv'
    variables = []
    if args.trace_dir: variables.append('EXTEND_COMPUTER_TIMING_PATH=' + shlex.quote(trace_remote))
    if args.low_jitter: variables.append('EXTEND_COMPUTER_TEST_LOW_JITTER=1')
    trace_env = ('env ' + ' '.join(variables) + ' ') if variables else ''
    if args.synthetic:
        command = f'cd {shlex.quote(args.remote_dir)} && echo EXTEND_COMPUTER_PID=$$ && exec {trace_env}./target/debug/examples/cursor_dummy serve'
    else:
        command = (f'cd {shlex.quote(args.remote_dir)} && echo EXTEND_COMPUTER_PID=$$ && exec {trace_env}./target/debug/extend-computer '
                   f'--ephemeral --state {state} serve --bind 0.0.0.0:48177 --pair --cursor-helper {shlex.quote(helper)}')
    server = subprocess.Popen(ssh + ['sh -c ' + shlex.quote(command)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
    output = lines(server)
    pid = None
    client = None
    try:
        code = None
        deadline = time.monotonic() + 15
        while not code:
            line = output.get(timeout=max(.01, deadline-time.monotonic()))
            if line is None: raise RuntimeError('receiver exited before pairing')
            if line.startswith('EXTEND_COMPUTER_PID='): pid = int(line.split('=')[1])
            match = re.search(r'Pair code: ([a-f0-9-]+)', line)
            if match: code = match.group(1)
            elif 'Pair code' not in line: print('receiver:', line, flush=True)
        # User explicitly selected this receiver and authorized this short test.
        if not args.synthetic:
            server.stdin.write('once\n' + ('allow-control\n' if args.session else 'allow-input\n' if args.input else 'allow-cursor\n')); server.stdin.flush()
        client_env = os.environ.copy()
        client_env.pop('EXTEND_COMPUTER_TEST_LOW_JITTER', None)
        if args.low_jitter: client_env['EXTEND_COMPUTER_TEST_LOW_JITTER'] = '1'
        if args.trace_dir:
            Path(args.trace_dir).mkdir(parents=True, exist_ok=True)
            client_env['EXTEND_COMPUTER_TIMING_PATH'] = str(Path(args.trace_dir).resolve() / 'sender.csv')
        driver_args = [args.driver, args.peer_host + ":48177", args.helper]
        if not args.synthetic: driver_args += [args.edge, str(args.offset_y)]
        if args.session: driver_args.append('session:' + str(args.seconds or 0))
        elif args.input: driver_args.append('input')
        client = subprocess.Popen(driver_args, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=client_env)
        client.stdin.write(code + '\n'); client.stdin.flush(); code = None
        client_output = lines(client)
        updates = 0
        deadline = (time.monotonic() + args.seconds + 15 if args.seconds else None) if args.session else time.monotonic() + 45
        while True:
            line = client_output.get(timeout=max(.01, deadline-time.monotonic()) if deadline is not None else None)
            if line is None: break
            print(line, flush=True)
            if line.startswith(('cursor_updates=', 'input_events=')): updates = int(line.split('=')[1])
        if client.wait(timeout=5) != 0:
            while not output.empty():
                line = output.get_nowait()
                if line is not None and 'Pair code' not in line: print('receiver:', line, flush=True)
            raise RuntimeError('cursor test failed')
        if args.trace_dir:
            # Wait for normal connection teardown to flush receiver's in-memory trace.
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                fetch = subprocess.run(ssh + ['cat ' + shlex.quote(trace_remote)], capture_output=True, timeout=5)
                if fetch.returncode == 0 and fetch.stdout:
                    (Path(args.trace_dir) / 'receiver.csv').write_bytes(fetch.stdout)
                    break
                time.sleep(.1)
            else: raise RuntimeError('receiver trace did not flush')
        if not args.session: assert updates > 0, 'no cursor movement captured; repeat with physical mouse movement'
        receiver_kind = 'dummy' if args.synthetic else 'native'
        event_kind = 'input events' if args.input else 'cursor updates'
        print(f'PASS: {updates} encrypted {event_kind} acknowledged by {receiver_kind} receiver', flush=True)
    finally:
        if client and client.poll() is None: client.terminate(); client.wait(timeout=5)
        if pid and server.poll() is None: subprocess.run(ssh + [f'kill -TERM {pid}'], timeout=10, check=False)
        if server.poll() is None: server.terminate()
        server.wait(timeout=5)

if __name__ == '__main__': main()
