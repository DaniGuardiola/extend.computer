"""Observe completed source interaction outcomes in an owned receiver viewer."""
import argparse
import base64
import hashlib
import html
import io
import json
import math
import subprocess
import sys
import time
import uuid
from pathlib import Path
from urllib.parse import urlsplit
import numpy as np
from PIL import Image
from bootstrap import ROOT, build_swift
from bench import product_adapter
from client import Client
from context import application, host, network
from metrics import network_delta, classify_route
from digital_quality import calibrate_viewport, owned_viewer_window
from progress import Progress
sys.path.insert(0, str(ROOT.parent / 'quality'))
from registration import normalized_capture
from digital import metrics


def mapped_reference(reference, received_size, fixture, model):
    if list(received_size)!=model['viewer_pixel_size'] or fixture['logical_size']!=model['fixture']['display_logical_size'] or fixture['scale']!=model['fixture']['scale']:
        raise ValueError('Interaction display or viewer changed after calibration')
    if model.get('quality_pixels_used_for_calibration') is not False or model.get('held_out_marker_indices')!=[4,5] or not 0<=model.get('max_residual_px',float('inf'))<=.5:
        raise ValueError('Independent viewport calibration evidence required')
    sx,sy=model['scale_xy'];ox,oy=model['origin_xy'];tx,ty=model['fixture']['target_in_display_top_left_px']
    if not all(math.isfinite(v) for v in [sx,sy,ox,oy,tx,ty]) or not .5<=min(sx,sy)<=max(sx,sy)<=2:
        raise ValueError('Invalid calibrated interaction transform')
    origin=[ox-tx*sx,oy-ty*sy]
    image=reference.transform(tuple(received_size),Image.Transform.AFFINE,
        (1/sx,0,-origin[0]/sx,0,1/sy,-origin[1]/sy),Image.Resampling.BILINEAR,fillcolor=(0,0,0))
    return image,[sx,sy],origin


def evaluate(reference, received, fixture, model=None):
    scale = fixture['scale']
    logical = fixture['logical_size']
    if scale not in (1, 2) or reference.size != tuple(round(v * scale) for v in logical):
        raise ValueError('Owned source screenshot geometry differs from declared backing size')
    if model is not None:
        reference,scales,origin=mapped_reference(reference,received.size,fixture,model)
    else:
        scales=[1,1];origin=[0,0]
        if received.size != reference.size:
            raise ValueError('Interaction observation requires exact 1:1 receiver presentation')
    def bounds(region):
        x,y,w,h=region
        left=math.ceil(origin[0]+x*scale*scales[0]);top=math.ceil(origin[1]+y*scale*scales[1])
        right=math.floor(origin[0]+(x+w)*scale*scales[0]);bottom=math.floor(origin[1]+(y+h)*scale*scales[1])
        if min(left,top)<0 or right>reference.width or bottom>reference.height or right-left<7 or bottom-top<7:
            raise ValueError('Invalid or clipped owned interaction region')
        return (left,top,right,bottom)
    inset=fixture.get('registration_marker_inset',8)
    if inset not in (8,40):raise ValueError('Unsupported interaction marker geometry')
    trailing=inset+28
    corners = [(inset,inset), (logical[0]-trailing,inset), (inset,logical[1]-trailing), (logical[0]-trailing,logical[1]-trailing)]
    markers = []
    for left, top in corners:
        box=bounds([left,top,28,28]);x,y=box[:2]
        expected = np.asarray(reference.crop(box).convert('L'), dtype=float)/255
        actual = np.asarray(received.crop(box).convert('L'), dtype=float)/255
        if expected.std() < .25 or np.mean((expected-actual)**2) > .01:
            raise ValueError('Interaction corner registration failed; wrong image, scale or placement')
        markers.append({'top_left_px': [x,y], 'luma_mse': float(np.mean((expected-actual)**2))})
    rows = []
    for name, region in fixture['visual_regions'].items():
        box=bounds(region)
        expected = np.asarray(reference.crop(box).convert('RGB'))
        actual = np.asarray(received.crop(box).convert('RGB'))
        score = metrics(Image.fromarray(expected), Image.fromarray(actual))['luma_ssim_box11']
        # Whole-region SSIM alone can conceal missing glyphs in a mostly blank
        # text field. Verify foreground contrast occupancy independently.
        weights = np.array([.2126, .7152, .0722])
        source_luma = expected @ weights
        receiver_luma = actual @ weights
        background = float(np.median(source_luma))
        source_ink = np.abs(source_luma - background) > 8
        receiver_ink = np.abs(receiver_luma - background) > 8
        union = int(np.logical_or(source_ink, receiver_ink).sum())
        overlap = float(np.logical_and(source_ink, receiver_ink).sum()/union) if union else 0.0
        rows.append({'region': name, 'luma_ssim_box11': score,
            'contrast_foreground_iou': overlap, 'source_foreground_pixels': int(source_ink.sum()),
            'accepted': score >= .95 and union >= 100 and overlap >= .8, 'bounds_px': list(box)})
    return {'accepted': all(row['accepted'] for row in rows), 'regions': rows,
        'registration': {'method': ('Independent viewport calibration followed by four held-out interaction checker corners; source-only bilinear reference' if model else 'Four checker corners at exact predicted raster coordinates; no resampling'), 'scale_fitted_from_scored_pixels':False, 'markers': markers},
        'thresholds': {'luma_ssim_box11': .95, 'contrast_foreground_iou': .8, 'minimum_union_pixels': 100, 'foreground_luma_delta': 8}, 'scope': 'Completed button/text/drag/scroll outcomes in viewer compositor screenshot; not transition latency or physical display quality'}


