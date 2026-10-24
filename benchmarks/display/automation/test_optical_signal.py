import json,sys,tempfile,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
import numpy as np
from optical_signal import align,analyze


def observations(delay=19, unrelated=False, periodic=False):
    rng=np.random.default_rng(714);values=rng.choice([-1,1],size=(600,16))
    source=values[np.arange(1920)//4]
    receiver=source[np.maximum(0,np.arange(1920)-delay)]
    if unrelated:receiver=rng.choice([-1,1],size=source.shape)
    if periodic:
        source=np.tile(((np.arange(1920)//4)%2*2-1)[:,None],(1,16));receiver=source.copy()
    rows=[]
    for i,(a,b) in enumerate(zip(source,receiver)):
        rows.append({'playback_s':i/30,'patches':[(120+90*np.stack([a,-a])).tolist(),
            (150+50*np.stack([b,-b])).tolist()],'tracked_corners':[[[1,2]]*4,[[3,4]]*4]})
    return rows


class OpticalSignalTests(unittest.TestCase):
    def test_observed_signal_retains_known_delay_without_counter_ids(self):
        rows=observations();result=align(rows,0,.125)
        self.assertEqual(result['status'],'modelled_alignment')
        self.assertAlmostEqual(result['estimated_constant_delay_ms'],19/240*1000,places=5)
        self.assertGreater(result['correlation'],.99)
        self.assertFalse(any('counter_ids' in row for row in rows))

    def test_unknown_alignment_and_periodic_ambiguity_are_not_success(self):
        for rows in [observations(unrelated=True),observations(periodic=True)]:
            with self.subTest():self.assertEqual(align(rows,0,.125)['status'],'ambiguous_alignment')

    def test_missing_markers_do_not_bypass_original_tracking_gate(self):
        rows=observations()
        for row in rows[:150]:row['tracked_corners'][1]=[]
        with self.assertRaisesRegex(ValueError,'tracking gate'):align(rows,0,.125)

    def test_static_or_wrong_cadence_cannot_produce_delay(self):
        rows=observations()
        for row in rows:row['patches']=[[[120]*16]*2]*2
        with self.assertRaisesRegex(ValueError,'varying'):align(rows,0,.125)
        with self.assertRaisesRegex(ValueError,'cadence'):align(observations(),0,1)

    def test_missing_or_invalid_camera_timescale_is_explicitly_unavailable(self):
        with tempfile.TemporaryDirectory() as temporary:
            folder=Path(temporary)
            for summary in [{}, {'video_metadata':{'nominal_playback_fps':30}},
                            {'video_metadata':{'nominal_playback_fps':240},'measured_sensor_hz':30}]:
                with self.subTest(summary=summary):
                    (folder/'summary.json').write_text(json.dumps(summary))
                    with self.assertRaisesRegex(ValueError,'timescale metadata'):analyze(folder,0)

if __name__=='__main__':unittest.main()
