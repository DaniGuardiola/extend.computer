import argparse
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import collect
from benchmark_core import (Flow, IntervalTracker, ParseError, normalize_peer,
                            parse_cpu_time, parse_nettop, parse_ps, summarize, validate_port)

HEADER = ',interface,state,bytes_in,bytes_out,re-tx,rtt_avg,\n'
TARGET = 'tcp4 192.0.2.1:57839<->192.0.2.2:39393,en0,Established,1000000,200000,7,5.88 ms,\n'
PS = ' 125.5 65536 01:30.25\n'


def flow(incoming=0, outgoing=0, retrans=0, rtt=5, key=('private-in-memory-key',)):
    return Flow(key, incoming, outgoing, retrans, rtt)


def sample(flow_rows, cpu=None, status='ok'):
    return {'record': 'sample', 'flows': flow_rows, 'cpu': cpu or {'available': False},
            'network_status': status, 'collection_elapsed_ms': 10}


class NetTopParserTests(unittest.TestCase):
    def test_only_established_target_flow_not_process_totals_or_listener(self):
        text = HEADER + 'synthetic-process.999,,,900000000,800000000,88,,\n' + TARGET
        text += 'tcp4 192.0.2.1:44300<->198.51.100.1:443,en0,Established,4000,5000,0,20 ms,\n'
        text += 'tcp4 *:39393<->*:*,,Listen,,,,,\n'
        text += 'tcp6 *.39393<->*.*,,Listen,,,,,\n'
        rows = parse_nettop(text)
        self.assertEqual(len(rows), 1)
        self.assertEqual((rows[0].bytes_in, rows[0].bytes_out), (1000000, 200000))
        self.assertEqual(rows[0].retransmissions, 7)
        self.assertEqual(rows[0].rtt_ms, 5.88)
        self.assertNotIn('192.0.2.', repr(rows))

    def test_state_is_required_and_nonestablished_is_excluded(self):
        with self.assertRaises(ParseError):
            parse_nettop(',bytes_in,bytes_out,re-tx,rtt_avg,\n' + TARGET)
        for state in ('Listen', 'SynSent', 'CloseWait'):
            self.assertEqual(parse_nettop(HEADER + TARGET.replace('Established', state)), [])

    def test_peer_is_exact_and_not_a_prefix_match(self):
        text = HEADER + TARGET + TARGET.replace('192.0.2.2:', '192.0.2.20:')
        self.assertEqual(len(parse_nettop(text, '192.0.2.2')), 1)
        self.assertEqual(parse_nettop(text, '192.0.2.200'), [])
        with self.assertRaises(ParseError) as caught:
            normalize_peer('PRIVATE_HOSTNAME_SENTINEL')
        self.assertNotIn('PRIVATE_HOSTNAME', str(caught.exception))

    def test_custom_transport_port_excludes_other_port_flow(self):
        custom = TARGET.replace(':39393', ':42424').replace('1000000', '77')
        rows = parse_nettop(HEADER + TARGET + custom, port=42424)
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0].bytes_in, 77)
        self.assertEqual(parse_nettop(HEADER + TARGET, port=42424), [])
        for value in (0, 65536, -1, True, '39393', 1.5):
            with self.assertRaises(ParseError):
                validate_port(value)
        self.assertEqual(validate_port(1), 1)
        self.assertEqual(validate_port(65535), 65535)
        for value in ('0', '65536', 'PRIVATE_PORT_SENTINEL'):
            with self.assertRaises(argparse.ArgumentTypeError) as caught:
                collect.port_argument(value)
            self.assertNotIn('PRIVATE', str(caught.exception))

    def test_ipv6_native_dot_port_and_bracketed_colon_port(self):
        text = HEADER + 'tcp6 fe80::1%en0.57839<->fe80::2%en0.39393,en0,Established,11,12,0,500 us,\n'
        rows = parse_nettop(text, 'fe80::2')
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0].rtt_ms, .5)
        rows = parse_nettop(HEADER + 'tcp6 [2001:db8::1]:57839<->[2001:db8::2]:39393,en0,Established,11,12,,0.02 s,\n')
        self.assertEqual(rows[0].rtt_ms, 20)
        self.assertIsNone(rows[0].retransmissions)

    def test_repeated_headers_work_but_duplicate_snapshot_is_rejected(self):
        rows = parse_nettop(HEADER + 'synthetic-process.1,,,1,2,3,,\n' + HEADER + TARGET)
        self.assertEqual(len(rows), 1)
        with self.assertRaises(ParseError):
            parse_nettop(HEADER + TARGET + HEADER + TARGET)

    def test_invalid_target_counters_errors_do_not_echo_row(self):
        for text in (TARGET.replace('1000000', 'PRIVATE_SENTINEL'), TARGET.replace('5.88 ms', 'PRIVATE_SENTINEL')):
            with self.assertRaises(ParseError) as caught:
                parse_nettop(HEADER + text)
            self.assertNotIn('PRIVATE', str(caught.exception))
            self.assertNotIn('192.0.2.', str(caught.exception))
        # An unrelated malformed HTTPS flow must not spoil selected display statistics.
        self.assertEqual(len(parse_nettop(HEADER + TARGET + 'tcp4 192.0.2.1:4<->192.0.2.2:443,en0,Established,PRIVATE,PRIVATE,PRIVATE,PRIVATE,\n')), 1)

    def test_missing_header_and_truncated_rows_fail(self):
        for text in ('', TARGET, HEADER + 'tcp4 192.0.2.1:1<->192.0.2.2:39393,en0,Established,\n'):
            with self.assertRaises(ParseError):
                parse_nettop(text)


