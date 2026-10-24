from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'campaign'))
from stages import summarize


def event(stage, at, value):
    return {'stage': stage, 'at_ns': at, 'bytes': value, 'frame_ns': 0}


def readback(at, value, status=0, valid=1):
    rows=[event('encoder_bitrate_read_status', at, status),
          event('encoder_bitrate_value_valid', at+1, valid)]
    if valid: rows.append(event('encoder_bitrate_value', at+2, value))
    return rows


class BitrateEvidenceTests(unittest.TestCase):
    def evidence(self, rows):
        return summarize(rows, 10, 100)['encoder_target_bitrate']

    def test_requested_and_legacy_attempts_do_not_establish_actual_rate(self):
        rows=[event('config_bitrate', 1, 20_000_000),event('bitrate', 20, 1_000_000)]
        result=summarize(rows, 10, 100)
        self.assertEqual(result['configuration']['bitrate'], 20_000_000)
        self.assertEqual(result['encoder_target_bitrate']['status'], 'unavailable')
        self.assertIsNone(result['encoder_target_bitrate']['constant_bps'])

    def test_prior_driver_readback_covers_unchanged_target(self):
        result=self.evidence(readback(1, 20_000_000))
        self.assertTrue(result['whole_window_verified'])
        self.assertEqual(result['constant_bps'], 20_000_000)
        self.assertEqual(result['initial_bps'], 20_000_000)

    def test_successful_adaptation_retains_verified_changes(self):
        rows=readback(1,20_000_000)+[event('bitrate_set_status',20,0)]+readback(21,8_000_000)
        result=self.evidence(rows)
        self.assertTrue(result['whole_window_verified'])
        self.assertIsNone(result['constant_bps'])
        self.assertEqual(result['observed_values_bps'], [8_000_000,20_000_000])
        self.assertEqual(result['final_bps'], 8_000_000)

    def test_failed_setter_prevents_fixed_target_claim(self):
        result=self.evidence(readback(1,20_000_000)+[event('bitrate_set_status',20,-50)])
        self.assertFalse(result['whole_window_verified'])
        self.assertIsNone(result['constant_bps'])
        self.assertIsNone(result['final_bps'])

    def test_failed_readback_never_reuses_older_value(self):
        result=self.evidence(readback(1,20_000_000)+readback(21,None,status=-50,valid=0))
        self.assertFalse(result['whole_window_verified'])
        self.assertIsNone(result['final_bps'])
        before=self.evidence(readback(1,20_000_000)+readback(5,None,status=-50,valid=0))
        self.assertIsNone(before['initial_bps'])

    def test_setter_needs_readback_within_window(self):
        rows=readback(1,20_000_000)+[event('bitrate_set_status',99,0)]+readback(101,8_000_000)
        result=self.evidence(rows)
        self.assertFalse(result['whole_window_verified'])
        self.assertEqual(len(result['unverified_setters']), 1)
        self.assertIsNone(result['final_bps'])

    def test_incomplete_driver_sequence_is_unverified(self):
        rows=readback(1,20_000_000)+[event('encoder_bitrate_read_status',99,0)]
        self.assertFalse(self.evidence(rows)['whole_window_verified'])


if __name__ == '__main__':
    unittest.main()
