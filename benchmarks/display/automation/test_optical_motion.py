import sys
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]/'campaign'))
from optical_motion import summarize, crossing_delay


def trace(stall=False, missing=False, swapped=False):
    rows=[]
    frozen=None
    for i in range(1440):
        t=i/240
        phase=t%2
        x=40+280*(phase if phase<1 else 2-phase)
        reference={'x':x,'width':16,'spatial_bound':2}
        target=dict(reference)
        if stall and 2.1 <= t < 2.35:
            if frozen is None:frozen=x
            target['x']=frozen
        values=[reference,target]
        if missing and i%2==0:values[1]=None
        if swapped:values.reverse()
        rows.append({'playback_s':t,'cursor_positions':values})
    return rows


class OpticalMotionTests(unittest.TestCase):
    def test_smooth_motion_has_no_holds(self):
        result=summarize(trace(),1,240,0)
        self.assertEqual(result['longest_observed_receiver_hold_ms'],0)
        self.assertGreater(result['receiver']['resolvable_changes'],200)
        self.assertNotIn('fps',result['receiver'])

    def test_stall_remains_measured_and_reference_swap_rejected(self):
        result=summarize(trace(stall=True),1,240,0)
        self.assertEqual(result['status'],'measured')
        self.assertGreater(result['longest_observed_receiver_hold_ms'],230)
        self.assertLess(result['longest_observed_receiver_hold_ms'],280)
        with self.assertRaisesRegex(ValueError,'reference stalled'):
            summarize(trace(stall=True,swapped=True),1,240,0)

    def test_occlusion_is_not_invented_as_stall(self):
        with self.assertRaisesRegex(ValueError,'coverage'):
            summarize(trace(missing=True),1,240,0)
        rows=trace()
        for row in rows[510:530]:row['cursor_positions'][1]=None
        result=summarize(rows,1,240,0)
        self.assertEqual(result['longest_observed_receiver_hold_ms'],0)
        self.assertGreater(result['receiver']['unreadable_gap_ms']['max'],70)
        self.assertGreater(result['receiver']['change_gaps_containing_unreadable_frames_ms']['max'],70)

    def test_static_receiver_is_measured_bad_performance(self):
        rows=trace()
        for row in rows:row['cursor_positions'][1]['x']=40
        result=summarize(rows,1,240,0)
        self.assertGreater(result['longest_observed_receiver_hold_ms'],5900)

    def test_known_delay_bounded_in_both_directions_and_cycles(self):
        rows = trace()
        for i, row in enumerate(rows):
            t = i/240 - .05
            phase = t % 2
            row['cursor_positions'][1]['x'] = 40+280*(phase if phase < 1 else 2-phase)
        result = summarize(rows, 1, 240, 0)['crossing_delay']
        self.assertEqual(result['status'], 'measured')
        self.assertEqual(result['ambiguous_source_crossings'], 0)
        self.assertGreater(result['matched_source_coverage'], .95)
        self.assertEqual({c['direction'] for c in result['crossings']}, {'left', 'right'})
        self.assertGreater(len({c['source_cycle'] for c in result['crossings']}), 1)
        for crossing in result['crossings']:
            self.assertLessEqual(crossing['lower_ms'], 50.000001)
            self.assertGreaterEqual(crossing['upper_ms'], 49.999999)
            # Four pixels of endpoint uncertainty on each signal, plus sampling.
            self.assertGreaterEqual(crossing['upper_ms']-crossing['lower_ms'], 14)
        self.assertAlmostEqual(result['midpoint_ms']['median'], 50, delta=3)

    def test_missing_crossing_and_hold_are_not_interpolated(self):
        rows = trace()
        # Crossing x=96 occurs at .2 seconds. Hide all uncertainty endpoints.
        for row in rows[43:54]:
            row['cursor_positions'][1] = None
        result = summarize(rows, 1, 240, 0)['crossing_delay']
        self.assertGreater(result['unmatched_source_crossings'], 0)
        self.assertFalse(any(c['x'] == 96 and c['source_cycle'] == 1 and c['direction'] == 'right'
                             for c in result['crossings']))
        held = summarize(trace(stall=True), 1, 240, 0)
        self.assertGreater(held['longest_observed_receiver_hold_ms'], 230)
        self.assertGreater(held['crossing_delay']['receiver_rejected']['hold_or_wide_interval'], 0)
        static = trace()
        for row in static:
            row['cursor_positions'][1]['x'] = 40
        self.assertEqual(summarize(static, 1, 240, 0)['crossing_delay']['status'],
                         'insufficient_or_ambiguous')

    def test_delay_outside_search_window_is_unmatched(self):
        rows = trace()
        for i, row in enumerate(rows):
            phase = (i/240 - .65) % 2
            row['cursor_positions'][1]['x'] = 40+280*(phase if phase < 1 else 2-phase)
        result = summarize(rows, 1, 240, 0)['crossing_delay']
        self.assertEqual(result['matched_crossings'], 0)
        self.assertEqual(result['status'], 'insufficient_or_ambiguous')
        self.assertIsNone(result['midpoint_ms'])

    def test_reversal_and_cycle_alias_are_rejected(self):
        rows = trace()
        # A short local bounce crossing x=160 fails the monotonic guard.
        for i, x in enumerate([146, 155, 164, 151, 160, 169], start=100):
            rows[i]['cursor_positions'][1]['x'] = x
        result = summarize(rows, 1, 240, 0)['crossing_delay']
        self.assertGreater(result['receiver_rejected']['reversal_guard'], 0)
        # Fast repeated cycles within the delay search window are ambiguous,
        # not assigned arbitrarily by ordinal or nearest-neighbor timing.
        rows = trace()
        for i, row in enumerate(rows):
            phase = (i/240 % .4)/.2
            x = 40+280*(phase if phase < 1 else 2-phase)
            for value in row['cursor_positions']:
                value['x'] = x
        result = crossing_delay(rows, [i/240 for i in range(len(rows))], 0, [])
        self.assertEqual(result['status'], 'insufficient_or_ambiguous')
        self.assertGreater(result['ambiguous_source_crossings'], 0)
        self.assertEqual(result['matched_crossings'], 0)

    def test_invalid_clock_or_source_is_rejected(self):
        rows=trace();rows[100]['playback_s']=rows[99]['playback_s']
        with self.assertRaises(ValueError):summarize(rows,1,240,0)
        rows=trace()
        for row in rows:row['cursor_positions'][0]['x']=40
        with self.assertRaisesRegex(ValueError,'not observed'):summarize(rows,1,240,0)

if __name__=='__main__':unittest.main()
