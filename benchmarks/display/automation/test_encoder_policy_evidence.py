import sys
import unittest
from pathlib import Path

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
from stages import summarize


class EncoderPolicyEvidenceTests(unittest.TestCase):
    def test_startup_auto_fallback_remains_separate_from_selected_mode(self):
        values={'encoder_low_latency_requested':1,'encoder_low_latency_auto':1,
                'encoder_low_latency_create_status':-12903,'encoder_standard_create_status':0,
                'encoder_low_latency_fallback':1,'encoder_low_latency':0,'encoder_hardware':1}
        rows=[{'stage':key,'bytes':value,'at_ns':10+i,'frame_ns':0}
              for i,(key,value) in enumerate(values.items())]
        rows.append({'stage':'encoded','bytes':100,'at_ns':200,'frame_ns':150})
        result=summarize(rows,100,300)
        self.assertTrue(result['configuration']['encoder_low_latency_requested'])
        self.assertTrue(result['configuration']['encoder_low_latency_auto'])
        self.assertTrue(result['configuration']['encoder_low_latency_fallback'])
        self.assertFalse(result['configuration']['encoder_low_latency'])
        self.assertEqual(result['configuration']['encoder_low_latency_create_status'],-12903)
        self.assertEqual(result['configuration']['encoder_standard_create_status'],0)
        self.assertTrue(result['configuration']['encoder_hardware'])
        self.assertEqual(result['event_counts']['encoded'],1)

    def test_latest_selected_mode_is_used_without_inference_for_old_traces(self):
        rows=[{'stage':'encoder_low_latency','bytes':0,'at_ns':10,'frame_ns':0},
              {'stage':'encoder_low_latency','bytes':1,'at_ns':20,'frame_ns':0},
              {'stage':'encoder_low_latency','bytes':0,'at_ns':400,'frame_ns':0}]
        self.assertTrue(summarize(rows,100,300)['configuration']['encoder_low_latency'])
        self.assertNotIn('encoder_low_latency',summarize([],100,300)['configuration'])


if __name__=='__main__':unittest.main()