def execute(client, product, out, tools, adapter, allow_scaled=False):
    out.mkdir(parents=True, exist_ok=False, mode=0o700)
    ident = str(uuid.uuid4()); started = False
    report = {'status':'running','kind':'interaction_visual','product':product,
        'limits':['Final visible outcomes only; transient typing and focus transitions not independently observed',
            'Screenshot observes viewer compositor, not panel scanout, hardware input or input-to-photon latency',
            'SSIM threshold is a diagnostic gate, not proof of fidelity parity']}
    try:
        app = application(product, adapter)
        before = client.request('/context/' + product)
        peer = urlsplit(client.config['url']).hostname
        receiver_before = network(app, peer)
        report['source_context'] = before
        report['receiver_context'] = {'host':host(),'application':app}
        if product == 'extend':
            viewers = [v['pid'] for v in app['display_helpers'] if v['role']=='viewer']
            if len(viewers)!=1: raise ValueError('Exactly one native viewer required')
            viewer = viewers[0]
        elif adapter and hasattr(adapter,'quality_viewer_pid'):
            viewer = adapter.quality_viewer_pid(app)
            if viewer not in app['pids']: raise ValueError('Viewer not owned by product')
        else: raise ValueError('Owned viewer adapter required')
        if not (adapter and hasattr(adapter,'focus_viewer') and adapter.focus_viewer(app,tools/'campaign-workload')):
            subprocess.run([str(tools/'campaign-workload'),'--activate',str(viewer)],check=True,stdout=subprocess.DEVNULL)
        windows = json.loads(subprocess.check_output([str(tools/'quality-fixture'),'--windows',str(viewer)],text=True))
        window=owned_viewer_window(windows,app,adapter)
        report['viewer_window'] = window
        report['scaled_viewer_mode']=allow_scaled
        externals=[screen for screen in before['displays'] if not screen['builtin'] and not screen['mirrored']]
        if allow_scaled and len(externals)!=1:raise ValueError('Exactly one unmirrored calibration display required')
        model=calibrate_viewport(client,product,window,out,externals[0]['scale'],Progress()) if allow_scaled else None
        if model:
            report['viewport_calibration']=model
            report['limits'].append('Source-only bilinear reference under separately measured viewport mapping; not original-pixel fidelity')
        reply = client.request('/interaction/start',{'id':ident,'product':product,'screen':'auto','observe':True})
        started = True
        if reply.get('id')!=ident: raise ValueError('Interaction start ID mismatch')
        deadline = time.monotonic()+10
        while time.monotonic()<deadline:
            ready = client.request('/interaction/state',{'id':ident})
            if ready.get('id')!=ident: raise ValueError('Interaction state ID mismatch')
            if ready.get('status')=='ready': break
            if ready.get('status')!='running': raise RuntimeError(ready.get('error','Interaction snapshot unavailable'))
            time.sleep(.15)
        else: raise TimeoutError('Interaction snapshot not ready')
        raw = base64.b64decode(ready.pop('reference_base64'),validate=True)
        if len(raw)>8_000_000 or hashlib.sha256(raw).hexdigest()!=ready['reference_sha256']:
            raise ValueError('Interaction reference hash or size mismatch')
        (out/'reference.png').write_bytes(raw)
        report['snapshot'] = ready
        time.sleep(.7)
        temporary = out/'viewer-temporary.png'
        try:
            subprocess.run(['/usr/sbin/screencapture','-x','-o','-l',str(window['id']),'-t','png',str(temporary)],check=True)
            with Image.open(io.BytesIO(raw)) as source, Image.open(temporary) as received:
                # Centered letterboxing can preserve a 1:1 source raster. Reject
                # scaling, fractional offsets and missing corner markers.
                w,h=source.size; dx=(received.width-w)/2;dy=(received.height-h)/2
                if model:
                    actual=received.copy();dx=dy=0
                else:
                    if min(dx,dy)<0 or dx!=round(dx) or dy!=round(dy): raise ValueError('Viewer is scaled or has fractional placement')
                    actual=received.crop((round(dx),round(dy),round(dx)+w,round(dy)+h))
                ref,ref_color=normalized_capture(source); image,image_color=normalized_capture(actual)
                report['color_management']={'source':ref_color,'receiver':image_color}
                report['viewer_crop']=[round(dx),round(dy),actual.width,actual.height]
                # Preserve this owned synthetic fixture when registration fails;
                # the temporary full window capture is still removed below.
                actual.save(out/'received.png',icc_profile=received.info.get('icc_profile',b''))
                report['visual']=evaluate(ref,image,ready['fixture'],model)
                if not report['visual']['accepted']: raise ValueError('One or more receiver interaction regions differ')
        finally: temporary.unlink(missing_ok=True)
        # Cleanup/native exit is acceptance evidence, not just a requested stop.
        completed=client.request('/interaction/stop',{'id':ident});started=False
        report['source_outcome']=completed
        if completed.get('id')!=ident:raise ValueError('Interaction cleanup ID mismatch')
        if completed.get('status')!='complete':raise RuntimeError(completed.get('error','Source cleanup failed'))
        after=client.request('/context/'+product);receiver_after=application(product,adapter)
        if after['application']['main_pid']!=before['application']['main_pid'] or receiver_after['main_pid']!=app['main_pid']:
            raise ValueError('Product restarted during observation')
        if after['displays']!=before['displays']:raise ValueError('Source display inventory changed')
        summary={'source':network_delta(before['network'],after['network']),
            'receiver':network_delta(receiver_before,network(receiver_after,peer))}
        report['network_summary']=summary;report['route']=classify_route(summary)
        if report['route']['classification']!='direct_lan':raise ValueError('Direct LAN not confirmed')
        report['status']='complete'
    except BaseException as error:
        report['status']='failed';report['error']=str(error) or type(error).__name__
        raise
    finally:
        if started:
            try:report['cleanup']=client.request('/interaction/stop',{'id':ident})
            except Exception as error:report['cleanup_error']=str(error)
        (out/'report.json').write_text(json.dumps(report,indent=2))
        (out/'report.html').write_text('<meta charset="utf-8"><title>Interaction visual check</title>'
            '<h1>Interaction visual check</h1><p>'+html.escape(report['status'])+'</p><pre>'+
            html.escape(json.dumps(report.get('visual',{}),indent=2))+'</pre><p>'+html.escape(report.get('error',''))+'</p>')
    return report


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--product',default='extend')
    p.add_argument('--pairing',type=Path,default=ROOT/'.cache/campaign/source-pairing.json')
    p.add_argument('--output',type=Path,required=True);p.add_argument('--no-open',action='store_true');p.add_argument('--allow-scaled',action='store_true',help='Use separately calibrated fullscreen viewport mapping; strict exact geometry remains default');a=p.parse_args()
    out=a.output.expanduser().resolve()
    try:execute(Client(a.pairing),a.product,out,build_swift(ROOT.parents[2]),product_adapter(),a.allow_scaled)
    finally:
        if (out/'report.html').exists() and not a.no_open:subprocess.run(['open',str(out/'report.html')],check=False)
    print('Interaction visual checks completed successfully. Results: '+str(a.output/'report.html'))

if __name__=='__main__':main()
