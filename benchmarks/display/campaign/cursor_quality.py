"""End-to-end synthetic cursor raster, hotspot, containment and visibility checks."""
import argparse,base64,html,io,json,math,subprocess,sys,time,uuid
from pathlib import Path
from urllib.parse import urlsplit
import numpy as np
from PIL import Image
from bootstrap import ROOT,build_swift
from bench import product_adapter
from client import Client
from context import application,network
from metrics import network_delta,classify_route
from progress import Progress
from cursor_session import STATES
sys.path.insert(0,str(ROOT.parent/'quality'))
from registration import normalized_capture
from digital_quality import owned_viewer_window,calibrate_viewport
from interaction_visual import mapped_reference


def colored_mask(pixels):
    return ((np.minimum(pixels[...,0],pixels[...,2])-pixels[...,1]>.4)&(pixels[...,0]>.7))|((np.minimum(pixels[...,1],pixels[...,2])-pixels[...,0]>.4)&(pixels[...,1]>.7))

def viewer_position(image_size,ready,position,allow_scaled=False):
    # Cursor placement is in logical points, independently of video density.
    # The fixture stores source backing coordinates, while this screenshot is
    # the receiver's 2x composition. Do not scale the cursor raster with video.
    if ready['scale'] not in (1,2):raise ValueError('Unsupported source cursor density')
    if allow_scaled:
        backing=[v*2 for v in ready['logical_size']]
        fit=min(image_size[i]/backing[i] for i in (0,1))
        if not .5<=fit<=2:raise ValueError('Cursor viewport scale outside supported range')
        return [(image_size[i]-backing[i]*fit)/2+position[i]*2/ready['scale']*fit for i in (0,1)]
    if list(image_size)!=[round(v*2) for v in ready['logical_size']]:
        raise ValueError('Viewer must match source logical geometry at receiver scale 2')
    return [v*2/ready['scale'] for v in position]

def mapped_cursor(expected,image_size,ready,position,model=None):
    if model:
        # Reuse independent marker validation, never locate the scored cursor.
        _,scales,offset=mapped_reference(Image.new('RGB',(1,1)),image_size,ready,model)
        point=[offset[i]+position[i]*scales[i] for i in (0,1)]
        fits=[scales[i]*ready['scale']/2 for i in (0,1)]
    else:
        point=viewer_position(image_size,ready,position,True)
        fits=[min(image_size[i]/(ready['logical_size'][i]*2) for i in (0,1))]*2
    origin=[math.floor(v) for v in point]
    size=[math.ceil(point[i]+expected.size[i]*fits[i])-origin[i] for i in (0,1)]
    raster=expected.transform(tuple(size),Image.Transform.AFFINE,
        (1/fits[0],0,-(point[0]-origin[0])/fits[0],0,1/fits[1],-(point[1]-origin[1])/fits[1]),
        Image.Resampling.BILINEAR,fillcolor='black')
    return raster,origin,{'method':('Independent repeated viewport calibration; source-only bilinear cursor reference' if model else 'Known whole-viewport aspect fit; bilinear source cursor reference only'),
        'fit':fits[0] if fits[0]==fits[1] else None,'fit_xy':fits,'fractional_top_left_px':point,'pixel_size':size,
        'geometry_fitted_from_received_image':False}

