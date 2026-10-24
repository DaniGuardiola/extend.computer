import sys,unittest
from pathlib import Path
from PIL import Image,ImageDraw
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'quality'))
import scaled
from registration import normalized_capture
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
from digital_quality import predicted_crop,owned_viewer_window


class ScaledQualityTests(unittest.TestCase):
    def test_adapter_selects_only_window_in_owned_inventory(self):
        class Adapter:
            def quality_window_id(self,app):return app['quality_window_id']
        windows=[{'id':n,'bounds':{'Width':1000,'Height':700}} for n in [10,20]]
        app={'product':'external','quality_window_id':20}
        self.assertEqual(owned_viewer_window(windows,app,Adapter())['id'],20)
        app['quality_window_id']=30
        with self.assertRaises(RuntimeError):owned_viewer_window(windows,app,Adapter())
        with self.assertRaises(RuntimeError):owned_viewer_window(windows,{'product':'extend'},Adapter())
        with self.assertRaises(RuntimeError):owned_viewer_window(windows,app,None)

    def test_inconsistent_corner_translation_retains_rejected_evidence(self):
        reference,fixture,manifest=self.fixture();view=scaled.geometry(fixture,[3456,2168])
        expected,mapped=scaled.expected_raster(reference,manifest,view)
        received=Image.new('RGB',(expected.width+12,expected.height+12),'black');received.paste(expected,(6,6))
        x,y,right,bottom=mapped['fiducial_bounds'][3]
        received.paste(expected.crop((x,y,right,bottom)),(6+x+2,6+y))
        with self.assertRaises(scaled.RegistrationError) as caught:scaled.register(expected,received,mapped,[6,6])
        self.assertFalse(caught.exception.evidence['accepted'])
        self.assertEqual(len(caught.exception.evidence['corners']),4)

    def test_explicit_png_srgb_preserves_pixels_but_untagged_still_fails(self):
        image=Image.new('RGB',(2,2),(64,128,192))
        with self.assertRaises(ValueError):normalized_capture(image)
        image.info['srgb']=0
        result,evidence=normalized_capture(image)
        self.assertEqual(result.tobytes(),image.tobytes())
        self.assertEqual(evidence['input_declaration'],'PNG sRGB chunk')
        for value in [True,4,'0']:
            image.info['srgb']=value
            with self.assertRaises(ValueError):normalized_capture(image)

    def fixture(self):
        image=Image.new('RGB',(2560,1440),'#eeeeee');draw=ImageDraw.Draw(image)
        for x,y in [(4,4),(1248,4),(4,688),(1248,688)]:
            for row in range(4):
                for column in range(4):
                    left=(x+column*7)*2;top=(y+row*7)*2
                    draw.rectangle((left,top,left+13,top+13),fill='white' if (row+column)%2==0 else 'black')
        fixture={'scale':2,'display_logical_size':[1728,1085],
                 'target_in_display_top_left_px':[448,366],'target_pixel_size':[2560,1440]}
        manifest={'pixel_size':[2560,1440],'backing_scale':2,
                  'regions':[{'name':'interior','bounds_px':[100,100,1000,500]}]}
        return image,fixture,manifest

    def test_exact_mode_still_rejects_one_pixel_viewport_difference(self):
        _,fixture,_=self.fixture()
        with self.assertRaises(ValueError):predicted_crop(fixture,[3456,2168])

    def test_known_fractional_viewport_registers_without_fitting_scale(self):
        reference,fixture,manifest=self.fixture();view=scaled.geometry(fixture,[3456,2168])
        self.assertAlmostEqual(view['fit'],1084/1085)
        expected,mapped=scaled.expected_raster(reference,manifest,view)
        received=Image.new('RGB',(expected.width+12,expected.height+12),'black');received.paste(expected,(6,6))
        box,evidence=scaled.register(expected,received,mapped,[6,6])
        self.assertEqual(box,[6,6,expected.width,expected.height]);self.assertFalse(evidence['scale_fitted_from_image'])
        self.assertEqual(mapped['regions'][0]['bounds_px'],[101,101,999,500])

    def test_wrong_scale_and_flipped_fiducials_fail(self):
        reference,fixture,manifest=self.fixture();view=scaled.geometry(fixture,[3456,2168])
        expected,mapped=scaled.expected_raster(reference,manifest,view)
        for altered in [expected.transpose(Image.Transpose.FLIP_LEFT_RIGHT),
                        expected.resize((expected.width-25,expected.height-15),Image.Resampling.BILINEAR)]:
            received=Image.new('RGB',(expected.width+12,expected.height+12),'black');received.paste(altered,(6,6))
            with self.assertRaises(ValueError):scaled.register(expected,received,mapped,[6,6])

if __name__=='__main__':unittest.main()
