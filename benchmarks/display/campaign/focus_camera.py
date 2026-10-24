"""Choose an autofocus region using both physical timing patches, then save it."""
import argparse,json,statistics,subprocess,sys,uuid
from pathlib import Path
import numpy as np
from PIL import Image
from bootstrap import ROOT,adb_at,build_swift
from bench import load,product_adapter,select_phone
from client import Client,stop_focus_phase
from context import application
from find_patches import find

def select_focus_candidate(baseline,candidates):
    valid=[v for v in [baseline,*candidates] if v.get('status')=='verified']
    if not valid:raise RuntimeError('No verified focus candidate; adjust camera/screens and retry')
    return max(valid,key=lambda v:v['balanced_score'])

def focus_region_candidates(regions):
    left,right=regions
    span=[min(left[0],right[0]),min(left[1],right[1]),
          max(left[2],right[2]),max(left[3],right[3])]
    # One spanning region also works on cameras that allow only one AF region.
    return [('left',[left]),('right',[right]),('span',[span])]

def framing_diagnostic(patches,reference):
    current=np.asarray(patches,dtype=float).reshape(-1,2)
    before=np.asarray(reference,dtype=float).reshape(-1,2)
    centered_before=before-before.mean(axis=0)
    centered_current=current-current.mean(axis=0)
    scale=float((centered_before*centered_current).sum()/(centered_before**2).sum())
    translation=current.mean(axis=0)-scale*before.mean(axis=0)
    return {'max_corner_shift_px':float(np.linalg.norm(current-before,axis=1).max()),
            'common_scale':scale,'translation_px':translation.tolist(),
            'max_similarity_residual_px':float(np.linalg.norm(current-(scale*before+translation),axis=1).max()),
            'limit_px':6,'limits':['A common scale change cannot distinguish refocusing from physical camera movement.']}

