"""Automated synthetic source/viewer digital quality checks, independently of camera timing."""
import argparse,base64,hashlib,html,json,subprocess,sys,time,uuid
from pathlib import Path
from PIL import Image
from bootstrap import ROOT,build_swift
from bench import product_adapter
from client import Client
from context import application,host,digest,network
from metrics import network_delta,classify_route
from urllib.parse import urlsplit
from progress import Progress
sys.path.insert(0,str(ROOT.parent/'quality'))
from digital import generate,compare
from registration import normalized_capture,register
import scaled
import calibration


def predicted_crop(fixture,received_size):
    scale=fixture['scale'];logical=fixture['display_logical_size']
    backing=[n*scale for n in logical]
    fit=min(received_size[0]/backing[0],received_size[1]/backing[1])
    if abs(fit-1)>1e-8:
        raise ValueError('Viewer scales source pixels; exact matched-raster diagnostic requires 1:1 presentation')
    offset=[(received_size[i]-backing[i])/2+fixture['target_in_display_top_left_px'][i] for i in (0,1)]
    if any(abs(x-round(x))>1e-8 for x in offset):
        raise ValueError('Viewer has fractional raster placement')
    return [round(n) for n in offset]

def owned_viewer_window(windows,app,adapter):
    windows=[w for w in windows if w['bounds']['Width']>=640 and w['bounds']['Height']>=480]
    if adapter and hasattr(adapter,'quality_window_id') and app['product']!='extend':
        selected=adapter.quality_window_id(app)
        windows=[w for w in windows if w['id']==selected]
    if len(windows)!=1:raise RuntimeError('Exactly one visible owned viewer window required')
    return windows[0]

