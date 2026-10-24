"""Cursor checks reject wrong shapes, locations and stale visible state."""
import sys,unittest
from pathlib import Path
from PIL import Image
sys.path.insert(0,str(Path(__file__).resolve().parent))
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
from cursor_quality import evaluate,viewer_position,mapped_cursor
from cursor_session import CursorController
from types import SimpleNamespace

class CursorRasterTests(unittest.TestCase):
    def test_calibrated_viewer_chrome_preserves_hotspot_and_density_checks(self):
        ready={'scale':2,'logical_size':[512,384]};size=(1024,816)
        model={'viewer_pixel_size':list(size),'scale_xy':[.996,.996],'origin_xy':[20.22,70.12],
            'fixture':{'display_logical_size':[512,384],'scale':2,'target_in_display_top_left_px':[20,20]},
            'quality_pixels_used_for_calibration':False,'held_out_marker_indices':[4,5],'max_residual_px':.1}
        image=Image.new('RGB',(128,128),'black');image.paste(Image.new('RGB',(112,112),'magenta'),(8,8))
        raster,position,evidence=mapped_cursor(image,size,ready,[200,300],model)
        actual=Image.new('RGB',size,'black');actual.paste(raster,tuple(position))
        evaluate(actual,raster,position,True)
        self.assertFalse(evidence['geometry_fitted_from_received_image'])
        with self.assertRaises(ValueError):evaluate(actual,raster,[position[0],position[1]+2],True)
        for changed in [{**model,'held_out_marker_indices':[]},{**model,'viewer_pixel_size':[1024,768]},
                        {**model,'quality_pixels_used_for_calibration':True}]:
            with self.assertRaises(ValueError):mapped_cursor(image,size,ready,[200,300],changed)
    def test_explicit_scaled_geometry_preserves_wrong_hotspot_gate(self):
        ready={'scale':2,'logical_size':[1728,1085]}
        image=Image.new('RGB',(128,128),'black');image.paste(Image.new('RGB',(112,112),'magenta'),(8,8))
        raster,position,evidence=mapped_cursor(image,(3456,2168),ready,[848,1068])
        actual=Image.new('RGB',(3456,2168),'black');actual.paste(raster,tuple(position))
        evaluate(actual,raster,position,True)
        self.assertFalse(evidence['geometry_fitted_from_received_image'])
        self.assertAlmostEqual(evidence['fit'],1084/1085)
        with self.assertRaises(ValueError):viewer_position(actual.size,ready,[848,1068])
        with self.assertRaises(ValueError):evaluate(actual,raster,[position[0]+2,position[1]],True)
    def test_video_density_does_not_change_logical_cursor_placement(self):
        self.assertEqual(viewer_position((1024,768),{'scale':1,'logical_size':[512,384]},[100,200]),[200,400])
        self.assertEqual(viewer_position((1024,768),{'scale':2,'logical_size':[512,384]},[200,400]),[200,400])
        for size,scale in [((1024,700),2),((512,384),1),((1024,768),3)]:
            with self.assertRaises(ValueError):viewer_position(size,{'scale':scale,'logical_size':[512,384]},[100,200])
    def test_shape_hotspot_and_absence(self):
        cursor=Image.new('RGB',(128,128),(255,64,255));actual=Image.new('RGB',(512,512),'black');actual.paste(cursor,(100,200))
        self.assertTrue(evaluate(actual,cursor,[100,200],True)['exact_match'])
        for position in [[108,200],[100,208]]:
            with self.assertRaises(ValueError):evaluate(actual,cursor,position,True)
        with self.assertRaises(ValueError):evaluate(actual,Image.new('RGB',(128,128),'cyan'),[100,200],True)
        with self.assertRaises(ValueError):evaluate(actual,cursor,[0,0],False)
        self.assertFalse(evaluate(Image.new('RGB',(512,512),'black'),cursor,[0,0],False)['visible'])
    def test_flipped_asymmetric_shape_is_rejected(self):
        cursor=Image.new('RGB',(128,128),'magenta');cursor.paste(Image.new('RGB',(32,32),'cyan'),(8,8))
        actual=Image.new('RGB',(512,512),'black');actual.paste(cursor.transpose(Image.Transpose.FLIP_TOP_BOTTOM),(100,200))
        with self.assertRaisesRegex(ValueError,'shape/hotspot'):evaluate(actual,cursor,[100,200],True)
    def test_stale_duplicate_colored_cursor_is_rejected(self):
        cursor=Image.new('RGB',(128,128),'magenta');actual=Image.new('RGB',(512,512),'black')
        actual.paste(cursor,(100,200));actual.paste(cursor,(300,200))
        with self.assertRaisesRegex(ValueError,'outside'):evaluate(actual,cursor,[100,200],True)
    def test_control_does_not_accept_paths_or_arbitrary_states(self):
        controller=CursorController(SimpleNamespace());controller.ident='owned'
        for body in [{'id':'other','state':'hidden'},{'id':'owned','state':'execute'},{'id':'owned','state':'hidden','path':'/tmp/arbitrary'}]:
            with self.assertRaises(ValueError):controller.state(body)
        for body in [{'id':'owned','product':'extend','screen':'auto','input_method':'run-shell'},
                     {'id':'owned','product':'extend','screen':'auto','input_method':'warp','command':'anything'}]:
            with self.assertRaises(ValueError):controller.start(body)
if __name__=='__main__':unittest.main()
