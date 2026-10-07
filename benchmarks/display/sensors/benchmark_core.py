"""Metadata-only parsing and interval accounting for a two-host TCP baseline."""
import csv
import io
import ipaddress
import math
import re
import statistics
from dataclasses import dataclass, field


class ParseError(ValueError):
    """A structural sensor error; never includes a raw row or endpoint."""


@dataclass(frozen=True)
class Flow:
    # Endpoint identity is transient and must never enter serialized output.
    key: tuple = field(repr=False)
    bytes_in: int
    bytes_out: int
    retransmissions: int | None
    rtt_ms: float | None


def normalize_peer(value):
    try:
        return str(ipaddress.ip_address(value.split('%', 1)[0]))
    except (ValueError, AttributeError):
        raise ParseError('Peer selector must be an IP address') from None


def _endpoint(value):
    value = value.strip()
    if '*' in value:
        return None
    match = re.fullmatch(r'(.+?)[.:](\d+)', value)
    if not match:
        return None
    address, port = match.groups()
    address = address.strip('[]').split('%', 1)[0]
    try:
        address = str(ipaddress.ip_address(address))
    except ValueError:
        return None
    port = int(port)
    if not 0 < port < 65536:
        return None
    return address, port


def _counter(value, optional=False):
    if not value.strip() and optional:
        return None
    if not re.fullmatch(r'\d+', value.strip()):
        raise ParseError('Invalid cumulative counter')
    return int(value)


def _rtt(value):
    if not value.strip() or value.strip() in ('-', 'n/a'):
        return None
    match = re.fullmatch(r'\s*(\d+(?:\.\d+)?)\s*(ms|us|µs|s)?\s*', value)
    if not match:
        raise ParseError('Invalid TCP RTT value')
    number, unit = match.groups()
    result = float(number) * {'ms': 1, 'us': .001, 'µs': .001, 's': 1000, None: 1}[unit]
    if not math.isfinite(result):
        raise ParseError('Invalid TCP RTT value')
    return result


def validate_port(value):
    if isinstance(value, bool) or not isinstance(value, int) or not 1 <= value <= 65535:
        raise ParseError('Transport port must be an integer from 1 to 65535')
    return value


def parse_nettop(text, peer=None, port=39393):
    """Return concrete Established flows for the configured port, never totals.

    CSV can repeat its header. A missing state column is rejected rather than
    inferring Established from counters. Endpoint identity stays in Flow.key
    solely to match counters between samples; the collector emits opaque IDs.
    """
    port = validate_port(port)
    peer = normalize_peer(peer) if peer is not None else None
    header = None
    flows = {}
    saw_header = False
    for row in csv.reader(io.StringIO(text)):
        row = [cell.strip() for cell in row]
        if not row or not any(row):
            continue
        if 'bytes_in' in row and 'bytes_out' in row:
            header = {name: index for index, name in enumerate(row) if name}
            if not {'state', 'bytes_in', 'bytes_out', 're-tx', 'rtt_avg'} <= header.keys():
                raise ParseError('Required nettop columns are missing')
            saw_header = True
            continue
        if header is None:
            continue
        label = row[0]
        match = re.fullmatch(r'tcp([46])\s+(.+?)<->(.+)', label)
        if not match:
            continue  # Process summary rows cannot be attributed to this flow.
        if len(row) <= max(header.values()):
            raise ParseError('Truncated nettop flow row')
        if row[header['state']].casefold() != 'established':
            continue
        left, right = _endpoint(match.group(2)), _endpoint(match.group(3))
        if left is None or right is None or port not in (left[1], right[1]):
            continue
        if peer is not None and peer not in (left[0], right[0]):
            continue
        key = (int(match.group(1)), left, right)
        if key in flows:
            # A multi-snapshot input is not one observation. Do not silently
            # combine counters from different instants under the same sample.
            raise ParseError('Duplicate target flow in one sensor sample')
        flows[key] = Flow(key, _counter(row[header['bytes_in']]),
                          _counter(row[header['bytes_out']]),
                          _counter(row[header['re-tx']], optional=True),
                          _rtt(row[header['rtt_avg']]))
    if not saw_header:
        raise ParseError('No nettop metric header')
    return list(flows.values())