class IntervalTests(unittest.TestCase):
    def test_first_sample_is_baseline_then_mbps_and_retransmission_delta(self):
        tracker = IntervalTracker()
        self.assertFalse(tracker.network([flow(100, 200, 7)], 0)[0]['interval']['valid'])
        item = tracker.network([flow(1600100, 800200, 10)], 2_000_000_000)[0]
        self.assertEqual(item['flow_id'], 'flow-1')
        self.assertEqual(item['interval']['mbps_in'], 6.4)
        self.assertEqual(item['interval']['mbps_out'], 3.2)
        self.assertEqual(item['interval']['retransmission_delta'], 3)
        self.assertNotIn('private-in-memory-key', json.dumps(item))

    def test_counter_reset_is_excluded_and_rebases(self):
        tracker = IntervalTracker()
        tracker.network([flow(1000, 2000, 10)], 0)
        reset = tracker.network([flow(10, 20, 0)], 2_000_000_000)[0]
        self.assertEqual(reset['interval']['reason'], 'counter_reset')
        result = tracker.network([flow(110, 220, 2)], 4_000_000_000)[0]
        self.assertEqual(result['interval']['bytes_in_delta'], 100)
        self.assertEqual(result['interval']['retransmission_delta'], 2)

    def test_retransmission_reset_alone_invalidates_window(self):
        tracker = IntervalTracker()
        tracker.network([flow(10, 20, 10)], 0)
        result = tracker.network([flow(20, 40, 2)], 2_000_000_000)[0]
        self.assertEqual(result['interval']['reason'], 'counter_reset')

    def test_dropout_and_reappearance_do_not_bridge_missing_window(self):
        for sensor_ok, reason in ((True, 'flow_dropout'), (False, 'sensor_dropout')):
            tracker = IntervalTracker()
            tracker.network([flow(10)], 0)
            missing = tracker.network([], 2_000_000_000, sensor_ok=sensor_ok)
            self.assertEqual(missing[0]['interval']['reason'], reason)
            returned = tracker.network([flow(100)], 4_000_000_000)[0]
            self.assertEqual(returned['interval']['reason'], 'baseline')
            self.assertEqual(returned['flow_id'], 'flow-1')

    def test_changed_endpoint_has_new_opaque_id(self):
        tracker = IntervalTracker()
        tracker.network([flow(key=('one',))], 0)
        rows = tracker.network([flow(key=('two',))], 2_000_000_000)
        self.assertEqual([row['flow_id'] for row in rows], ['flow-1', 'flow-2'])
        self.assertFalse(any(row['interval']['valid'] for row in rows))

    def test_gap_and_nonpositive_time_are_invalid(self):
        for timestamp, reason in ((10_000_000_000, 'cadence_gap'), (0, 'nonpositive_interval')):
            tracker = IntervalTracker()
            tracker.network([flow()], 0)
            self.assertEqual(tracker.network([flow(10)], timestamp)[0]['interval']['reason'], reason)

    def test_missing_retransmission_stays_unknown_not_zero(self):
        tracker = IntervalTracker()
        tracker.network([flow(retrans=None)], 0)
        result = tracker.network([flow(20, retrans=4)], 2_000_000_000)[0]
        self.assertTrue(result['interval']['valid'])
        self.assertIsNone(result['interval']['retransmission_delta'])


