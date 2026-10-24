import sys,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
from PIL import Image,ImageDraw,ImageFilter
from focus_camera import sharpness,focus_region_candidates,framing_diagnostic
from client import stop_focus_phase

class FocusSharpnessTests(unittest.TestCase):
    def test_geometry_reports_scale_without_treating_it_as_proven_refocusing(self):
        points=[[[10,10],[100,10],[10,100],[100,100]],[[200,10],[300,10],[200,100],[300,100]]]
        scaled=[[[x*1.01-2,y*1.01-1] for x,y in patch] for patch in points]
        result=framing_diagnostic(scaled,points)
        self.assertAlmostEqual(result['common_scale'],1.01)
        self.assertAlmostEqual(result['max_similarity_residual_px'],0)
        self.assertEqual(result['limit_px'],6)
    def test_spanning_candidate_uses_one_region_covering_both_patches(self):
        candidates=dict(focus_region_candidates([[.1,.3,.4,.8],[.6,.2,.9,.7]]))
        self.assertEqual(candidates['span'],[[.1,.2,.9,.8]])
        self.assertEqual(candidates['left'],[[.1,.3,.4,.8]])
        self.assertEqual(candidates['right'],[[.6,.2,.9,.7]])
    def fixture(self):
        image=Image.new('RGB',(400,180),'black');draw=ImageDraw.Draw(image)
        for i,x in enumerate(range(272,352,16)):
            draw.rectangle((x,156,x+15,175),fill='white' if i%2==0 else 'black')
        return image,[[10,30],[374,30],[10,170],[374,170]]
    def test_independent_edge_score_decreases_with_blur(self):
        image,quad=self.fixture()
        sharp=sharpness(image,quad)['edge_slope_per_logical_pixel']
        blurred=sharpness(image.filter(ImageFilter.GaussianBlur(2)),quad)['edge_slope_per_logical_pixel']
        self.assertGreater(sharp,blurred*1.5)
    def test_flat_or_out_of_frame_patch_rejected(self):
        image,quad=self.fixture()
        with self.assertRaisesRegex(ValueError,'contrast'):sharpness(Image.new('RGB',image.size,'gray'),quad)
        with self.assertRaisesRegex(ValueError,'outside'):sharpness(image,[[x+400,y] for x,y in quad])

class FocusPhaseTransitionTests(unittest.TestCase):
    def test_waits_for_owned_worker_before_measured_phase(self):
        class Source:
            def __init__(self):self.calls=[];self.states=iter(['running','running','failed'])
            def request(self,path,body=None):
                self.calls.append(path)
                return {'id':'owned','scene':'warmup','status':next(self.states)} if path=='/status' else {}
        source=Source();stop_focus_phase(source,'owned')
        self.assertEqual(source.calls,['/status','/stop','/status','/status'])
    def test_never_stops_other_phase(self):
        class Source:
            def request(self,path,body=None):
                if path!='/status':raise AssertionError('Must not stop another phase')
                return {'id':'other','scene':'warmup','status':'running'}
        with self.assertRaisesRegex(RuntimeError,'ownership'):stop_focus_phase(Source(),'owned')

if __name__=='__main__':unittest.main()
