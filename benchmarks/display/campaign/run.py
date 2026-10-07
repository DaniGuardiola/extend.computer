"""Repeatable two-host campaign: baseline, warm-up, scenes, optical capture and report."""
import argparse,datetime,hashlib,html,json,os,shutil,signal,subprocess,sys,threading,time,uuid
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE));sys.path.insert(0,str(HERE.parent/'automation'))
from client import Client
from context import host,application,sample,network,digest
from stages import snapshot
from metrics import resources,optical,quality,stats,source_patch,network_delta,classify_route
from bootstrap import ROOT,build_swift
from bench import load,save,adb_at,select_phone,phone_identity,product_adapter

def write(path,data):
    temporary=path.with_name(path.name+'.tmp')
    temporary.write_text(json.dumps(data,indent=2,allow_nan=False));temporary.replace(path)
class Sampler:
    def __init__(self,app):self.app=app;self.rows=[];self.event=threading.Event();self.thread=threading.Thread(target=self.loop,daemon=True);self.error=None
    def loop(self):
        try:
            while not self.event.is_set():self.rows.append(sample(self.app));self.event.wait(.5)
        except Exception as e:self.error=str(e)
    def __enter__(self):self.thread.start();return self
    def __exit__(self,*_):self.event.set();self.thread.join(timeout=20)
    def summary(self):
        if self.error:raise RuntimeError('Resource sampler failed: '+self.error)
        if len(self.rows)<3 or not all(r['main_alive'] for r in self.rows):raise RuntimeError('Application stopped during measurement')
        return resources(self.rows)

def profile(context,screen):
    displays=context['displays'];targets=[d for d in displays if not d['builtin'] and not d['mirrored']]
    if screen!='auto':targets=[d for d in targets if str(d['id'])==screen]
    if len(targets)!=1:raise RuntimeError('Exactly one unmirrored extended display required; select --screen ID')
    return targets[0]

def verify_source(result,initial,screen,expected_build):
    current=profile(result['context'],screen)
    if current!=initial:raise RuntimeError('Source display geometry changed')
    if result['context']['application']['executable_sha256']!=expected_build:raise RuntimeError('Product build changed')
    if result.get('fixture') and result['fixture']['source_callback_hz']<50 and result['scene']!='static':raise RuntimeError('Source cannot sustain workload cadence')

def artifact_inventory(root):
    return {str(p.relative_to(root)):digest(p) for p in root.rglob('*') if p.is_file() and p.name not in {'manifest.json','report.html'}}

def open_report(path):
    try:
        subprocess.Popen(['/usr/bin/open',str(path.resolve())],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,start_new_session=True)
    except OSError as error:
        print('Could not open report automatically: '+str(error),file=sys.stderr)

def render(root,report):
    rows=[]
    for phase in report['phases']:
        metrics=phase.get('optical') or {};lat=(metrics.get('screen_to_screen_transition_ms') or {}).get('median')
        source=(phase['resources']['source']['cpu_percent_one_core'] or {}).get('median');receiver=(phase['resources']['receiver']['cpu_percent_one_core'] or {}).get('median')
        def number(v):return 'unavailable' if v is None else f'{v:.2f}'
        rows.append('<tr>'+''.join('<td>'+html.escape(str(v))+'</td>' for v in [phase['repetition'],phase['scene'],phase.get('route',{}).get('label','Unclassified'),number(metrics.get('receiver_visible_hz')),number(lat),number(source),number(receiver),number(phase.get('quality',{}).get('global_optical_ssim')),phase['status']])+'</tr>')
    body='<html><meta charset="utf-8"><title>Display benchmark</title><style>body{font:16px system-ui;margin:40px;max-width:1100px}table{border-collapse:collapse}td,th{border:1px solid #ccc;padding:10px}pre{white-space:pre-wrap}</style>'
    status={'setup_check_passed':'Setup check passed','complete_with_declared_limits':'Benchmark completed successfully','failed':'Benchmark failed'}.get(report['status'],report['status'])
    body+='<h1>Display benchmark · '+html.escape(report['product'])+'</h1><p><strong>'+html.escape(status)+'</strong></p>'
    if report.get('failure'):body+='<p>'+html.escape(report['failure'])+'</p>'
    body+='<table><tr><th>Repeat</th><th>Scene</th><th>Route</th><th>Visible updates/s</th><th>Screen-to-screen median ms</th><th>Source CPU % one core</th><th>Receiver CPU % one core</th><th>Optical strip SSIM</th><th>Status</th></tr>'+''.join(rows)+'</table>'
    body+='<h2>Per-scene resource, network and stage evidence</h2>'
    for phase in report['phases']:
        body+='<details><summary>'+html.escape(str(phase['repetition'])+' '+phase['scene'])+'</summary><pre>'+html.escape(json.dumps(phase,indent=2))+'</pre></details>'
    body+='<h2>Context, baseline and evidence limits</h2><pre>'+html.escape(json.dumps({k:v for k,v in report.items() if k!='phases'},indent=2))+'</pre></html>'
    (root/'report.html').write_text(body)

