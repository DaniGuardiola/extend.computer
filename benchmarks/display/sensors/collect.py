#!/usr/bin/env python3
"""Collect sanitized TCP flow and PID resource metadata for a configured port.

No debugger, active network probe, payload read, log read, or UI automation.
Run on each endpoint separately; --role labels the local source or receiver.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

from benchmark_core import (IntervalTracker, ParseError, normalize_peer,
                            parse_nettop, parse_ps, summarize, validate_port)

CADENCE_SECONDS = 2


def positive_int(value):
    try:
        result = int(value)
        if result > 0:
            return result
    except ValueError:
        pass
    raise argparse.ArgumentTypeError('Must be a positive integer')


def duration(value):
    try:
        result = float(value)
        if 0 < result <= 86400:
            return result
    except ValueError:
        pass
    raise argparse.ArgumentTypeError('Duration must be positive and at most one day')


def peer_argument(value):
    try:
        return normalize_peer(value)
    except ParseError:
        raise argparse.ArgumentTypeError('Peer selector must be an IP address') from None


def port_argument(value):
    try:
        return validate_port(int(value))
    except (ValueError, ParseError):
        raise argparse.ArgumentTypeError('Port must be an integer from 1 to 65535') from None


def sensor(command):
    """Do not return stderr or include sensor text in an error message."""
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=5, check=False,
                                env={**os.environ, 'LC_ALL': 'C'})
    except subprocess.TimeoutExpired:
        return None, 'sensor_timeout'
    except OSError:
        return None, 'sensor_unavailable'
    if result.returncode != 0:
        return None, 'sensor_failed'
    return result.stdout, None


def take_sample(pid, peer, tracker, sequence, port=39393):
    started = time.monotonic_ns()
    nettop, error = sensor(['/usr/bin/nettop', '-n', '-m', 'tcp', '-p', str(pid), '-L', '1', '-x',
                            '-J', 'interface,state,bytes_in,bytes_out,re-tx,rtt_avg'])
    network_ns = time.monotonic_ns()
    flows = []
    if error is None:
        try:
            flows = parse_nettop(nettop, peer, port)
        except ParseError:
            error = 'sensor_parse_error'
    resources, cpu_error = sensor(['/bin/ps', '-p', str(pid), '-o', '%cpu=,rss=,time='])
    cpu_ns = time.monotonic_ns()
    cpu = None
    if cpu_error is None:
        try:
            cpu = parse_ps(resources)
        except ParseError:
            cpu_error = 'sensor_parse_error'
    finished = time.monotonic_ns()
    result = {'record': 'sample', 'sequence': sequence,
              'sample_started_monotonic_ns': started,
              'network_completed_monotonic_ns': network_ns,
              'cpu_completed_monotonic_ns': cpu_ns,
              'sample_completed_monotonic_ns': finished,
              'collection_elapsed_ms': (finished - started) / 1e6,
              'network_status': 'sensor_error' if error else 'ok' if flows else 'no_matching_flow',
              'network_error': error, 'cpu_error': cpu_error,
              'flows': tracker.network(flows, network_ns, sensor_ok=error is None),
              'cpu': tracker.cpu(cpu, cpu_ns)}
    # Transient raw sensor strings and endpoint keys are never serialized.
    return result


def write_new_json(path, value):
    with path.open('x') as stream:
        json.dump(value, stream, indent=2, allow_nan=False)
        stream.write('\n')


def collect(args):
    port = validate_port(args.port)
    summary_path = args.summary_output or args.output.with_name(args.output.name + '.summary.json')
    if summary_path == args.output or summary_path.exists():
        raise ValueError('Summary output must be a separate new file')
    tracker = IntervalTracker()
    started = time.monotonic_ns()
    deadline = started + int(args.duration * 1e9)
    header = {'record': 'run', 'format_version': 1, 'role': args.role,
              'pid': args.pid, 'requested_duration_seconds': args.duration,
              'sampling_cadence_seconds': CADENCE_SECONDS,
              'started_monotonic_ns': started, 'flow_port': port,
              'flow_state_required': 'Established',
              're_tx_unit': 'nettop native cumulative counter; packet/byte unit not asserted',
              'peer_selector_used': args.peer is not None,
              'privacy': {'endpoint_addresses_retained': False, 'sensor_raw_output_retained': False,
                          'keys_payloads_logs_or_input_read': False},
              'perturbation': {'debugger_attached_by_collector': False, 'active_network_probes': False,
                              'nettop_and_ps_subprocesses': True, 'observer_effect_eliminated': False}}
    records = [header]
    args.output.parent.mkdir(parents=True, exist_ok=True)
    summary_path.parent.mkdir(parents=True, exist_ok=True)
    # Exclusive creation avoids accidentally mixing separate runs.
    fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    sequence = 0
    next_tick = started
    interrupted = False
    with os.fdopen(fd, 'w') as stream:
        stream.write(json.dumps(header, allow_nan=False) + '\n')
        stream.flush()
        try:
            while time.monotonic_ns() < deadline:
                now = time.monotonic_ns()
                if now < next_tick:
                    time.sleep(min((next_tick - now) / 1e9, max(0, (deadline - now) / 1e9)))
                    if time.monotonic_ns() >= deadline:
                        break
                sample = take_sample(args.pid, args.peer, tracker, sequence, port)
                records.append(sample)
                stream.write(json.dumps(sample, allow_nan=False) + '\n')
                stream.flush()
                sequence += 1
                next_tick += int(CADENCE_SECONDS * 1e9)
                # Skip missed slots instead of creating a burst of sensor calls.
                now = time.monotonic_ns()
                while next_tick < now:
                    next_tick += int(CADENCE_SECONDS * 1e9)
        except KeyboardInterrupt:
            interrupted = True
        end = {'record': 'end', 'completed_monotonic_ns': time.monotonic_ns(),
               'sample_count': sequence, 'interrupted': interrupted}
        stream.write(json.dumps(end) + '\n')
        stream.flush()
    records.append(end)
    write_new_json(summary_path, summarize(records))
    return {'samples': sequence, 'interrupted': interrupted,
            'summary_written': True, 'endpoint_addresses_retained': False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pid', type=positive_int, required=True)
    parser.add_argument('--port', type=port_argument, required=True,
                        help='Established TCP transport port (default: 39393)')
    parser.add_argument('--duration', type=duration, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--summary-output', type=Path)
    parser.add_argument('--peer', type=peer_argument, help='Optional peer IP selector; never saved')
    parser.add_argument('--role', choices=('source', 'receiver', 'unspecified'), default='unspecified')
    args = parser.parse_args()
    try:
        result = collect(args)
    except (OSError, ValueError):
        parser.exit(1, 'Collection failed: output unavailable or invalid configuration\n')
    print(json.dumps(result))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