class CpuAndSummaryTests(unittest.TestCase):
    def test_ps_cpu_formats_multicore_and_rss_units(self):
        result = parse_ps(PS)
        self.assertEqual(result['ps_cpu_percent'], 125.5)
        self.assertEqual(result['rss_bytes'], 64 * 1024**2)
        self.assertEqual(result['cumulative_cpu_seconds'], 90.25)
        self.assertEqual(parse_cpu_time('2-01:02:03.50'), 2 * 86400 + 3723.5)
        self.assertEqual(parse_cpu_time('100:00.00'), 6000)
        for text in ('PRIVATE_SENTINEL', '01:60.0', '1:60:00', '1 2 3 4', 'nan 1 01:00'):
            with self.assertRaises(ParseError):
                parse_ps(text) if ' ' in text else parse_cpu_time(text)

    def test_cpu_delta_reset_and_dropout(self):
        tracker = IntervalTracker()
        tracker.cpu(parse_ps('100 1000 01:00.0'), 0)
        second = tracker.cpu(parse_ps('100 2000 01:03.0'), 2_000_000_000)
        self.assertEqual(second['interval']['cpu_percent'], 150)
        reset = tracker.cpu(parse_ps('1 1000 00:01.0'), 4_000_000_000)
        self.assertEqual(reset['interval']['reason'], 'counter_reset')
        tracker.cpu(None, 6_000_000_000)
        self.assertEqual(tracker.cpu(parse_ps('1 1000 00:02.0'), 8_000_000_000)['interval']['reason'], 'baseline')

    def test_time_weighted_summary_omits_private_fields(self):
        tracker = IntervalTracker()
        rows = [{'record': 'run', 'role': 'source', 'private': 'PRIVATE_ENDPOINT_SENTINEL'}]
        rows.append(sample(tracker.network([flow(0, 0, 0, 1)], 0)))
        rows.append(sample(tracker.network([flow(1_000_000, 0, 2, 3)], 2_000_000_000)))
        rows.append(sample(tracker.network([flow(5_000_000, 0, 5, 9)], 6_000_000_000)))
        result = summarize(rows)
        out = result['flows'][0]
        self.assertEqual(out['valid_windows'], 2)
        self.assertAlmostEqual(out['time_weighted_mbps_in'], 40 / 6)
        self.assertEqual(out['interval_mbps_in']['median'], 6)
        self.assertEqual(out['tcp_rtt_ms']['median'], 3)
        self.assertEqual(out['retransmission_delta_sum'], 5)
        self.assertNotIn('PRIVATE', json.dumps(result))
        self.assertNotIn('private-in-memory-key', json.dumps(result))

    def test_empty_summary_has_unknown_metrics_and_sensor_counts(self):
        rows = [sample([], status='sensor_error'), sample([], status='no_matching_flow')]
        result = summarize(rows)
        self.assertEqual(result['network_sensor_error_samples'], 1)
        self.assertEqual(result['no_matching_flow_samples'], 1)
        self.assertIsNone(result['cpu']['time_weighted_interval_percent'])
        self.assertEqual(result['flows'], [])

    def test_summary_port_comes_from_run_configuration(self):
        self.assertEqual(summarize([{'record': 'run', 'flow_port': 42424}])['target_port'], 42424)
        self.assertIsNone(summarize([{'record': 'run', 'flow_port': 'PRIVATE_PORT_SENTINEL'}])['target_port'])
        self.assertIsNone(summarize([])['target_port'])

    def test_arbitrary_flow_names_are_not_echoed(self):
        with self.assertRaises(ParseError) as caught:
            summarize([sample([{'flow_id': 'PRIVATE_ENDPOINT_SENTINEL'}])])
        self.assertNotIn('PRIVATE', str(caught.exception))

    def test_metadata_fields_cannot_be_used_to_echo_strings_or_nan(self):
        tracker = IntervalTracker()
        rows = tracker.network([flow()], 0)
        for value in ('PRIVATE_FIELD_SENTINEL', float('nan'), -1):
            bad = json.loads(json.dumps(rows))
            bad[0]['tcp_rtt_ms'] = value
            with self.assertRaises(ParseError) as caught:
                summarize([sample(bad)])
            self.assertNotIn('PRIVATE', str(caught.exception))