def validate_resume(previous,current,root):
    if previous['product']!=current['product']:raise RuntimeError('Resume product differs')
    if previous['status'] not in {'failed','running'}:raise RuntimeError('Only interrupted or failed campaigns can resume')
    for key in ['renderer','seconds','repetitions','mode','workload_sha256']:
        if previous['settings'].get(key)!=current['settings'].get(key):raise RuntimeError('Resume setting differs: '+key)
    for role in ['source','receiver']:
        old,new=previous['context'][role],current['context'][role]
        for key in ['model','chip','memory_bytes','os','architecture']:
            if old['host'].get(key)!=new['host'].get(key):raise RuntimeError('Resume hardware/OS differs: '+role+' '+key)
        if old['application']['executable_sha256']!=new['application']['executable_sha256']:raise RuntimeError('Resume product build differs: '+role)
        if old['host'].get('power',{}).get('source')!=new['host'].get('power',{}).get('source'):raise RuntimeError('Resume power source differs: '+role)
    keys=['width','height','scale','pixel_width','pixel_height','refresh_hz']
    if any(previous['context']['source_display'].get(k)!=current['context']['source_display'].get(k) for k in keys):raise RuntimeError('Resume source geometry differs')
    def panels(report):return [{k:d.get(k) for k in keys} for d in report['context']['receiver'].get('displays',[]) if d.get('builtin')]
    if panels(previous)!=panels(current):raise RuntimeError('Resume receiver panel differs')
    if previous['context']['phone']!=current['context']['phone']:raise RuntimeError('Resume phone differs')
    manifest=json.loads((root/'manifest.json').read_text())['artifacts']
    if manifest.get('report.json')!=digest(root/'report.json'):raise RuntimeError('Resume report integrity check failed')
    completed=set()
    for phase in previous['phases']:
        key=(phase['repetition'],phase['scene'])
        if phase['status']!='measured' or key in completed:raise RuntimeError('Invalid saved scene checkpoint')
        if key[0] not in range(1,previous['settings']['repetitions']+1) or key[1] not in {'static','scroll','panel','motion','recovery'}:raise RuntimeError('Unknown saved scene checkpoint')
        prefix=f'{key[0]:02}-{key[1]}/'
        artifacts={name:value for name,value in manifest.items() if name.startswith(prefix)}
        if prefix+'source.json' not in artifacts or prefix+'camera/raw/video.mp4' not in artifacts:raise RuntimeError('Saved scene evidence missing')
        for name,value in artifacts.items():
            path=root/name
            if not path.is_file() or digest(path)!=value:raise RuntimeError('Saved scene integrity check failed: '+name)
        completed.add(key)
    return completed

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--product',required=True,type=str);p.add_argument('--pairing',type=Path,default=ROOT/'.cache/campaign/source-pairing.json')
    p.add_argument('--screen',default='auto');p.add_argument('--seconds',type=int);p.add_argument('--repetitions',type=int);p.add_argument('--output',type=Path)
    p.add_argument('--resume',type=Path,help='Continue a failed campaign, preserving measured scenes and old attempts')
    p.add_argument('--no-open',action='store_true',help='Do not open the HTML report automatically')
    p.add_argument('--smoke',action='store_true',help='Five-second single-scene setup check; never a comparison result');p.add_argument('--check',action='store_true');p.add_argument('--source-index',type=int,choices=[0,1],default=None)
    a=p.parse_args()
    previous=None
    if a.resume:
        if a.output or a.smoke:p.error('--resume cannot be combined with --output or --smoke')
        a.resume=a.resume.expanduser().resolve();previous=json.loads((a.resume/'report.json').read_text())
    if a.seconds is None:a.seconds=previous['settings']['seconds'] if previous else 30
    if a.repetitions is None:a.repetitions=previous['settings']['repetitions'] if previous else 3
    if a.smoke:a.seconds=5;a.repetitions=1
    if not 5<=a.seconds<=54 or not 1<=a.repetitions<=10:p.error('seconds must be 5..54; repetitions 1..10')
    adapter=product_adapter()
    if a.product!='extend' and not (adapter and a.product in adapter.products()):raise RuntimeError('Product adapter unavailable')
    client=Client(a.pairing)
    health=client.request('/health');source=client.request('/context/'+a.product)
    tools=build_swift(ROOT.parents[2]);source_display=profile(source,a.screen)
    if health['workload_sha256']!=digest(ROOT.parents[2]/'benchmarks/display/workloads/campaign-workload.swift'):raise RuntimeError('Workload versions differ between Macs')
    app=application(a.product,adapter);config=load();adb=adb_at(config.get('adb'));phone=select_phone(adb,remembered=config.get('phone_identity'))
    config['adb']=str(adb);config['phone_identity']=phone_identity(adb,phone);save(config)
    from urllib.parse import urlsplit
    peer=urlsplit(client.config['url']).hostname
    receiver_displays=json.loads(subprocess.run([str(tools/'campaign-workload'),'--context'],capture_output=True,text=True,check=True).stdout)
    receiver_windows=json.loads(subprocess.run([str(tools/'campaign-workload'),'--windows',str(app['main_pid'])],capture_output=True,text=True,check=True).stdout)
    receiver={'displays':receiver_displays,'application_windows':receiver_windows,'host':host(),'application':app,'network':network(app,peer)}
    if a.product!='extend' and source['application']['executable_sha256']!=app['executable_sha256']:raise RuntimeError('Third-party test builds differ between Macs')
    if a.product=='extend' and any(source['application'].get(k)!=app.get(k) for k in ['version','build']):raise RuntimeError('Extend versions differ between Macs')
    helper_profiles=[h for h in source['application'].get('display_helpers',[]) if h['role']=='extend']
    print('Both hosts, source display, product builds, shared fixture and phone transport ready.',flush=True)
    if a.check and not a.resume:print('Preflight only; no workload or camera recording started.');return
    minimum_free=2*1024**3+5*a.repetitions*(a.seconds+6)*20*1024**2
    disk_root=(a.resume if a.resume else a.output.parent if a.output else Path(os.environ.get('DISPLAY_BENCH_RUNS',str(ROOT/'runs'))))
    while not disk_root.exists():disk_root=disk_root.parent
    if shutil.disk_usage(disk_root).free<minimum_free:raise RuntimeError(f'Insufficient free disk space; allow at least {minimum_free/1024**3:.1f} GiB for recordings and analysis')
    stamp=datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ')
    root=a.resume or a.output or Path(os.environ.get('DISPLAY_BENCH_RUNS',str(ROOT/'runs')))/f'{"setup-check" if a.smoke else "campaign"}-{a.product}-{stamp}-{uuid.uuid4().hex[:8]}'
    if not a.resume:root.mkdir(parents=True,exist_ok=False,mode=0o700)
    report={'schema_version':1,'product':a.product,'status':'running','created_utc':stamp,'context':{'source':source,'receiver':receiver,'source_display':source_display,'phone':{'model':subprocess.run([str(adb),'-s',phone,'shell','getprop','ro.product.model'],capture_output=True,text=True,check=True).stdout.strip(),'identity_sha256':config['phone_identity']}},
            'settings':{'renderer':'appkit-campaign-v1','seconds':a.seconds,'repetitions':a.repetitions,'workload_sha256':health['workload_sha256'],'mode':'extend','codec':{'status':'unavailable','reason':'No verified live codec hook supplied'},'bitrate':{'status':'unavailable','reason':'Not inferred from preferences or socket byte totals'}},'phases':[],
            'limits':['Metrics include declared observation scope.','Optical latency is screen-to-screen transition delay, not input latency; rolling-shutter row bias remains.','Optical quality includes panels and camera, not codec-only distortion.','Baseline is connected idle, not cold app-off. Recovery is a sender-process pause, not a network-link failure.']}
    if helper_profiles:
        report['settings']['active_display_helper']=helper_profiles[0]
        report['settings']['requested_fps']={'status':'observed_requested','fps':helper_profiles[0]['requested_fps']}
        report['settings']['bitrate']={'status':'observed_requested','bps':helper_profiles[0]['requested_bitrate_bps']}
    completed=set();segment=root
    if previous:
        completed=validate_resume(previous,report,root)
        print(f'Resume verified: {len(completed)} measured scenes retained.',flush=True)
        if a.check:print('Preflight only; no workload or camera recording started.');return
        segment=root/'resumptions'/f'{stamp}-{uuid.uuid4().hex[:8]}';segment.mkdir(parents=True)
        write(segment/'previous-report.json',previous)
        current_context=report['context'];report=previous;report['status']='running';report.pop('failure',None)
        report.setdefault('resumptions',[]).append({'at_utc':stamp,'context':current_context,'preserved_scenes':len(completed),'segment':str(segment.relative_to(root))})
        report['limits'].append('Campaign resumed across separate measurement sessions; fresh contexts and idle baselines retained.')
    def phase(scene,seconds):return {'id':str(uuid.uuid4()),'product':a.product,'scene':scene,'seconds':seconds,'screen':a.screen}
    try:
        print('Collecting '+('3' if a.smoke else '20')+'-second connected-idle baseline on both hosts...',flush=True)
        base=phase('baseline',3 if a.smoke else 20)
        with Sampler(app) as sampler:
            client.request('/start',base);baseline=client.wait(base['id'],base['seconds'])
        write(segment/'baseline-source.json',baseline);write(segment/'baseline-receiver.json',sampler.rows)
        baseline_summary={'definition':'Connected product, no workload; product and host process CPU, memory, sensor overhead','source':resources(baseline['rows']),'receiver':sampler.summary()}
        if previous:report['resumptions'][-1]['baseline']=baseline_summary
        else:report['baseline']=baseline_summary
        warm=phase('warmup',3 if a.smoke else 30);client.request('/start',warm);client.wait(warm['id'],warm['seconds'])
        for repetition in range(1,a.repetitions+1):
            # Reverse scene order each repetition to reduce ordering bias.
            scenes=['motion'] if a.smoke else ['static','scroll','panel','motion','recovery'];scenes=scenes if repetition%2 else list(reversed(scenes))
            for scene in scenes:
                if (repetition,scene) in completed:continue
                print(f'Repeat {repetition}/{a.repetitions}: {scene}',flush=True)
                folder=root/f'{repetition:02}-{scene}'
                if folder.exists():
                    attempts=root/'attempts';attempts.mkdir(exist_ok=True)
                    folder.rename(attempts/f'{folder.name}-{stamp}-{uuid.uuid4().hex[:8]}')
                folder.mkdir();before_network=network(app,peer)
                if not (adapter and hasattr(adapter,'focus_viewer') and adapter.focus_viewer(app,tools/'campaign-workload')):
                    viewers=[h['pid'] for h in app.get('display_helpers',[]) if h['role']=='viewer']
                    subprocess.run([str(tools/'campaign-workload'),'--activate',str(viewers[0] if len(viewers)==1 else app['main_pid'])],check=True)
                job=phase(scene,a.seconds);write(folder/'phase-request.json',job)
                argv=[ROOT/'.venv/bin/python3',ROOT/'camera_run.py','--serial',phone,'--adb',adb,'--extend-repo',ROOT.parents[2],'--product',a.product,'--phase-seconds',str(a.seconds),'--seconds',str(a.seconds+(2 if a.smoke else 6)),'--output',folder/'camera']
                if scene=='static':argv+=['--allow-static']
                argv+=['--ready-command',sys.executable,HERE/'client.py','--pairing',a.pairing,'--phase',folder/'phase-request.json','--ack',folder/'phase-ack.json']
                with Sampler(app) as sampler:
                    result=subprocess.run([str(x) for x in argv],check=True)
                    measured=client.wait(job['id'],a.seconds)
                verify_source(measured,source_display,a.screen,source['application']['executable_sha256']);write(folder/'source.json',measured);write(folder/'receiver-resources.json',sampler.rows)
                ack=json.loads((folder/'phase-ack.json').read_text())
                measured_rows=[r for r in sampler.rows if ack['request_start_ns']<=r['at_ns']<=ack['ack_ns']+a.seconds*1e9]
                sampler.summary()
                if len(measured_rows)<3:raise RuntimeError('Receiver measurement window missing')
                receiver_resources=resources(measured_rows)
                receiver_resources['window_uncertainty_ms']=(ack['ack_ns']-ack['request_start_ns'])/1e6
                camera=folder/'camera';patches=json.loads((camera/'patches.json').read_text())
                reference=source_patch(camera/'raw/registration.png',patches) if a.source_index is None else a.source_index
                o=None if scene=='static' else optical(camera,reference)
                q=quality(camera/'raw/registration.png',patches)
                current=application(a.product,adapter)
                if current['executable_sha256']!=app['executable_sha256']:raise RuntimeError('Receiver product build changed')
                record={'repetition':repetition,'scene':scene,'status':'measured','resources':{'source':resources(measured['rows']),'receiver':receiver_resources},'optical':o,'quality':q,'source_patch_index':reference,
                        'network':{'source_start':measured['network_start'],'source_end':measured['network_end'],'receiver_start':before_network,'receiver_end':network(app,peer)},
                        'fault':measured.get('fault'),'stages':{'source':measured['stages'],'receiver':snapshot(app,adapter,ack['ack_ns'],int(ack['ack_ns']+a.seconds*1e9))},'source_cadence':measured['fixture']['source_callback_hz'],'camera':json.loads((camera/'summary.json').read_text())}
                if o and (o['terminal_receiver_stall_ms'] or 0)>1000:raise RuntimeError('Receiver stopped updating before scene completed')
                if scene=='recovery' and (not measured.get('fault') or not measured['fault']['scope_pids']):raise RuntimeError('Recovery fault was not injected')
                record['network_summary']={'source':network_delta(record['network']['source_start'],record['network']['source_end']),'receiver':network_delta(before_network,record['network']['receiver_end'])}
                record['route']=classify_route(record['network_summary'])
                cfg=record['stages']['source'].get('configuration',{})
                codec=cfg.get('encoder_codec_fourcc')
                if codec is not None:report['settings']['codec']={'status':'observed','fourcc':int(codec).to_bytes(4,'big').decode('ascii'),'scope':'VideoToolbox encoder created by application'}
                if cfg.get('requested_bitrate_bps') is not None:report['settings']['bitrate']={'status':'observed_requested','bps':cfg['requested_bitrate_bps']}
                if cfg.get('requested_fps') is not None:report['settings']['requested_fps']={'status':'observed_requested','fps':cfg['requested_fps']}
                report['phases'].append(record);write(root/'report.json',report)
        if len(report['phases'])!=(1 if a.smoke else 5)*a.repetitions:raise RuntimeError('Campaign missing measured scenes')
        report['status']='setup_check_passed' if a.smoke else 'complete_with_declared_limits'
    except BaseException as error:
        report['status']='failed';report['failure']=str(error)
        try:client.request('/stop',{})
        except Exception:pass
        raise
    finally:
        write(root/'report.json',report);render(root,report)
        write(root/'manifest.json',{'schema_version':1,'kind':'optical-campaign-v1','status':report['status'],'context':report['context'],'settings':report['settings'],'artifacts':artifact_inventory(root)})
        print('Results: '+str(root),flush=True)
        if not a.no_open:open_report(root/'report.html')
    print('Setup check passed.' if a.smoke else 'Benchmark completed successfully.',flush=True)
if __name__=='__main__':
    try:main()
    except (Exception,KeyboardInterrupt) as e:print('Campaign stopped: '+str(e),file=sys.stderr);sys.exit(1)