def calibrate_viewport(client,product,window,out,scale,ui):
    manifest=calibration.generate(out,scale);ident=str(uuid.uuid4());started=False
    try:
        reply=client.request('/quality/start',{'id':ident,'product':product,'screen':'auto',
            'image_base64':base64.b64encode((out/'calibration-target.png').read_bytes()).decode()});started=True
        if reply['target_sha256']!=manifest['target_sha256']:raise ValueError('Calibration source target hash mismatch')
        (out/'calibration-fixture.json').write_text(json.dumps(reply['fixture'],indent=2))
        data=base64.b64decode(reply['reference_base64'],validate=True)
        if hashlib.sha256(data).hexdigest()!=reply['reference_sha256']:raise ValueError('Calibration source capture hash mismatch')
        (out/'calibration-reference.png').write_bytes(data);time.sleep(1)
        captures=[]
        for i in range(2):
            with ui.step(f'viewport calibration capture {i+1}/2'):
                temporary=out/'calibration-window-temporary.png'
                try:
                    subprocess.run(['/usr/sbin/screencapture','-x','-o','-l',str(window['id']),'-t','png',str(temporary)],check=True)
                    with Image.open(temporary) as image:
                        view=scaled.geometry(reply['fixture'],image.size,margin=32);box=view['crop_in_viewer_xyxy']
                        path=out/f'calibration-received-{i+1}.png'
                        image.crop(box).save(path,icc_profile=image.info.get('icc_profile',b''))
                        captures.append((path,list(image.size),box[:2]))
                finally:temporary.unlink(missing_ok=True)
                time.sleep(.3)
    finally:
        if started:client.request('/quality/stop',{'id':ident})
    with Image.open(out/'calibration-reference.png') as image:source,_=normalized_capture(image)
    models=[]
    for path,size,origin in captures:
        with Image.open(path) as image:received,_=normalized_capture(image)
        model=calibration.fit(source,received,manifest,reply['fixture'],size,origin)
        model['received_sha256']=digest(path);model['reference_sha256']=digest(out/'calibration-reference.png');models.append(model)
    result=calibration.validate_repeats(models)
    (out/'calibration.json').write_text(json.dumps({'captures':models,'accepted_mapping':result},indent=2))
    return result

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--product',required=True)
    parser.add_argument('--pairing',type=Path,default=ROOT/'.cache/campaign/source-pairing.json')
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--font',type=Path,default=Path('/System/Library/Fonts/Supplemental/Arial.ttf'))
    parser.add_argument('--scale',type=int,choices=[1,2,3],default=2)
    parser.add_argument('--no-open',action='store_true')
    parser.add_argument('--allow-scaled',action='store_true',help='Explicit compositor-quality mode using observed viewport geometry and a bilinear source reference; not exact-raster quality')
    parser.add_argument('--calibrate-viewport',action='store_true',help='Use a separate calibration fixture with repeated captures and held-out markers; requires --allow-scaled')
    args=parser.parse_args();out=args.output.expanduser().resolve();out.mkdir(parents=True,exist_ok=False,mode=0o700)
    if args.calibrate_viewport and not args.allow_scaled:parser.error('--calibrate-viewport requires --allow-scaled')
    ui=Progress();ui.heading(args.product,'digital quality');tools=build_swift(ROOT.parents[2])
    adapter=product_adapter();app=application(args.product,adapter)
    if args.product=='extend':
        viewers=[item['pid'] for item in app['display_helpers'] if item['role']=='viewer']
        if len(viewers)!=1:raise RuntimeError('Exactly one native viewer required')
        viewer=viewers[0]
    elif adapter and hasattr(adapter,'quality_viewer_pid'):
        viewer=adapter.quality_viewer_pid(app)
        if viewer not in app['pids']:raise RuntimeError('Adapter viewer does not belong to prepared product')
    else:raise RuntimeError('Product adapter must identify its owned quality viewer')
    if not (adapter and hasattr(adapter,'focus_viewer') and adapter.focus_viewer(app,tools/'campaign-workload')):
        subprocess.run([str(tools/'campaign-workload'),'--activate',str(viewer)],check=True,stdout=subprocess.DEVNULL)
    windows=json.loads(subprocess.check_output([str(tools/'quality-fixture'),'--windows',str(viewer)],text=True))
    window=owned_viewer_window(windows,app,adapter)
    manifest=generate(out,args.font,args.scale);client=Client(args.pairing)
    source_context=client.request('/context/'+args.product)
    receiver_network=network(app,urlsplit(client.config['url']).hostname)
    report={'schema_version':1,'status':'running','product':args.product,'kind':'digital_raster_quality',
        'source_context':source_context,'receiver_context':{'host':host(),'application':app},'samples':[],
        'capture_geometry':[],'viewer_window':window,'scaled_viewer_mode':args.allow_scaled,
        'limits':['Static synthetic raster only; motion quality and physical panels require separate tests',
            'ICC-normalized 8-bit screenshots include OS compositor and capture color transforms',
            'Scaled mode uses a declared bilinear source reference, not original-pixel fidelity or physical panel quality' if args.allow_scaled else
            'Exact 1:1 pixel presentation required; scaled-viewer quality is not covered by this command']}
    ident=str(uuid.uuid4());started=False
    try:
        model=calibrate_viewport(client,args.product,window,out,args.scale,ui) if args.calibrate_viewport else None
        if model:report['viewport_calibration']=model
        with ui.step('source synthetic fixture'):
            reply=client.request('/quality/start',{'id':ident,'product':args.product,'screen':'auto',
                'image_base64':base64.b64encode((out/'target.png').read_bytes()).decode()});started=True
            if reply['target_sha256']!=manifest['target_sha256']:raise RuntimeError('Source fixture hash differs')
            source=base64.b64decode(reply['reference_base64'],validate=True)
            if hashlib.sha256(source).hexdigest()!=reply['reference_sha256']:raise RuntimeError('Reference hash differs')
            (out/'reference.png').write_bytes(source);report['fixture']=reply['fixture']
        time.sleep(1)
        # Capture all samples before CPU-heavy scoring. Capture only the owned
        # viewer; the source fixture covers its entire streamed desktop.
        for index in range(3):
            with ui.step(f'viewer capture {index+1}/3'):
                path=out/f'received-{index+1}.png'
                # screencapture silently refuses a hidden basename on macOS,
                # even with exit status zero. Delete this full-window file below.
                temporary=out/'viewer-capture-temporary.png'
                try:
                    subprocess.run(['/usr/sbin/screencapture','-x','-o','-l',str(window['id']),'-t','png',str(temporary)],check=True)
                    with Image.open(temporary) as image:
                        margin=6
                        view=(scaled.calibrated_geometry(reply['fixture'],image.size,model) if model else
                              scaled.geometry(reply['fixture'],image.size)) if args.allow_scaled else None
                        if view:box=view['crop_in_viewer_xyxy']
                        else:
                            predicted=predicted_crop(reply['fixture'],image.size)
                            margin=6;x,y=predicted;w,h=manifest['pixel_size']
                            box=[x-margin,y-margin,x+w+margin,y+h+margin]
                        if min(box[:2])<0 or box[2]>image.width or box[3]>image.height:
                            raise ValueError('Synthetic target and registration margin must fit inside viewer')
                        image.crop(box).save(path,icc_profile=image.info.get('icc_profile',b''))
                        report['capture_geometry'].append({'viewer_pixel_size':list(image.size),
                            'crop_in_viewer_xyxy':box,'predicted_target_px':[margin,margin],
                            'scaled_geometry':view,'scope':'Only synthetic target and bounded black border retained'})
                finally:temporary.unlink(missing_ok=True)
                if index<2:time.sleep(.5)
        source_after=client.request('/context/'+args.product)
        receiver_after=application(args.product,adapter)
        if source_after['application']['main_pid']!=source_context['application']['main_pid'] or receiver_after['main_pid']!=app['main_pid']:
            raise ValueError('Product restarted during digital captures')
        display=[d for d in source_after['displays'] if d['id']==reply['fixture']['display_id']]
        if len(display)!=1 or [display[0]['width'],display[0]['height']]!=reply['fixture']['display_logical_size'] or display[0]['scale']!=reply['fixture']['scale']:
            raise ValueError('Source geometry changed during captures')
        summary={'source':network_delta(source_context['network'],source_after['network']),
            'receiver':network_delta(receiver_network,network(receiver_after,urlsplit(client.config['url']).hostname))}
        report['network_summary']=summary;report['route']=classify_route(summary)
        if report['route']['classification']!='direct_lan':raise ValueError('Direct LAN not confirmed during digital quality capture')
        client.request('/quality/stop',{'id':ident});started=False
        with Image.open(out/'reference.png') as raw_reference:
            reference,reference_color=normalized_capture(raw_reference)
        for index in range(3):
            with ui.step(f'quality scoring {index+1}/3'):
                path=out/f'received-{index+1}.png'
                with Image.open(path) as raw_received:
                    received,received_color=normalized_capture(raw_received)
                predicted=report['capture_geometry'][index]['predicted_target_px']
                view=report['capture_geometry'][index]['scaled_geometry']
                scoring_reference,scoring_manifest=scaled.expected_raster(reference,manifest,view) if view else (reference,manifest)
                box,registration=(scaled.register if view else register)(scoring_reference,received,scoring_manifest,predicted)
                scored=compare(scoring_reference,received,scoring_manifest,[0,0,*scoring_manifest['pixel_size']],box,
                    'Four checker fiducials verified; known viewport bilinear reference; receiver pixels unchanged; ICC conversion to sRGB' if view else
                    'Four checker fiducials verified; integral 1:1 compositor geometry; ICC conversion to sRGB')
                if view:
                    scored['comparison_kind']='scaled_compositor_raster'
                    scored['registration']['resampling']='Source reference only: known viewport affine, bilinear in sRGB; receiver unchanged'
                    scored['limitations'].append('Does not measure detail lost solely through viewport scaling; bilinear sRGB is a declared reference model, not verified OS interpolation')
                scored['registration_evidence']=registration
                scored['color_management']={'source':reference_color,'received':received_color}
                scored['input_sha256']={'reference':digest(out/'reference.png'),'received':digest(path),'target':manifest['target_sha256']}
                report['samples'].append(scored)
        report['status']='complete_with_declared_limits'
        print('Digital quality checks completed successfully.',flush=True)
    except BaseException as error:
        if isinstance(error,scaled.RegistrationError):report['registration_failure']=error.evidence
        report['status']='failed';report['error']=str(error);raise
    finally:
        if started:
            try:client.request('/quality/stop',{'id':ident})
            except Exception as error:report['cleanup_error']=str(error);report['status']='failed'
        (out/'report.json').write_text(json.dumps(report,indent=2,allow_nan=False))
        body='<meta charset="utf-8"><title>Digital display quality</title><style>body{font:16px system-ui;max-width:1100px;margin:40px auto}td,th{padding:8px;text-align:left}pre{white-space:pre-wrap}</style><h1>Digital display quality</h1><p>'+html.escape(report['status'])+'</p>'
        if report.get('error'):body+='<p>'+html.escape(report['error'])+'</p>'
        for i,sample in enumerate(report['samples'],1):
            body+=f'<h2>Capture {i}</h2><table><tr><th>Region</th><th>Luma SSIM</th><th>RGB PSNR dB</th><th>Edge RMSE</th><th>Chroma RMSE</th></tr>'
            for name,row in sample['regions'].items():
                values=[name,*[row[k] for k in ['luma_ssim_box11','rgb_psnr_db','edge_gradient_rmse','chroma_rmse_bt709']]]
                body+='<tr>'+''.join('<td>'+html.escape(str(v) if isinstance(v,str) or v is None else f'{v:.5f}')+'</td>' for v in values)+'</tr>'
            body+='</table>'
        body+='<h2>Context and validity evidence</h2><pre>'+html.escape(json.dumps(report,indent=2))+'</pre>'
        (out/'report.html').write_text(body)
        if not args.no_open:subprocess.run(['open',str(out/'report.html')],check=False)
        print('Results: '+str(out/'report.html'))

if __name__=='__main__':main()