def parse_cpu_time(value):
    """Parse ps cumulative CPU time: [[days-]hours:]minutes:seconds.fraction."""
    days = 0
    if '-' in value:
        day, value = value.split('-', 1)
        if not day.isdigit():
            raise ParseError('Invalid cumulative CPU time')
        days = int(day)
    parts = value.split(':')
    if len(parts) not in (2, 3):
        raise ParseError('Invalid cumulative CPU time')
    if not all(re.fullmatch(r'\d+', part) for part in parts[:-1]):
        raise ParseError('Invalid cumulative CPU time')
    if not re.fullmatch(r'\d+(?:\.\d+)?', parts[-1]):
        raise ParseError('Invalid cumulative CPU time')
    seconds = float(parts[-1])
    if seconds >= 60:
        raise ParseError('Invalid cumulative CPU time')
    minutes = int(parts[-2])
    hours = int(parts[-3]) if len(parts) == 3 else 0
    if len(parts) == 3 and minutes >= 60:
        raise ParseError('Invalid cumulative CPU time')
    return days * 86400 + hours * 3600 + minutes * 60 + seconds


def parse_ps(text):
    rows = [row.split() for row in text.splitlines() if row.strip()]
    if len(rows) != 1 or len(rows[0]) != 3:
        raise ParseError('Expected one PID resource row')
    cpu, rss, cpu_time = rows[0]
    try:
        cpu = float(cpu)
    except ValueError:
        raise ParseError('Invalid process CPU percentage') from None
    if not math.isfinite(cpu) or cpu < 0:
        raise ParseError('Invalid process CPU percentage')
    rss = _counter(rss)
    return {'ps_cpu_percent': cpu, 'rss_bytes': rss * 1024,
            'cumulative_cpu_seconds': parse_cpu_time(cpu_time)}


class IntervalTracker:
    def __init__(self, max_gap_seconds=4.5):
        self.max_gap_seconds = max_gap_seconds
        self._flow_ids = {}
        self._previous = {}
        self._cpu_previous = None

    def network(self, flows, timestamp_ns, sensor_ok=True):
        current = {flow.key: flow for flow in flows}
        results = []
        for key in self._previous.keys() - current.keys():
            results.append({'flow_id': self._flow_ids[key], 'present': False,
                            'interval': {'valid': False, 'reason': 'flow_dropout' if sensor_ok else 'sensor_dropout'}})
        for flow in flows:
            if flow.key not in self._flow_ids:
                self._flow_ids[flow.key] = f'flow-{len(self._flow_ids) + 1}'
            output = {'flow_id': self._flow_ids[flow.key], 'present': True,
                      'bytes_in': flow.bytes_in, 'bytes_out': flow.bytes_out,
                      'retransmissions': flow.retransmissions, 'tcp_rtt_ms': flow.rtt_ms}
            previous = self._previous.get(flow.key)
            if previous is None:
                output['interval'] = {'valid': False, 'reason': 'baseline'}
            else:
                old, old_ns = previous
                elapsed = (timestamp_ns - old_ns) / 1e9
                reset = flow.bytes_in < old.bytes_in or flow.bytes_out < old.bytes_out
                if flow.retransmissions is not None and old.retransmissions is not None:
                    reset = reset or flow.retransmissions < old.retransmissions
                reason = ('nonpositive_interval' if elapsed <= 0 else 'counter_reset' if reset
                          else 'cadence_gap' if elapsed > self.max_gap_seconds else None)
                if reason:
                    output['interval'] = {'valid': False, 'reason': reason}
                else:
                    incoming = flow.bytes_in - old.bytes_in
                    outgoing = flow.bytes_out - old.bytes_out
                    retrans = (flow.retransmissions - old.retransmissions
                               if flow.retransmissions is not None and old.retransmissions is not None else None)
                    output['interval'] = {'valid': True, 'elapsed_seconds': elapsed,
                                          'bytes_in_delta': incoming, 'bytes_out_delta': outgoing,
                                          'mbps_in': incoming * 8 / elapsed / 1e6,
                                          'mbps_out': outgoing * 8 / elapsed / 1e6,
                                          'retransmission_delta': retrans}
            results.append(output)
        # A dropout explicitly breaks continuity; returning flows need a new baseline.
        self._previous = {key: (flow, timestamp_ns) for key, flow in current.items()} if sensor_ok else {}
        return results

    def cpu(self, metrics, timestamp_ns):
        if metrics is None:
            self._cpu_previous = None
            return {'available': False, 'interval': {'valid': False, 'reason': 'sensor_dropout'}}
        output = {'available': True, **metrics}
        previous = self._cpu_previous
        reason = 'baseline'
        if previous is not None:
            old, old_ns = previous
            elapsed = (timestamp_ns - old_ns) / 1e9
            delta = metrics['cumulative_cpu_seconds'] - old['cumulative_cpu_seconds']
            reason = ('nonpositive_interval' if elapsed <= 0 else 'counter_reset' if delta < 0
                      else 'cadence_gap' if elapsed > self.max_gap_seconds else None)
            if reason is None:
                output['interval'] = {'valid': True, 'elapsed_seconds': elapsed,
                                      'cpu_seconds_delta': delta,
                                      'cpu_percent': 100 * delta / elapsed}
        if reason:
            output['interval'] = {'valid': False, 'reason': reason}
        self._cpu_previous = (dict(metrics), timestamp_ns)
        return output


