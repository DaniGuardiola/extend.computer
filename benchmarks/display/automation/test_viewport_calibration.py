import sys,tempfile,unittest
from pathlib import Path
from PIL import Image,ImageDraw
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'quality'))
import calibration,scaled

class ViewportCalibrationTests(unittest.TestCase):
    def setup_fixture(self,directory):
        manifest=calibration.generate(Path(directory),2)
        source=Image.open(Path(directory)/'calibration-target.png').convert('RGB')
        fixture={'scale':2,'display_logical_size':[1728,1085],
            'target_in_display_top_left_px':[448,366],'target_pixel_size':[2560,1440]}
        sx,sy=.9962,.9963;ox,oy=452.0,368.0
        received=source.transform((3456,2168),Image.Transform.AFFINE,
            (1/sx,0,-ox/sx,0,1/sy,-oy/sy),Image.Resampling.BILINEAR)
        return manifest,source,fixture,received,[sx,sy],[ox,oy]

    def test_independent_calibration_recovers_mapping_and_rejects_resize(self):
        with tempfile.TemporaryDirectory() as directory:
            m,s,f,r,scales,origin=self.setup_fixture(directory)
            model=calibration.fit(s,r,m,f,r.size)
            for a,b in zip(model['scale_xy'],scales):self.assertAlmostEqual(a,b,places=4)
            for a,b in zip(model['origin_xy'],origin):self.assertLess(abs(a-b),.15)
            self.assertFalse(model['quality_pixels_used_for_calibration'])
            self.assertEqual(model['held_out_marker_indices'],[4,5])
            calibration.validate_repeats([model,model])
            scaled.calibrated_geometry(f,r.size,model)
            with self.assertRaises(ValueError):scaled.calibrated_geometry(f,[3456,2166],model)
            changed={**model,'origin_xy':[origin[0]+2,origin[1]]}
            with self.assertRaises(ValueError):calibration.validate_repeats([model,changed])

    def test_held_out_marker_change_cannot_refit_mapping(self):
        with tempfile.TemporaryDirectory() as directory:
            m,s,f,r,scales,origin=self.setup_fixture(directory)
            x,y=m['centers_px'][4];cx=round(origin[0]+x*scales[0]);cy=round(origin[1]+y*scales[1])
            draw=ImageDraw.Draw(r);draw.rectangle((cx-40,cy-40,cx+40,cy+40),fill='black')
            draw.rectangle((cx-31+4,cy-31,cx+31+4,cy+31),fill='white')
            with self.assertRaisesRegex(ValueError,'markers disagree'):calibration.fit(s,r,m,f,r.size)

    def test_viewer_chrome_translation_uses_only_bounded_calibration_search(self):
        with tempfile.TemporaryDirectory() as directory:
            m,s,f,r,scales,origin=self.setup_fixture(directory)
            def shifted(delta):
                return r.transform(r.size,Image.Transform.AFFINE,(1,0,0,0,1,-delta),Image.Resampling.NEAREST)
            model=calibration.fit(s,shifted(24),m,f,r.size)
            self.assertLess(abs(model['origin_xy'][1]-origin[1]-24),.15)
            self.assertLessEqual(model['max_residual_px'],.5)
            with self.assertRaises(ValueError):calibration.fit(s,shifted(40),m,f,r.size)

    def test_missing_or_extra_marker_content_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            m,s,f,r,_,_=self.setup_fixture(directory)
            for bad in [Image.new('RGB',r.size,'black'),Image.new('RGB',r.size,'white')]:
                with self.assertRaises(ValueError):calibration.fit(s,bad,m,f,bad.size)

if __name__=='__main__':unittest.main()