def sharpness(image,quad):
    # Homography from the fixture's known marker centers, independently of strips.
    rows=[];values=[]
    for (x,y),(u,v) in zip([(10,150),(374,150),(10,10),(374,10)],quad):
        rows.extend([[x,y,1,0,0,0,-u*x,-u*y],[0,0,0,x,y,1,-v*x,-v*y]]);values.extend([u,v])
    h=np.append(np.linalg.solve(rows,values),1).reshape(3,3)
    xs=np.arange(274,350,.25);ys=np.arange(12,21,1)
    x,y=np.meshgrid(xs,ys);p=h@np.stack([x.ravel(),y.ravel(),np.ones(x.size)])
    u=(p[0]/p[2]).reshape(x.shape);v=(p[1]/p[2]).reshape(x.shape)
    a=np.asarray(image).astype(float).mean(axis=2)/255
    ix=np.floor(u).astype(int);iy=np.floor(v).astype(int);dx=u-ix;dy=v-iy
    if ix.min()<0 or iy.min()<0 or ix.max()+1>=a.shape[1] or iy.max()+1>=a.shape[0]:raise ValueError('Strip outside frame')
    profile=((1-dx)*(1-dy)*a[iy,ix]+dx*(1-dy)*a[iy,ix+1]+(1-dx)*dy*a[iy+1,ix]+dx*dy*a[iy+1,ix+1]).mean(axis=0)
    contrast=np.percentile(profile,90)-np.percentile(profile,10)
    if contrast<.2:raise ValueError('Insufficient strip contrast')
    gradients=np.abs(np.gradient(profile,.25))/contrast
    sharp=float(np.median([max(gradients[abs(xs-edge)<4]) for edge in [288,304,320,336]]))
    return {'edge_slope_per_logical_pixel':sharp,'contrast':float(contrast)}

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--product',required=True)
    p.add_argument('--pairing',type=Path,default=ROOT/'.cache/campaign/source-pairing.json')
    p.add_argument('--phone');p.add_argument('--output',type=Path,required=True)
    p.add_argument('--camera-zoom',type=float,default=1.0)
    a=p.parse_args();adapter=product_adapter()
    from capture_phone import camera_zoom
    zoom=camera_zoom(a.camera_zoom)
    if a.product!='extend' and not (adapter and a.product in adapter.products()):p.error('Product adapter unavailable')
    config=load();adb=adb_at(config.get('adb'));phone=select_phone(adb,a.phone,config.get('phone_identity'))
    client=Client(a.pairing);client.request('/context/'+a.product)
    tools=build_swift(ROOT.parents[2]);app=application(a.product,adapter)
    root=a.output.expanduser().resolve();root.mkdir(parents=True,exist_ok=False)
    report={'status':'running','camera_zoom':zoom,'candidates':[],'limits':['Selects among autofocus-region candidates on the current rig, not every possible lens distance.','Edge response includes physical panels and camera; not a display-quality score.','Recalibrate after moving the camera or screens.']}
    initial=None
    def capture(name,regions):
        nonlocal initial
        state=client.request('/status')
        if state.get('status') not in ('idle','complete','failed'):raise RuntimeError('Source already has a running workload')
        if not (adapter and hasattr(adapter,'focus_viewer') and adapter.focus_viewer(app,tools/'campaign-workload')):
            viewers=[h['pid'] for h in app.get('display_helpers',[]) if h['role']=='viewer']
            subprocess.run([str(tools/'campaign-workload'),'--activate',str(viewers[0] if len(viewers)==1 else app['main_pid'])],check=True,stdout=subprocess.DEVNULL)
        ident=str(uuid.uuid4());folder=root/name
        try:
            client.request('/start',{'id':ident,'product':a.product,'scene':'warmup','seconds':60,'screen':'auto'})
            argv=[sys.executable,str(ROOT/'capture_phone.py'),'--serial',phone,'--adb',str(adb),'--seconds','3','--quiet','--probe',str(tools/'probe'),'--output',str(folder)]
            argv+=['--zoom',str(zoom)]
            if regions is not None:
                path=root/(name+'-regions.json');path.write_text(json.dumps(regions));argv+=['--focus-regions',str(path)]
            subprocess.run(argv,check=True)
            duration=json.loads((folder/'video.json').read_text())['duration_s'];frames=[];scores=[]
            for n,fraction in enumerate((.25,.5,.75)):
                frame=folder/f'frame-{n}.png'
                try:
                    subprocess.run([str(tools/'extract'),str(folder/'raw/video.mp4'),str(frame),str(duration*fraction)],check=True)
                    patches=find(frame)
                    if len(patches)!=2:raise ValueError('Both physical timing patches must be visible')
                    if initial is not None:
                        geometry=framing_diagnostic(patches,initial)
                        (folder/f'framing-{n}.json').write_text(json.dumps(geometry,indent=2))
                        if geometry['max_corner_shift_px']>6:
                            raise ValueError(f"Camera framing changed during focus selection ({geometry['max_corner_shift_px']:.2f} px; limit 6 px)")
                    image=Image.open(frame).convert('RGB')
                    scores.append([sharpness(image,q) for q in patches]);frames.append(patches)
                    for i,q in enumerate(patches):
                        x=[v[0] for v in q];y=[v[1] for v in q]
                        image.crop((min(x)-15,min(y)-15,max(x)+15,max(y)+20)).save(folder/f'patch-{n}-{i}.png')
                finally:frame.unlink(missing_ok=True)
            if initial is None:initial=frames[1]
            medians=[statistics.median(v[i]['edge_slope_per_logical_pixel'] for v in scores) for i in (0,1)]
            result={'candidate':name,'status':'verified','regions':regions,'frames':scores,'median_slopes_left_to_right':medians,'balanced_score':min(medians),'focus':json.loads((folder/'summary.json').read_text())['focus']}
            (folder/'focus-score.json').write_text(json.dumps(result,indent=2));return result
        finally:
            if client.request('/status').get('id')==ident:stop_focus_phase(client,ident)
    try:
        report['baseline']=capture('baseline',None)
        regions=[]
        for q in initial:
            x=[v[0]/1920 for v in q];y=[v[1]/1080 for v in q]
            regions.append([max(0,min(x)-.025),max(0,min(y)-.025),min(1,max(x)+.025),min(1,max(y)+.025)])
        for name,value in focus_region_candidates(regions):
            print('Focus candidate: '+name,flush=True)
            try:result=capture(name,value)
            except (ValueError,RuntimeError,subprocess.SubprocessError) as error:result={'candidate':name,'status':'rejected','reason':str(error)}
            report['candidates'].append(result)
        best=select_focus_candidate(report['baseline'],report['candidates'])
        path=None
        if best['regions'] is not None:
            path=root/'selected-focus-regions.json';path.write_text(json.dumps(best['regions'],indent=2))
        distance=best['focus']['locked_distance_diopters']
        distance_range=[max(0,distance-.1),distance+.1]
        distance_path=root/'selected-focus-distance-range.json';distance_path.write_text(json.dumps(distance_range,indent=2))
        report.update(status='focus_region_selected',selected_candidate=best['candidate'],selected_regions=str(path) if path else None,selected_distance_range=str(distance_path))
        region_arg=' --focus-regions '+str(path) if path else ''
        zoom_arg=' --camera-zoom '+str(zoom)
        print('Focus calibrated. Use campaign'+region_arg+zoom_arg,flush=True)
        print('Optional experimental distance guard: --focus-distance-range '+str(distance_path)+'. A matching distance does not prove image sharpness.',flush=True)
    except BaseException as error:
        report.update(status='failed',reason=str(error));raise
    finally:(root/'report.json').write_text(json.dumps(report,indent=2))

if __name__=='__main__':main()