def _stats(values):
    values = sorted(values)
    if not values:
        return {'count': 0, 'min': None, 'median': None, 'p95': None, 'max': None}
    return {'count': len(values), 'min': values[0], 'median': statistics.median(values),
            'p95': values[max(0, math.ceil(.95 * len(values)) - 1)], 'max': values[-1]}


def _metric(value, positive=False, integer=False, optional=False):
    if value is None and optional:
        return
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ParseError('Invalid numeric metadata field')
    if not math.isfinite(value) or value < 0 or (positive and value == 0):
        raise ParseError('Invalid numeric metadata field')
    if integer and not isinstance(value, int):
        raise ParseError('Invalid integer metadata field')


def _validate_sample(sample):
    if 'collection_elapsed_ms' in sample:
        _metric(sample['collection_elapsed_ms'])
    flow_rows = sample.get('flows', [])
    if not isinstance(flow_rows, list):
        raise ParseError('Invalid flow metadata list')
    for flow in flow_rows:
        if not isinstance(flow, dict):
            raise ParseError('Invalid flow metadata record')
        interval = flow.get('interval', {})
        if not isinstance(interval, dict) or not isinstance(interval.get('valid'), bool):
            raise ParseError('Invalid interval metadata record')
        if flow.get('present'):
            for field in ('bytes_in', 'bytes_out'):
                _metric(flow.get(field), integer=True)
            _metric(flow.get('retransmissions'), integer=True, optional=True)
            _metric(flow.get('tcp_rtt_ms'), optional=True)
        if interval['valid']:
            _metric(interval.get('elapsed_seconds'), positive=True)
            for field in ('bytes_in_delta', 'bytes_out_delta'):
                _metric(interval.get(field), integer=True)
            for field in ('mbps_in', 'mbps_out'):
                _metric(interval.get(field))
            _metric(interval.get('retransmission_delta'), integer=True, optional=True)
    cpu = sample.get('cpu', {})
    if not isinstance(cpu, dict):
        raise ParseError('Invalid CPU metadata record')
    if cpu.get('available'):
        for field in ('ps_cpu_percent', 'cumulative_cpu_seconds'):
            _metric(cpu.get(field))
        _metric(cpu.get('rss_bytes'), integer=True)
        interval = cpu.get('interval', {})
        if not isinstance(interval, dict) or not isinstance(interval.get('valid'), bool):
            raise ParseError('Invalid CPU interval metadata record')
        if interval['valid']:
            _metric(interval.get('elapsed_seconds'), positive=True)
            _metric(interval.get('cpu_seconds_delta'))
            _metric(interval.get('cpu_percent'))


