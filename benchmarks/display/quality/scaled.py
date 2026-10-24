"""Known viewport geometry, with independent fiducials; no fitted scale or color."""
import math
import numpy as np
from PIL import Image
from digital import crop,luma

class RegistrationError(ValueError):
    def __init__(self,message,evidence):
        super().__init__(message)
        self.evidence=evidence


def geometry(fixture,viewer_size,margin=6):
    scale=fixture['scale'];backing=[n*scale for n in fixture['display_logical_size']]
    fit=min(viewer_size[i]/backing[i] for i in (0,1))
    if not .5<=fit<=2:raise ValueError('Scaled quality profile outside supported range')
    origin=[(viewer_size[i]-backing[i]*fit)/2+fixture['target_in_display_top_left_px'][i]*fit for i in (0,1)]
    size=fixture['target_pixel_size']
    start=[math.floor(v) for v in origin]
    end=[math.ceil(origin[i]+size[i]*fit) for i in (0,1)]
    box=[start[0]-margin,start[1]-margin,end[0]+margin,end[1]+margin]
    if min(box[:2])<0 or box[2]>viewer_size[0] or box[3]>viewer_size[1]:
        raise ValueError('Scaled target and registration border must fit inside viewer')
    return {'fit':fit,'target_origin_in_viewer_px':origin,'crop_in_viewer_xyxy':box,
            'target_canvas_size':[end[i]-start[i] for i in (0,1)],
            'target_origin_in_canvas_px':[origin[i]-start[i] for i in (0,1)],
            'predicted_target_px':[margin,margin]}

def calibrated_geometry(fixture,viewer_size,model,margin=6):
    for key in ['display_logical_size','scale','target_in_display_top_left_px','target_pixel_size']:
        if fixture[key]!=model['fixture'][key]:raise ValueError('Quality fixture differs from calibrated geometry')
    if list(viewer_size)!=model['viewer_pixel_size']:raise ValueError('Viewer resized after calibration')
    scales=model['scale_xy'];origin=model['origin_xy'];size=fixture['target_pixel_size']
    start=[math.floor(n) for n in origin];end=[math.ceil(origin[i]+size[i]*scales[i]) for i in (0,1)]
    box=[start[0]-margin,start[1]-margin,end[0]+margin,end[1]+margin]
    if min(box[:2])<0 or box[2]>viewer_size[0] or box[3]>viewer_size[1]:raise ValueError('Calibrated target outside viewport')
    return {'fit_xy':scales,'target_origin_in_viewer_px':origin,'crop_in_viewer_xyxy':box,
            'target_canvas_size':[end[i]-start[i] for i in (0,1)],
            'target_origin_in_canvas_px':[origin[i]-start[i] for i in (0,1)],
            'predicted_target_px':[margin,margin],'geometry_source':'Separate calibration fixture'}


def expected_raster(reference,manifest,view):
    sx,sy=view.get('fit_xy',[view.get('fit'),view.get('fit')]);origin=view['target_origin_in_canvas_px'];size=view['target_canvas_size']
    expected=reference.transform(tuple(size),Image.Transform.AFFINE,
        (1/sx,0,-origin[0]/sx,0,1/sy,-origin[1]/sy),
        resample=Image.Resampling.BILINEAR,fillcolor=(0,0,0))
    mapped={**manifest,'pixel_size':size,'regions':[]}
    for region in manifest['regions']:
        bounds=region['bounds_px']
        # Score only interior pixels; fractional region edges can include the
        # neighboring region. Geometry comes solely from observed viewport.
        box=[math.ceil(origin[0]+bounds[0]*sx),math.ceil(origin[1]+bounds[1]*sy),
             math.floor(origin[0]+bounds[2]*sx),math.floor(origin[1]+bounds[3]*sy)]
        mapped['regions'].append({**region,'bounds_px':box})
    mapped['fiducial_bounds']=[]
    for x,y in [(4,4),(1248,4),(4,688),(1248,688)]:
        s=manifest['backing_scale']
        mapped['fiducial_bounds'].append([math.ceil(origin[0]+x*s*sx),math.ceil(origin[1]+y*s*sy),
            math.floor(origin[0]+(x+28)*s*sx),math.floor(origin[1]+(y+28)*s*sy)])
    return expected,mapped


def register(reference,received,manifest,predicted,radius=6):
    evidence=[]
    for x,y,right,bottom in manifest['fiducial_bounds']:
        target=luma(np.asarray(reference.crop((x,y,right,bottom)),dtype=float)/255)
        if target.std()<.3:raise ValueError('Scaled source fiducial lacks contrast')
        candidates=[]
        for dy in range(-radius,radius+1):
            for dx in range(-radius,radius+1):
                try:patch=crop(received,[predicted[0]+dx+x,predicted[1]+dy+y,right-x,bottom-y])
                except ValueError:continue
                values=luma(np.asarray(patch,dtype=float)/255)
                candidates.append((float(np.mean((values-target)**2)),dx,dy))
        candidates.sort()
        if len(candidates)<2 or candidates[0][0]>.04:raise ValueError('Scaled receiver fiducial mismatch')
        score,dx,dy=candidates[0]
        if abs(dx)==radius or abs(dy)==radius or candidates[1][0]-score<.005:
            raise ValueError('Scaled receiver registration ambiguous or outside bound')
        evidence.append({'offset_px':[dx,dy],'luma_mse':score,'runner_up_mse':candidates[1][0]})
    shifts=[e['offset_px'] for e in evidence]
    if any(s!=shifts[0] for s in shifts):
        raise RegistrationError('Scaled corners disagree with observed viewport geometry',{
            'corners':evidence,'search_radius_px':radius,
            'scale_fitted_from_image':False,'accepted':False,
            'reason':'Independent corners require inconsistent translations; declared viewport mapping is not verified'})
    width,height=manifest['pixel_size'];dx,dy=shifts[0]
    box=[predicted[0]+dx,predicted[1]+dy,width,height];crop(received,box)
    return box,{'method':'Four independent checker templates; known viewport scale, bounded integer translation',
                'corners':evidence,'search_radius_px':radius,'scale_fitted_from_image':False}
