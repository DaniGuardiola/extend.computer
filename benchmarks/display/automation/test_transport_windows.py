import json,sys,tempfile,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
from stages import transport_windows

class TransportWindowTests(unittest.TestCase):
    def test_overlap_keeps_boundary_windows_without_summing_them(self):
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'app.log';offset=100_000_000_000
            values=[{'wall_end_ns':offset+finish*1_000_000_000,'window_ms':5000,'assembled_frames':count}
                    for finish,count in [(5,10),(10,20),(15,30),(20,40)]]
            path.write_text(''.join('Display transport timing: '+json.dumps(v)+'\n' for v in values)+'Display transport timing: {partial')
            rows=[{'at_ns':8_000_000_000,'wall_ns':offset+8_000_000_000}]
            result=transport_windows(path,rows,7_000_000_000,12_000_000_000)
            self.assertEqual(result['windows'],values[1:3])
            self.assertEqual(result['phase_wall_start_ns'],offset+7_000_000_000)
            self.assertEqual(result['status'],'observed_overlapping_windows')
    def test_missing_clock_mapping_is_unavailable_not_zero_traffic(self):
        result=transport_windows('/nonexistent',[],0,1)
        self.assertEqual(result['status'],'unavailable');self.assertEqual(result['windows'],[])
    def test_invalid_window_and_wrong_record_types_are_ignored(self):
        with tempfile.TemporaryDirectory() as tmp:
            path=Path(tmp)/'app.log'
            path.write_text('\n'.join('Display transport timing: '+json.dumps(v) for v in [[],{'wall_end_ns':True,'window_ms':5000},{'wall_end_ns':10,'window_ms':-1}]))
            result=transport_windows(path,[{'at_ns':1,'wall_ns':101}],0,2)
            self.assertEqual(result['status'],'unavailable')

if __name__=='__main__':unittest.main()
