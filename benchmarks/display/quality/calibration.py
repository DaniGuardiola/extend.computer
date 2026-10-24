"""Separate synthetic viewport calibration; scored pixels never fit geometry."""
import hashlib,json,math
import numpy as np
from PIL import Image,ImageDraw
from digital import luma


def generate(directory,scale):
    width,height=1280*scale,720*scale
    centers=[(48*scale,48*scale),(1232*scale,48*scale),
             (48*scale,672*scale),(1232*scale,672*scale),
             (640*scale,48*scale),(640*scale,672*scale)]
    image=Image.new('RGB',(width,height),'black');draw=ImageDraw.Draw(image)
    half=16*scale
    for x,y in centers:draw.rectangle((x-half,y-half,x+half-1,y+half-1),fill='white')
    path=directory/'calibration-target.png';image.save(path)
    return {'pixel_size':[width,height],'centers_px':centers,'marker_size_px':half*2,
            'target_sha256':hashlib.sha256(path.read_bytes()).hexdigest()}


def locate(image,centers,expected_size,radius=16):
    values=luma(np.asarray(image.convert('RGB'),dtype=float)/255);points=[]
    half=math.ceil(expected_size/2)+radius
    for cx,cy in centers:
        left,top=math.floor(cx)-half,math.floor(cy)-half
        right,bottom=math.floor(cx)+half+1,math.floor(cy)+half+1
        if left<0 or top<0 or right>image.width or bottom>image.height:
            raise ValueError('Calibration marker search exceeds captured synthetic area')
        patch=values[top:bottom,left:right];mask=patch>.5
        ys,xs=np.nonzero(mask)
        if len(xs)<expected_size**2*.8 or len(xs)>expected_size**2*1.2:
            raise ValueError('Calibration marker missing, clipped, or contaminated')
        x0,x1=xs.min(),xs.max();y0,y1=ys.min(),ys.max()
        if abs((x1-x0+1)-expected_size)>3 or abs((y1-y0+1)-expected_size)>3:
            raise ValueError('Calibration marker has unexpected dimensions')
        if mask.sum()/((x1-x0+1)*(y1-y0+1))<.98:
            raise ValueError('Calibration marker is not a filled rectangle')
        # Weight edge coverage, including antialias pixels, while excluding black
        # background/codec noise. Pixel centers use the continuous edge grid.
        weights=np.where(patch>.08,patch,0)
        xx=np.arange(left,right)+.5;yy=np.arange(top,bottom)+.5
        point=[float(weights.sum(axis=0)@xx/weights.sum()),float(weights.sum(axis=1)@yy/weights.sum())]
        if max(abs(point[0]-cx),abs(point[1]-cy))>radius:
            raise ValueError('Calibration marker moved outside bounded search')
        points.append(point)
    return np.array(points)


def fit(source,received,manifest,fixture,viewer_size,capture_origin=(0,0)):
    backing=np.array(fixture['display_logical_size'])*fixture['scale']
    nominal=min(viewer_size[i]/backing[i] for i in (0,1))
    origin=(np.array(viewer_size)-backing*nominal)/2+np.array(fixture['target_in_display_top_left_px'])*nominal
    centers=np.array(manifest['centers_px'],dtype=float)
    src=locate(source,centers,manifest['marker_size_px'])
    # Viewer chrome may offset content while keeping its aspect-fit scale.
    # Only independent calibration markers use this bounded 32-pixel search;
    # scored image pixels never fit the mapping.
    dst=locate(received,origin+centers*nominal-np.array(capture_origin),manifest['marker_size_px']*nominal,radius=32)+np.array(capture_origin)
    coefficients=[];residuals=[]
    # Only four corner markers fit the model. Two midpoint markers are held out.
    for axis in (0,1):
        matrix=np.stack([src[:4,axis],np.ones(4)],axis=1)
        scale,offset=np.linalg.lstsq(matrix,dst[:4,axis],rcond=None)[0]
        coefficients.append([float(scale),float(offset)])
        residuals.append(dst[:,axis]-(src[:,axis]*scale+offset))
    error=float(np.abs(residuals).max())
    scales=[c[0] for c in coefficients]
    if error>.5:raise ValueError('Independent calibration markers disagree with axis-aligned mapping')
    if any(abs(s/nominal-1)>.02 for s in scales) or abs(scales[0]/scales[1]-1)>.002:
        raise ValueError('Calibration scale outside bounded viewport model')
    return {'scale_xy':scales,'origin_xy':[c[1] for c in coefficients],
            'source_marker_centers_px':src.tolist(),'received_marker_centers_px':dst.tolist(),
            'max_residual_px':error,'held_out_marker_indices':[4,5],
            'model':'Independent calibration fixture; axis-aligned affine, four fit markers and two held-out markers',
            'quality_pixels_used_for_calibration':False,'viewer_pixel_size':list(viewer_size),
            'fixture':fixture,'calibration_target_sha256':manifest['target_sha256']}


def validate_repeats(models):
    if len(models)<2:raise ValueError('Repeated calibration captures required')
    base=models[0];size=np.array(base['fixture']['target_pixel_size'])
    for model in models[1:]:
        delta=np.abs(np.array(model['origin_xy'])-base['origin_xy'])+size*np.abs(np.array(model['scale_xy'])-base['scale_xy'])
        if max(delta)>.5:raise ValueError('Viewport mapping changed across calibration captures')
    return base