def summarize(records):
    """Summarize whitelisted metrics only; do not copy arbitrary input fields."""
    if not isinstance(records, list) or not all(isinstance(row, dict) for row in records):
        raise ParseError('Invalid metadata record list')
    run = next((row for row in records if row.get('record') == 'run'), {})
    role = run.get('role')
    if role not in ('source', 'receiver', 'unspecified'):
        role = 'unspecified'
    target_port = run.get('flow_port')
    if not isinstance(target_port, int) or isinstance(target_port, bool) or not 1 <= target_port <= 65535:
        target_port = None
    samples = [row for row in records if row.get('record') == 'sample']
    for sample in samples:
        _validate_sample(sample)
    flows = {}
    for sample in samples:
        for flow in sample.get('flows', []):
            identifier = flow.get('flow_id', '')
            if not isinstance(identifier, str) or not re.fullmatch(r'flow-\d+', identifier):
                raise ParseError('Invalid opaque flow ID')
            flows.setdefault(identifier, []).append(flow)
    output_flows = []
    for identifier, rows in flows.items():
        intervals = [row['interval'] for row in rows if row.get('interval', {}).get('valid')]
        elapsed = sum(row['elapsed_seconds'] for row in intervals)
        invalid = {}
        for row in rows:
            if not row.get('interval', {}).get('valid'):
                reason = row.get('interval', {}).get('reason')
                if reason not in ('baseline', 'flow_dropout', 'sensor_dropout', 'counter_reset', 'cadence_gap', 'nonpositive_interval'):
                    reason = 'unknown'
                invalid[reason] = invalid.get(reason, 0) + 1
        retrans = [row['retransmission_delta'] for row in intervals if row.get('retransmission_delta') is not None]
        output_flows.append({'flow_id': identifier, 'present_samples': sum(row.get('present') is True for row in rows),
                             'valid_windows': len(intervals), 'valid_seconds': elapsed,
                             'invalid_windows': invalid,
                             'time_weighted_mbps_in': sum(row['bytes_in_delta'] for row in intervals) * 8 / elapsed / 1e6 if elapsed else None,
                             'time_weighted_mbps_out': sum(row['bytes_out_delta'] for row in intervals) * 8 / elapsed / 1e6 if elapsed else None,
                             'interval_mbps_in': _stats([row['mbps_in'] for row in intervals]),
                             'interval_mbps_out': _stats([row['mbps_out'] for row in intervals]),
                             'tcp_rtt_ms': _stats([row['tcp_rtt_ms'] for row in rows if row.get('present') and row.get('tcp_rtt_ms') is not None]),
                             'retransmission_delta_sum': sum(retrans) if retrans else None,
                             'retransmission_available_windows': len(retrans)})
    cpu = [sample['cpu'] for sample in samples if sample.get('cpu', {}).get('available')]
    cpu_intervals = [row['interval'] for row in cpu if row.get('interval', {}).get('valid')]
    cpu_elapsed = sum(row['elapsed_seconds'] for row in cpu_intervals)
    cpu_invalid = {}
    for sample in samples:
        row = sample.get('cpu', {})
        if not row.get('interval', {}).get('valid'):
            reason = row.get('interval', {}).get('reason')
            if reason not in ('baseline', 'sensor_dropout', 'counter_reset', 'cadence_gap', 'nonpositive_interval'):
                reason = 'unknown'
            cpu_invalid[reason] = cpu_invalid.get(reason, 0) + 1
    return {'format_version': 1, 'role': role, 'target_port': target_port,
            'sample_count': len(samples),
            'network_sensor_error_samples': sum(row.get('network_status') == 'sensor_error' for row in samples),
            'no_matching_flow_samples': sum(row.get('network_status') == 'no_matching_flow' for row in samples),
            'flows': output_flows,
            'cpu': {'available_samples': len(cpu), 'valid_windows': len(cpu_intervals),
                    'valid_seconds': cpu_elapsed, 'invalid_windows': cpu_invalid,
                    'time_weighted_interval_percent': 100 * sum(row['cpu_seconds_delta'] for row in cpu_intervals) / cpu_elapsed if cpu_elapsed else None,
                    'interval_percent': _stats([row['cpu_percent'] for row in cpu_intervals]),
                    'ps_reported_percent': _stats([row['ps_cpu_percent'] for row in cpu]),
                    'rss_mib': _stats([row['rss_bytes'] / 1024**2 for row in cpu])},
            'collector_elapsed_ms': _stats([row['collection_elapsed_ms'] for row in samples if 'collection_elapsed_ms' in row]),
            'perturbation': {'debugger_attached_by_collector': False, 'active_network_probes': False,
                             'sensors': ['nettop process TCP statistics', 'ps PID-only resource statistics'],
                             'sampling_cadence_seconds': 2, 'collection_wall_time_measured': True,
                             'collector_cpu_cost_measured': False,
                             'observer_effect_eliminated': False},
            'limits': ['TCP byte counts include all bytes on the selected flow; they are not video-only bitrate.',
                       'TCP RTT is not capture-to-display latency.',
                       'Each host uses its own monotonic clock; timestamps are not synchronized across hosts.',
                       'CPU percentage can exceed 100 on multiple cores; ps percentage and interval CPU-time percentage have different averaging windows.',
                       'RTT percentiles describe sampled TCP estimates, not individual packets.',
                       'Retransmission deltas retain nettop re-tx native counter units; they are not labeled as packet counts or a loss percentage.',
                       'Valid windows exclude baselines, counter resets, dropouts and cadence gaps.']}
