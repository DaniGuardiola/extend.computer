"""Strict integer translation registration; never rescales scored rasters."""
import hashlib
import io
import numpy as np
from PIL import ImageCms
from digital import crop, luma


def normalized_capture(image):
    profile = image.info.get('icc_profile')
    if not profile:
        intent=image.info.get('srgb')
        if type(intent) is int and intent in (0,1,2,3):
            return image.convert('RGB'), {'input_declaration':'PNG sRGB chunk','png_rendering_intent':intent,
                'output_space':'sRGB','transform':'Explicit sRGB source; no color conversion',
                'geometry_resampled':False}
        raise ValueError('Capture lacks an ICC profile; color pipeline cannot be verified')
    source = ImageCms.ImageCmsProfile(io.BytesIO(profile))
    result = ImageCms.profileToProfile(image, source, ImageCms.createProfile('sRGB'), outputMode='RGB',
        renderingIntent=ImageCms.Intent.RELATIVE_COLORIMETRIC)
    return result, {'input_icc_sha256': hashlib.sha256(profile).hexdigest(),
        'output_space': 'sRGB', 'transform': 'ICC relative colorimetric, 8-bit RGB',
        'geometry_resampled': False}


def register(reference, received, manifest, predicted, radius=6):
    width, height = manifest['pixel_size']
    if reference.size != (width, height):
        raise ValueError('Source capture dimensions differ from fixture backing size')
    scale = manifest['backing_scale']
    corners = [(4,4),(1248,4),(4,688),(1248,688)]
    evidence = []
    for left, top in corners:
        x, y, side = left * scale, top * scale, 28 * scale
        target = luma(np.asarray(reference.crop((x,y,x+side,y+side)),dtype=float)/255)
        yy, xx = np.indices(target.shape)
        white = ((xx // (7*scale) + yy // (7*scale)) % 2) == 0
        contrast = float(target[white].mean() - target[~white].mean())
        if contrast < .65:
            raise ValueError('Source fiducial orientation, raster scale or contrast differs')
        candidates = []
        for dy in range(-radius,radius+1):
            for dx in range(-radius,radius+1):
                box = [predicted[0]+dx+x,predicted[1]+dy+y,side,side]
                try: patch = crop(received,box)
                except ValueError: continue
                values = luma(np.asarray(patch,dtype=float)/255)
                candidates.append((float(np.mean((values-target)**2)),dx,dy))
        candidates.sort()
        if len(candidates)<2 or candidates[0][0]>.04:
            raise ValueError('Receiver fiducial does not match source at expected scale')
        score,dx,dy=candidates[0]
        if abs(dx)==radius or abs(dy)==radius or candidates[1][0]-score<.005:
            raise ValueError('Receiver registration is ambiguous or outside bounded translation')
        evidence.append({'offset_px':[dx,dy],'luma_mse':score,'runner_up_mse':candidates[1][0],
            'source_checker_contrast':contrast})
    shifts=np.asarray([row['offset_px'] for row in evidence])
    if not np.all(shifts==shifts[0]):
        raise ValueError('Corners disagree: scale, deformation or crop mismatch')
    dx,dy=shifts[0].tolist()
    box=[predicted[0]+dx,predicted[1]+dy,width,height]
    crop(received,box)
    return box, {'method':'Four independent checker templates; exact integer translation only',
        'search_radius_px':radius,'corners':evidence,'resampling':'none'}
