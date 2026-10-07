import unittest
from benchmarks.display.optical.decode import checksum, decode

class OpticalTests(unittest.TestCase):
    def test_all_counter_values(self):
        for value in range(4096):
            word=value<<4|checksum(value)
            row=[220 if word&(1<<(15-i)) else 20 for i in range(16)]
            self.assertEqual(decode([row,[240-x for x in row]]),value)
    def test_blur_rejected(self):
        self.assertIsNone(decode([[127]*16,[128]*16]))
    def test_single_bit_corruption_rejected(self):
        word=123<<4|checksum(123)
        for bit in range(16):
            damaged=word^(1<<bit)
            row=[230 if damaged&(1<<(15-i)) else 10 for i in range(16)]
            self.assertIsNone(decode([row,[240-x for x in row]]))
    def test_shape(self):
        with self.assertRaises(ValueError):decode([[0]])