class CollectorTests(unittest.TestCase):
    def test_take_sample_is_metadata_only_and_commands_are_pid_scoped(self):
        commands = []
        def fake_sensor(command):
            commands.append(command)
            return (HEADER + TARGET, None) if command[0].endswith('nettop') else (PS, None)
        with patch.object(collect, 'sensor', side_effect=fake_sensor), patch.object(collect.time, 'monotonic_ns', side_effect=(0, 1000000, 2000000, 3000000)):
            row = collect.take_sample(999, '192.0.2.2', IntervalTracker(), 0)
        wire = json.dumps(row)
        self.assertNotIn('192.0.2.', wire)
        self.assertNotIn('synthetic-process.', wire)
        self.assertEqual(row['collection_elapsed_ms'], 3)
        self.assertIn('state', commands[0][-1])
        self.assertEqual(commands[1], ['/bin/ps', '-p', '999', '-o', '%cpu=,rss=,time='])

    def test_sensor_errors_do_not_become_zero_or_raw_output(self):
        with patch.object(collect, 'sensor', return_value=(None, 'sensor_failed')):
            row = collect.take_sample(999, None, IntervalTracker(), 0)
        self.assertEqual(row['network_status'], 'sensor_error')
        self.assertFalse(row['cpu']['available'])
        self.assertEqual(row['flows'], [])

    def test_exclusive_output_bounded_schedule_and_summary(self):
        clock = [0]
        def monotonic():
            return clock[0]
        def sleep(seconds):
            clock[0] += int(seconds * 1e9)
        def fake_sample(pid, peer, tracker, sequence, port=39393):
            clock[0] += 100_000_000
            row = sample(tracker.network([flow(sequence * 1_000_000)], clock[0]))
            row.update({'sequence': sequence})
            return row
        with tempfile.TemporaryDirectory() as temporary:
            out = Path(temporary) / 'metadata.jsonl'
            args = argparse.Namespace(pid=999, port=42424, duration=3, output=out, summary_output=None,
                                      peer='192.0.2.2', role='receiver')
            with patch.object(collect.time, 'monotonic_ns', side_effect=monotonic), patch.object(collect.time, 'sleep', side_effect=sleep), patch.object(collect, 'take_sample', side_effect=fake_sample):
                result = collect.collect(args)
            self.assertEqual(result['samples'], 2)
            records = [json.loads(line) for line in out.read_text().splitlines()]
            self.assertEqual(records[0]['role'], 'receiver')
            self.assertEqual(records[0]['flow_port'], 42424)
            self.assertEqual(records[-1]['record'], 'end')
            self.assertNotIn('192.0.2.', out.read_text())
            self.assertEqual(json.loads(Path(str(out) + '.summary.json').read_text())['flows'][0]['valid_windows'], 1)
            self.assertEqual(json.loads(Path(str(out) + '.summary.json').read_text())['target_port'], 42424)
            with self.assertRaises(ValueError):
                collect.collect(args)


if __name__ == '__main__':
    unittest.main()