def evaluate(actual,expected,position,visible):
    pixels=np.asarray(actual.convert('RGB'),dtype=float)/255
    mask=colored_mask(pixels)
    if not visible:
        if mask.sum()>4:raise ValueError('Cursor remains visible while hidden or outside display')
        return {'visible':False,'colored_pixels':int(mask.sum())}
    x,y=map(round,position);w,h=expected.size
    if x<0 or y<0 or x+w>actual.width or y+h>actual.height:raise ValueError('Expected cursor bounds outside viewer')
    reference=np.asarray(expected.convert('RGB'),dtype=float)/255
    received=pixels[y:y+h,x:x+w];mse=float(np.mean((received-reference)**2))
    score=None if mse==0 else -10*math.log10(mse)
    if score is not None and score<30:raise ValueError(f'Cursor shape/hotspot mismatch: {score:.2f} dB')
    if mask.sum()<100:raise ValueError('Synthetic cursor not visible')
    # Placement/duplication uses the declared cursor raster bounds. Comparing
    # color-threshold masks inside those bounds would conflate tiny independent
    # calibration/resampling differences with an extra cursor. Full raster
    # PSNR above still verifies shape and hotspot inside the bounds.
    expected_mask=np.zeros(mask.shape,dtype=bool);expected_mask[y:y+h,x:x+w]=True
    if (mask & ~expected_mask).sum()>8:raise ValueError('Cursor colored pixels outside expected hotspot bounds')
    return {'visible':True,'rgb_psnr_db':score,'exact_match':mse==0,'colored_pixels':int(mask.sum()),'expected_top_left_px':[x,y]}


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--product',required=True)
    p.add_argument('--allow-scaled',action='store_true',help='Explicit known whole-viewport cursor aspect-fit diagnostic; strict exact geometry remains default')
    p.add_argument('--calibrate-viewport',action='store_true',help='Use independent repeated viewport markers; requires --allow-scaled')
    p.add_argument('--input-method',choices=['warp','mouse-events'],default='warp',help='Source pointer warp or OS-posted mouse movement; neither is physical hardware input')
    p.add_argument('--collect-failures',action='store_true',help='Retain failed raster states and continue safe state checks; thresholds and failed exit status stay unchanged')
    p.add_argument('--pairing',type=Path,default=ROOT/'.cache/campaign/source-pairing.json');p.add_argument('--output',type=Path,required=True);p.add_argument('--no-open',action='store_true')
    a=p.parse_args()
    if a.calibrate_viewport and not a.allow_scaled:p.error('--calibrate-viewport requires --allow-scaled')
    out=a.output.expanduser().resolve();out.mkdir(parents=True,exist_ok=False,mode=0o700)
    report={'status':'running','kind':'cursor_raster','product':a.product,'states':[],
        'limits':['Synthetic cursor only; static state checks, not input-to-photon latency or optical cursor cadence.',('Requires source scale 1 or 2 and receiver scale 2. Explicit scaled mode uses declared whole-viewport aspect-fit geometry with a source-only bilinear reference; default mode requires matching fullscreen logical geometry.' if a.allow_scaled else 'Requires source scale 1 or 2, receiver scale 2, and matching fullscreen logical geometry.'),'Screenshot includes viewer composition, not physical panel scanout.']}
    client=Client(a.pairing);adapter=product_adapter();tools=build_swift(ROOT.parents[2]);ident=str(uuid.uuid4());started=False
    ui=Progress();ui.heading(a.product,'cursor raster')
    try:
        app=application(a.product,adapter);before=client.request('/context/'+a.product);peer=urlsplit(client.config['url']).hostname
        receiver_before=network(app,peer);report['source_context']=before;report['receiver_application']=app
        if a.product=='extend':
            viewers=[v['pid'] for v in app['display_helpers'] if v['role']=='viewer']
            if len(viewers)!=1:raise ValueError('Exactly one native viewer required')
            viewer=viewers[0]
        elif adapter and hasattr(adapter,'quality_viewer_pid'):viewer=adapter.quality_viewer_pid(app)
        else:raise ValueError('Owned viewer adapter required')
        if not (adapter and hasattr(adapter,'focus_viewer') and adapter.focus_viewer(app,tools/'campaign-workload')):
            subprocess.run([str(tools/'campaign-workload'),'--activate',str(viewer)],check=True,stdout=subprocess.DEVNULL)
        windows=json.loads(subprocess.check_output([str(tools/'quality-fixture'),'--windows',str(viewer)],text=True))
        window=owned_viewer_window(windows,app,adapter)
        model=None
        if a.calibrate_viewport:
            external=[d for d in before['displays'] if not d['builtin'] and not d['mirrored']]
            if len(external)!=1:raise ValueError('Exactly one unmirrored calibration display required')
            model=calibrate_viewport(client,a.product,window,out,external[0]['scale'],ui)
            report['viewport_calibration']=model
            report['limits'].append('Independent repeated marker mapping replaces nominal viewport centering; scored cursor pixels never determine geometry.')
        ready=client.request('/cursor/start',{'id':ident,'product':a.product,'screen':'auto','input_method':a.input_method});started=True;report['fixture']=ready
        if ready.get('input_method')!=a.input_method:raise ValueError('Source cursor input method differs')
        if ready['scale'] not in (1,2):raise ValueError('Unsupported source cursor density')
        for state in STATES:
            with ui.step(state+' capture' if a.collect_failures else state):
                reply=client.request('/cursor/state',{'id':ident,'state':state});time.sleep(.7)
                if reply['display_id']!=ready['display_id'] or reply['logical_size']!=ready['logical_size'] or reply['scale']!=ready['scale'] or reply.get('input_method')!=ready['input_method']:raise ValueError('Cursor fixture geometry or input method changed')
                expected=Image.open(io.BytesIO(base64.b64decode(reply.pop('cursor_png_base64'),validate=True))).convert('RGBA')
                expected.save(out/(state+'-expected.png'))
                record={'requested':state,'source':reply,'status':'checking'};report['states'].append(record)
                black=Image.new('RGBA',expected.size,(0,0,0,255));black.alpha_composite(expected);expected=black.convert('RGB')
                if expected.size!=(128,128):raise ValueError('Unexpected synthetic cursor raster')
                temporary=out/'viewer-temporary.png'
                try:
                    subprocess.run(['/usr/sbin/screencapture','-x','-o','-l',str(window['id']),'-t','png',str(temporary)],check=True)
                    with Image.open(temporary) as image:
                        if a.allow_scaled:
                            expected,position,mapping=mapped_cursor(expected,image.size,ready,reply['cursor_top_left_px'],model)
                            record['reference_mapping']=mapping
                        else:position=viewer_position(image.size,ready,reply['cursor_top_left_px'])
                        normalized,color=normalized_capture(image)
                        mask=colored_mask(np.asarray(normalized.convert('RGB'),dtype=float)/255)
                        ys,xs=np.nonzero(mask)
                        record['received_cursor_color_observation']={'colored_pixels':int(mask.sum()),
                            'bounds_px':[int(xs.min()),int(ys.min()),int(xs.max())+1,int(ys.max())+1] if len(xs) else None}
                        if reply['visible']:
                            x,y=map(round,position);normalized.crop((x,y,x+expected.width,y+expected.height)).save(out/(state+'.png'))
                        try:
                            result=evaluate(normalized,expected,position,reply['visible'])
                        except ValueError as error:
                            if not a.collect_failures:raise
                            record.update({'status':'failed','error':str(error),'color_management':color})
                            ui.line(ui.paint('×','31')+'  '+state+' raster check: '+str(error))
                            continue
                        # Only the synthetic cursor crop is retained above; the full viewer file is deleted.
                finally:temporary.unlink(missing_ok=True)
                record.update({'status':'passed','result':result,'color_management':color})
        after=client.request('/context/'+a.product);receiver_after=application(a.product,adapter)
        if before['application']['main_pid']!=after['application']['main_pid'] or app['main_pid']!=receiver_after['main_pid']:raise ValueError('Product restarted during cursor checks')
        summary={'source':network_delta(before['network'],after['network']),'receiver':network_delta(receiver_before,network(receiver_after,peer))}
        report['route']=classify_route(summary);report['network_summary']=summary
        if report['route']['classification']!='direct_lan':raise ValueError('Direct LAN not proven during cursor checks')
        failed=[row['requested'] for row in report['states'] if row['status']=='failed']
        report['status']='failed' if failed else 'complete_with_declared_limits'
        if failed:report['error']='Cursor raster states failed: '+', '.join(failed)
    except BaseException as error:report['status']='failed';report['error']=str(error);raise
    finally:
        if started:
            try:report['cleanup']=client.request('/cursor/stop',{'id':ident})
            except Exception as error:report['cleanup']={'error':str(error)}
            if not all(report['cleanup'].get(k) is True for k in ['position_restored','visibility_restored']):
                report['status']='failed';report.setdefault('error','Cursor restoration failed')
        (out/'report.json').write_text(json.dumps(report,indent=2))
        (out/'report.html').write_text('<meta charset="utf-8"><title>Cursor raster checks</title><h1>Cursor raster checks</h1><pre>'+html.escape(json.dumps(report,indent=2))+'</pre>')
        if not a.no_open:subprocess.run(['open',str(out/'report.html')],check=False)
    if report['status']=='failed':raise RuntimeError(report['error'])
    print('Cursor raster checks completed successfully. Results: '+str(out/'report.html'))
if __name__=='__main__':main()
