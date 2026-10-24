"""Reject stale/missing receiver content rather than accepting blank background."""
import sys
import unittest
from pathlib import Path
import numpy as np
from PIL import Image, ImageDraw
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
from interaction_visual import evaluate


class VisualInteractionTests(unittest.TestCase):
    def fixture(self,inset=8):
        width,height=1100,750
        image=Image.new('RGB',(width,height),'white');draw=ImageDraw.Draw(image)
        trailing=inset+28
        for x,y in [(inset,inset),(width-trailing,inset),(inset,height-trailing),(width-trailing,height-trailing)]:
            for row in range(4):
                for column in range(4):
                    left=x+column*7;top=y+row*7
                    draw.rectangle((left,top,left+6,top+6),fill='black' if (row+column)%2 else 'white')
        # Sparse foreground in a large mostly blank text field.
        for x in range(90,220,20):draw.rectangle((x,145,x+6,158),fill='black')
        return image,{'logical_size':[width,height],'scale':1,'registration_marker_inset':inset,'visual_regions':{'replacement':[80,140,360,36]}}

    def test_inset_markers_avoid_indicator_but_still_require_all_four(self):
        reference,fixture=self.fixture(40)
        received=reference.copy();ImageDraw.Draw(received).rectangle((0,0,65,20),fill='purple')
        self.assertTrue(evaluate(reference,received,fixture)['accepted'])
        ImageDraw.Draw(received).rectangle((40,40,67,67),fill='white')
        with self.assertRaisesRegex(ValueError,'registration'):evaluate(reference,received,fixture)
        with self.assertRaisesRegex(ValueError,'Unsupported'):evaluate(reference,reference,{**fixture,'registration_marker_inset':41})

    def test_matching_content_passes_but_removed_glyphs_fail(self):
        reference,fixture=self.fixture()
        self.assertTrue(evaluate(reference,reference.copy(),fixture)['accepted'])
        received=reference.copy();ImageDraw.Draw(received).rectangle((80,140,439,175),fill='white')
        result=evaluate(reference,received,fixture)
        self.assertFalse(result['accepted'])
        self.assertEqual(result['regions'][0]['contrast_foreground_iou'],0)

    def test_wrong_registration_and_rescaling_are_rejected(self):
        reference,fixture=self.fixture();wrong=reference.copy()
        ImageDraw.Draw(wrong).rectangle((8,8,35,35),fill='white')
        with self.assertRaisesRegex(ValueError,'registration'):evaluate(reference,wrong,fixture)
        with self.assertRaisesRegex(ValueError,'1:1'):evaluate(reference,reference.resize((550,375)),fixture)

    def test_separate_calibration_preserves_missing_content_failure(self):
        reference,fixture=self.fixture();sx,sy=.996,.996;ox,oy=.3,.2;size=(1098,748)
        model={'scale_xy':[sx,sy],'origin_xy':[ox+100*sx,oy+80*sy],
            'fixture':{'display_logical_size':fixture['logical_size'],'scale':1,'target_in_display_top_left_px':[100,80]},
            'viewer_pixel_size':list(size),'quality_pixels_used_for_calibration':False,
            'held_out_marker_indices':[4,5],'max_residual_px':.1}
        def render(image):return image.transform(size,Image.Transform.AFFINE,(1/sx,0,-ox/sx,0,1/sy,-oy/sy),Image.Resampling.BILINEAR)
        result=evaluate(reference,render(reference),fixture,model)
        self.assertTrue(result['accepted'])
        self.assertFalse(result['registration']['scale_fitted_from_scored_pixels'])
        missing=reference.copy();ImageDraw.Draw(missing).rectangle((80,140,439,175),fill='white')
        self.assertFalse(evaluate(reference,render(missing),fixture,model)['accepted'])
        for changed in [{**model,'viewer_pixel_size':[1099,748]},
                        {**model,'quality_pixels_used_for_calibration':True},
                        {**model,'held_out_marker_indices':[]},
                        {**model,'max_residual_px':1}]:
            with self.assertRaises(ValueError):evaluate(reference,render(reference),fixture,changed)
        wrong=render(reference).transform(size,Image.Transform.AFFINE,(1,0,-3,0,1,0))
        with self.assertRaisesRegex(ValueError,'registration'):evaluate(reference,wrong,fixture,model)

    def test_changed_foreground_fails_even_with_same_dark_pixel_count(self):
        reference,fixture=self.fixture();wrong=reference.copy();draw=ImageDraw.Draw(wrong)
        draw.rectangle((80,140,439,175),fill='white')
        for x in range(250,380,20):draw.rectangle((x,145,x+6,158),fill='black')
        self.assertFalse(evaluate(reference,wrong,fixture)['accepted'])

if __name__=='__main__':unittest.main()
