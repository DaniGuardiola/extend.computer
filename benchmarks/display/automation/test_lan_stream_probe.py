import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
from lan_stream_probe import packet,decode,HEADER,SIZE

class StreamProbeTests(unittest.TestCase):
    def test_synthetic_train_identity_is_not_arbitrary_payload(self):
        self.assertEqual(decode(packet(600,7,8),8),(600,7))
        data=bytearray(packet(1,0,8));data[-1]=1
        with self.assertRaisesRegex(ValueError,'payload'):decode(bytes(data),8)
    def test_truncation_wrong_chunk_count_and_invalid_part_are_rejected(self):
        for value in [packet(1,0,8)[:-1],packet(1,0,4),
                      HEADER.pack(b'EXSP',1,8,8)+bytes(SIZE-HEADER.size)]:
            with self.subTest(size=len(value)):
                with self.assertRaises(ValueError):decode(value,8)

if __name__=='__main__':unittest.main()
