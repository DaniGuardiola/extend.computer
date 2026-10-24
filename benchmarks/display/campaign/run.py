"""Repeatable two-host campaign: baseline, warm-up, scenes, optical capture and report."""
import argparse,datetime,hashlib,html,json,os,re,shutil,signal,subprocess,sys,threading,time,uuid
from pathlib import Path
HERE=Path(__file__).resolve().parent
sys.path.insert(0,str(HERE));sys.path.insert(0,str(HERE.parent/'automation'))
from client import Client
from context import host,application,sample,network,digest
from stages import snapshot
from metrics import resources,optical,quality,stats,source_patch,network_delta,classify_route
from bootstrap import ROOT,build_swift
from bench import load,save,adb_at,select_phone,phone_identity,product_adapter
from progress import Progress

def recordings_finished(ui):
    ui.line('')
    if os.environ.get('EXTEND_DISPLAY_SETTINGS_SWEEP') == '1':
        ui.line('Mode recording finished. Keep both Macs and phone idle; more settings modes follow.')
    else:
        ui.line(ui.paint('Recordings finished. Both Macs and phone are free to use.','1;32'))
        ui.line('Keep phone connected until transfers finish. Processing uses this Mac in the background.')

def campaign_scenes(settings):
    if settings.get('campaign_profile')=='setup_check':return ['motion']
    if settings.get('campaign_profile')=='diagnostic':return list(settings['scene_selection'])
    return ['static','scroll','panel','motion','recovery']+(['cursor'] if settings.get('include_cursor') else [])+(['cursor-optical','cursor-sweep'] if settings.get('include_optical_cursor') else [])

def campaign_mode(args,previous=None):
    scenes=getattr(args,'scene',None);seconds=getattr(args,'seconds',None)
    if scenes:
        if getattr(args,'include_optical_cursor',False):raise ValueError('Use --scene cursor-optical and --scene cursor-sweep instead of --include-optical-cursor with a scene selection')
        if any(scene not in {'static','scroll','panel','motion','recovery','cursor','cursor-optical','cursor-sweep'} for scene in scenes):raise ValueError('Unknown diagnostic scene')
        if len(scenes)!=len(set(scenes)):raise ValueError('Duplicate diagnostic scene')
        if getattr(args,'include_cursor',False):raise ValueError('Use --scene cursor instead of --include-cursor with a scene selection')
    diagnostic=scenes is not None or seconds is not None
    if previous and diagnostic:raise ValueError('Resume inherits saved scene selection and duration; do not override --scene or --seconds')
    if diagnostic and (args.smoke or getattr(args,'check',False) or getattr(args,'process',None)):
        raise ValueError('--scene and --seconds are development capture options, incompatible with --check, --smoke or --process')
    mode='diagnostic' if diagnostic else 'setup_check' if args.smoke else 'quick' if args.quick else (previous or {}).get('settings',{}).get('campaign_profile','full')
    if previous and mode!=previous['settings'].get('campaign_profile','full'):raise ValueError('Resume campaign profile differs')
    if mode=='quick' or (mode=='diagnostic' and args.quick):
        if args.repetitions not in (None,1):raise ValueError('Quick mode uses exactly one repetition')
        args.repetitions=1
    if mode=='diagnostic':
        args.selected_scenes=list(previous['settings']['scene_selection']) if previous else list(scenes or campaign_scenes({'include_cursor':getattr(args,'include_cursor',False),'include_optical_cursor':getattr(args,'include_optical_cursor',False)}))
        if args.repetitions is None:args.repetitions=previous['settings']['repetitions'] if previous else 1
    return mode

def scene_order(settings,repetition):
    scenes=campaign_scenes(settings)
    return scenes if repetition%2 else list(reversed(scenes))

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

def verify_display_profile(display,expected):
    if expected is None:return
    match=re.fullmatch(r'([1-9]\d*)x([1-9]\d*)@([12])',expected)
    if not match:raise ValueError('Display profile must be WIDTHxHEIGHT@SCALE, e.g. 1728x1085@2')
    width,height,scale=map(int,match.groups())
    wanted={'width':width,'height':height,'scale':scale,'pixel_width':width*scale,'pixel_height':height*scale}
    actual={key:display.get(key) for key in wanted}
    if actual!=wanted:raise RuntimeError(f'Display profile mismatch: expected {wanted}, observed {actual}')

def verify_source(result,initial,screen,expected_build):
    current=profile(result['context'],screen)
    if current!=initial:raise RuntimeError('Source display geometry changed')
    if result['context']['application']['executable_sha256']!=expected_build:raise RuntimeError('Product build changed')
    # A deliberately selected 30 Hz display cannot drive a 50 Hz fixture.
    # Keep the historical 60 Hz acceptance ratio for the selected refresh rate.
    expected_hz=min(current.get('refresh_hz') or 60,60)
    if result.get('fixture') and result['fixture']['source_callback_hz']<expected_hz*5/6 and result['scene']!='static':raise RuntimeError('Source cannot sustain workload cadence')

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
        cfg=phase.get('display_configuration',{});helper=report['settings'].get('active_display_helper',{})
        width=cfg.get('width',helper.get('logical_width'));height=cfg.get('height',helper.get('logical_height'))
        profile=f'{width} × {height}'+(' (Retina)' if cfg.get('retina',helper.get('scale')==2) else '') if width and height else 'unavailable'
        fps=cfg.get('fps',helper.get('requested_fps'));bitrate=cfg.get('bitrate',helper.get('requested_bitrate_bps'))
        rows.append('<tr>'+''.join('<td>'+html.escape(str(v))+'</td>' for v in [phase['repetition'],phase['scene'],profile,fps if fps else 'unavailable',number(bitrate/1_000_000 if bitrate else None),phase.get('route',{}).get('label','Unclassified'),number(metrics.get('receiver_visible_hz')),number(lat),number(source),number(receiver),number(phase.get('quality',{}).get('global_optical_ssim')),phase['status']])+'</tr>')
    body='<html><meta charset="utf-8"><title>Display benchmark</title><style>body{font:16px system-ui;margin:40px;max-width:1100px}table{border-collapse:collapse}td,th{border:1px solid #ccc;padding:10px}pre{white-space:pre-wrap}</style>'
    status={'setup_check_passed':'Setup check passed','complete_with_declared_limits':'Benchmark completed successfully','failed':'Benchmark failed'}.get(report['status'],report['status'])
    body+='<h1>Display benchmark · '+html.escape(report['product'])+'</h1><p><strong>'+html.escape(status)+'</strong></p>'
    if report['settings'].get('campaign_profile')=='quick':body+='<p><strong>Quick development run · one repetition</strong></p>'
    if report['settings'].get('campaign_profile')=='diagnostic':body+='<p><strong>Targeted development diagnostic · not a full benchmark</strong></p>'
    if report.get('failure'):body+='<p>'+html.escape(report['failure'])+'</p>'
    body+='<table><tr><th>Repeat</th><th>Scene</th><th>Display profile</th><th>FPS target</th><th>Bitrate ceiling Mbps</th><th>Route</th><th>Detected visible updates/s</th><th>Screen-to-screen median ms</th><th>Source CPU % one core</th><th>Receiver CPU % one core</th><th>Optical strip SSIM</th><th>Status</th></tr>'+''.join(rows)+'</table>'
    body+='<p>Detected visible updates are a lower bound when camera counters are unreadable. Frame gaps can include observation gaps; native drawable callbacks are separate evidence, not proof of physical visibility.</p>'
    acquisition=[]
    for phase in report['phases']:
        profile=(phase.get('camera') or {}).get('capture_profile') or {}
        if not profile:continue
        steps=profile.get('requested_exposure_compensation_steps');ev=profile.get('exposure_compensation_step_ev')
        requested_ev=steps*ev if type(steps) is int and type(ev) in (int,float) else None
        acquisition.append('<tr>'+''.join('<td>'+html.escape(str(v))+'</td>' for v in [phase['repetition'],phase['scene'],
            profile.get('camera','unavailable'),profile.get('requested_capture_fps','unavailable'),number(requested_ev),
            profile.get('helper_apk_sha256','unavailable')])+'</tr>')
    if acquisition:
        body+='<h2>Camera acquisition</h2><p>Matched comparisons require the same camera helper build and requested capture profile. Actual automatic exposure and ISO remain in sensor.jsonl.</p><table><tr><th>Repeat</th><th>Scene</th><th>Camera ID</th><th>Capture FPS</th><th>Requested exposure EV</th><th>Installed helper SHA-256</th></tr>'+''.join(acquisition)+'</table>'
    readability=[]
    for phase in report['phases']:
        optical=phase.get('optical') or {}
        if 'transition_delay_interval_width_ms' not in optical:continue
        coverage=optical.get('counter_readable_coverage') or {};width=optical.get('transition_delay_interval_width_ms') or {}
        gaps=optical.get('unreadable_counter_intervals_ms') or {}
        percentage=lambda role:number(coverage[role]*100) if coverage.get(role) is not None else 'unavailable'
        readability.append('<tr>'+''.join('<td>'+html.escape(str(v))+'</td>' for v in [phase['repetition'],phase['scene'],percentage('source'),percentage('receiver'),number(width.get('median')),number(width.get('p95')),number((gaps.get('receiver') or {}).get('max'))])+'</tr>')
    if readability:
        body+='<h2>Camera counter uncertainty</h2><p>Each transition is bounded by the preceding readable counter and first readable new counter. These intervals exclude exposure and rolling-shutter bias. Missing readings can create uncertainty much larger than one camera sample.</p><table><tr><th>Repeat</th><th>Scene</th><th>Source readable %</th><th>Receiver readable %</th><th>Median timing interval width ms</th><th>p95 width ms</th><th>Max receiver unreadable interval ms</th></tr>'+''.join(readability)+'</table>'
    signals=[]
    for phase in report['phases']:
        signal=phase.get('signal_alignment') or {}
        if not signal:continue
        signals.append('<tr>'+''.join('<td>'+html.escape(str(v))+'</td>' for v in [phase['repetition'],phase['scene'],
            signal.get('status','unavailable'),number(signal.get('estimated_constant_delay_ms')),
            number(signal.get('correlation')),signal.get('near_peak_delay_range_ms','unavailable')])+'</tr>')
    if signals:
        body+='<h2>Signal alignment diagnostic</h2><p>Observed grayscale cell signals are matched under a constant-delay model. Candidate peaks in ambiguous results are not accepted latency measurements. This model does not repair counter IDs, measure frame rate or replace counter delay bounds. Near-peak ranges are score thresholds, not confidence intervals; exposure and scanout bias remain.</p><table><tr><th>Repeat</th><th>Scene</th><th>Status</th><th>Candidate constant delay ms</th><th>Correlation</th><th>Near-peak score range ms</th></tr>'+''.join(signals)+'</table>'
    cursor_rows=[]
    for phase in report['phases']:
        observed=phase.get('physical_cursor')
        if not observed:continue
        timing=observed['state_reference_to_cursor_ms']
        cursor_rows.append('<tr>'+''.join('<td>'+html.escape(str(v))+'</td>' for v in
            [phase['repetition'],number(timing['median']),number(timing['p95']),
                number(timing['max']),observed['matched_transition_coverage']])+'</tr>')
    if cursor_rows:
        body+='<h2>Physical cursor transitions</h2><p>Software state reference to visible pointer. Hardware mouse input latency is not measured.</p><table><tr><th>Repeat</th><th>Median ms</th><th>p95 ms</th><th>Max ms</th><th>Matched coverage</th></tr>'+''.join(cursor_rows)+'</table>'
    motion_rows=[]
    for phase in report['phases']:
        motion=phase.get('physical_cursor_motion')
        if not motion:continue
        gap=motion['receiver']['inter_resolvable_change_ms'] or {}
        motion_rows.append('<tr>'+''.join('<td>'+html.escape(str(v))+'</td>' for v in
            [phase['repetition'],number(gap.get('p95')),number(motion['longest_observed_receiver_hold_ms']),
             len(motion['receiver']['observed_holds']),number((motion['receiver'].get('change_gaps_containing_unreadable_frames_ms') or {}).get('max')),number(motion['joint_readable_coverage']*100)])+'</tr>')
    if motion_rows:
        body+='<h2>Visible continuous pointer motion</h2><p>Camera-observed movement above the spatial uncertainty threshold. Change intervals are not FPS; holds require a moving source reference. Zero confirmed holds is a lower bound: unreadable intervals can hide stalls.</p><table><tr><th>Repeat</th><th>p95 resolvable change gap ms</th><th>Longest hold ms</th><th>Holds ≥100 ms</th><th>Max unresolved gap ms</th><th>Readable joint %</th></tr>'+''.join(motion_rows)+'</table>'
    body+='<h2>Per-scene resource, network and stage evidence</h2>'
    for phase in report['phases']:
        body+='<details><summary>'+html.escape(str(phase['repetition'])+' '+phase['scene'])+'</summary><pre>'+html.escape(json.dumps(phase,indent=2))+'</pre></details>'
    body+='<h2>Context, baseline and evidence limits</h2><pre>'+html.escape(json.dumps({k:v for k,v in report.items() if k!='phases'},indent=2))+'</pre></html>'
    (root/'report.html').write_text(body)

def validate_resume(previous,current,root):
    if previous['product']!=current['product']:raise RuntimeError('Resume product differs')
    if previous['status'] not in {'failed','running'}:raise RuntimeError('Only interrupted or failed campaigns can resume')
    for key in ['renderer','seconds','repetitions','mode','workload_sha256','required_display_profile','capture_schedule']:
        if previous['settings'].get(key)!=current['settings'].get(key):raise RuntimeError('Resume setting differs: '+key)
    if bool(previous['settings'].get('include_cursor'))!=bool(current['settings'].get('include_cursor')):raise RuntimeError('Resume optional cursor phase differs')
    if bool(previous['settings'].get('include_optical_cursor'))!=bool(current['settings'].get('include_optical_cursor')):raise RuntimeError('Resume optical cursor phases differ')
    if previous['settings'].get('camera_focus_regions')!=current['settings'].get('camera_focus_regions'):raise RuntimeError('Resume camera focus regions differ')
    if previous['settings'].get('camera_focus_distance_range')!=current['settings'].get('camera_focus_distance_range'):raise RuntimeError('Resume camera focus distance range differs')
    if previous['settings'].get('camera_zoom',1.0)!=current['settings'].get('camera_zoom',1.0):raise RuntimeError('Resume camera zoom differs')
    if previous['settings'].get('camera_exposure_compensation_steps',0)!=current['settings'].get('camera_exposure_compensation_steps',0):raise RuntimeError('Resume camera exposure compensation differs')
    if previous['settings'].get('cursor_input_method','warp')!=current['settings'].get('cursor_input_method','warp'):raise RuntimeError('Resume cursor input method differs')
    if previous['settings'].get('campaign_profile','full')!=current['settings'].get('campaign_profile','full'):raise RuntimeError('Resume campaign profile differs')
    if previous['settings'].get('scene_selection')!=current['settings'].get('scene_selection'):raise RuntimeError('Resume scene selection differs')
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
        if phase['status'] not in {'measured','captured'} or key in completed:raise RuntimeError('Invalid saved scene checkpoint')
        if key[0] not in range(1,previous['settings']['repetitions']+1) or key[1] not in campaign_scenes(previous['settings']):raise RuntimeError('Unknown saved scene checkpoint')
        prefix=f'{key[0]:02}-{key[1]}/'
        artifacts={name:value for name,value in manifest.items() if name.startswith(prefix)}
        required=['source.json','camera/raw/video.mp4'] if phase['status']=='measured' else ['source.json','camera/capture.json','camera/phone-session.json']
        if any(prefix+name not in artifacts for name in required):raise RuntimeError('Saved scene evidence missing')
        for name,value in artifacts.items():
            path=root/name
            if not path.is_file() or digest(path)!=value:raise RuntimeError('Saved scene integrity check failed: '+name)
        completed.add(key)
    return completed

def process_capture(root,report,row,adb,phone,ui):
    folder=root/f"{row['repetition']:02}-{row['scene']}";camera=folder/'camera'
    collected=(camera/'summary.json').exists() and (camera/'raw/video.mp4').exists()
    if collected:
        summary=json.loads((camera/'summary.json').read_text())
        if summary['video_sha256']!=digest(camera/'raw/video.mp4'):raise RuntimeError('Collected video integrity check failed')
    argv=[ROOT/'.venv/bin/python3',ROOT/'camera_run.py','--extend-repo',ROOT.parents[2],'--product',report['product'],
          '--phase-seconds',str(report['settings']['seconds']),'--seconds',str(report['settings']['seconds']+(2 if report['settings'].get('campaign_profile')=='setup_check' else 6)),
          '--output',camera,'--analyze-existing' if collected else '--collect-existing']
    if not collected:
        if phone is None:raise RuntimeError('Phone needed to collect pending recordings')
        argv+=['--serial',phone,'--adb',adb]
    if row['scene']=='static':argv+=['--allow-static']
    with (folder/'camera.log').open('a') as log:
        try:subprocess.run([str(x) for x in argv],check=True,env={**os.environ,'DISPLAY_BENCH_PROGRESS':'1'},stderr=log)
        except subprocess.CalledProcessError as error:raise RuntimeError('Processing failed; diagnostics: '+str(folder/'camera.log')) from error
    patches=json.loads((camera/'patches.json').read_text())
    reference=report['settings'].get('source_patch_index_override')
    if reference is None:reference=source_patch(camera/'raw/registration.png',patches)
    o=None if row['scene']=='static' else optical(camera,reference)
    if o:
        try:
            from optical_signal import analyze as signal_alignment
            row['signal_alignment']=signal_alignment(camera,reference)
        except (ValueError,OSError,ImportError) as error:
            row['signal_alignment']={'status':'unavailable','reason':str(error)}
    if row['scene']=='cursor-optical':
        from optical_cursor import optical_cursor
        row['physical_cursor']=optical_cursor(camera,reference)
    if row['scene']=='cursor-sweep':
        from optical_motion import optical_motion
        row['physical_cursor_motion']=optical_motion(camera,reference)
    if o and (o['terminal_receiver_stall_ms'] or 0)>1000:raise RuntimeError('Receiver stopped updating before scene completed')
    provenance=json.loads((camera/'provenance.json').read_text())
    frame_timing=json.loads((camera/'raw/registration.png.timing.json').read_text())
    if frame_timing.get('selection')!='zero-tolerance-v1':raise RuntimeError('Registration frame selection not verified')
    row['registration_frame_timing']=frame_timing
    row['analysis_pipeline']={'frame_selection':frame_timing['selection'],
        'camera_sources':provenance['tool_source_sha256'],
        'native_sources':{name:value['source_sha256'] for name,value in provenance['native_tools'].items()},
        'metric_sources':{name:digest(Path(__file__).parent/name) for name in ['metrics.py','optical_cursor.py','optical_motion.py','optical_signal.py']}}
    row.update(optical=o,quality=quality(camera/'raw/registration.png',patches),source_patch_index=reference,
               camera=json.loads((camera/'summary.json').read_text()),status='measured')
    ui.line('   '+ui.paint('✓','32')+'  scene analyzed')

def processing_failure(folder,error,started):
    details={}
    for name in ['failure.json','optical-failure.json']:
        path=folder/'camera'/name
        if path.exists() and path.stat().st_mtime_ns>=started:
            try:details[name]=json.loads(path.read_text())
            except (OSError,json.JSONDecodeError):pass
    capture=details.get('failure.json',{});optical_failure=details.get('optical-failure.json',{})
    detail=capture if optical_failure.get('stage')=='collection' and capture else optical_failure or capture
    retryable=detail.get('retryable',isinstance(error,(subprocess.SubprocessError,OSError)) or str(error).startswith('Phone needed'))
    if detail.get('error') in {'Non-increasing sensor timestamps','Capture cadence outside tolerance','Insufficient sensor samples'}:retryable=False
    return {'error':detail.get('error',str(error)),'error_type':detail.get('error_type',type(error).__name__),
        'retryable':retryable,'diagnostic':str((folder/'camera.log').name),'camera_failure':details,
        'at_utc':datetime.datetime.now(datetime.timezone.utc).isoformat()}

def process_captures(root,report,adb,phone,ui):
    """Keep independent scenes even when one capture fails strict analysis."""
    captured=[row for row in report['phases'] if row['status']=='captured']
    failures=[row for row in captured if row.get('processing_failure',{}).get('retryable') is False]
    pending=[row for row in captured if row not in failures]
    for index,row in enumerate(pending,1):
        folder=root/f"{row['repetition']:02}-{row['scene']}"
        ui.scene(index,len(pending),row['scene']+' · processing',row['repetition'],report['settings']['repetitions'])
        started=time.time_ns()
        try:
            process_capture(root,report,row,adb,phone,ui)
            row.pop('processing_failure',None)
        except Exception as error:
            row['processing_failure']=processing_failure(folder,error,started)
            report.setdefault('processing_failures',[]).append({'repetition':row['repetition'],'scene':row['scene'],**row['processing_failure']})
            failures.append(row)
            ui.line('   '+ui.paint('!','33')+'  scene retained: '+row['processing_failure']['error'])
        finally:
            write(root/'report.json',report)
            write(root/'manifest.json',{'artifacts':artifact_inventory(root)})
    if failures:
        names=', '.join(f"{row['repetition']}:{row['scene']}" for row in failures)
        raise RuntimeError('Processing failed for '+names+'; other scenes retained. See processing_failures in report.json and scene camera.log; use --resume --retry-scene for invalid captures.')

def retry_saved_scenes(root,report,completed,specifications):
    """Archive failed capture evidence before explicitly scheduling recapture."""
    selected=[]
    for specification in specifications:
        parts=specification.split(':',1)
        repetition=int(parts[0]) if len(parts)==2 and parts[0].isdigit() else None
        scene=parts[-1]
        if len(parts)==2 and repetition is None:raise ValueError('Retry scene must be SCENE or REPETITION:SCENE')
        candidates=[row for row in report['phases'] if row['scene']==scene and (repetition is None or row['repetition']==repetition)
            and row['status']=='captured' and row.get('processing_failure')]
        if not candidates:raise ValueError('No failed captured scene matches '+specification)
        for row in candidates:
            if row not in selected:selected.append(row)
    for row in selected:
        folder=root/f"{row['repetition']:02}-{row['scene']}"
        attempts=root/'attempts';attempts.mkdir(exist_ok=True)
        archived=attempts/(folder.name+'-retry-'+uuid.uuid4().hex[:8])
        folder.rename(archived);write(archived/'checkpoint.json',row)
        report['phases'].remove(row);completed.discard((row['repetition'],row['scene']))
        report.setdefault('retried_scenes',[]).append({'repetition':row['repetition'],'scene':row['scene'],'evidence':str(archived.relative_to(root))})
    write(root/'report.json',report);write(root/'manifest.json',{'artifacts':artifact_inventory(root)})


def phone_free_bytes(adb,phone):
    text=subprocess.run([str(adb),'-s',phone,'shell','df','-k','/data'],capture_output=True,text=True,check=True).stdout
    rows=[line.split() for line in text.splitlines() if len(line.split())>=6]
    if not rows or not rows[-1][3].isdigit():raise RuntimeError('Could not verify free phone storage')
    return int(rows[-1][3])*1024

def schedule_reanalysis(root,report):
    failed=[row for row in report['phases'] if row['status']=='captured' and row.get('processing_failure')]
    for row in failed:
        camera=root/f"{row['repetition']:02}-{row['scene']}"/'camera'
        if not (camera/'summary.json').is_file() or not (camera/'raw/video.mp4').is_file():
            raise RuntimeError('Reanalysis requires collected footage; use normal processing or recapture')
    if not failed:raise RuntimeError('No failed collected scenes to reanalyze')
    archive=root/'analysis-attempts'/uuid.uuid4().hex
    archive.mkdir(parents=True)
    write(archive/'checkpoint.json',report)
    for row in failed:
        camera=root/f"{row['repetition']:02}-{row['scene']}"/'camera'
        target=archive/f"{row['repetition']:02}-{row['scene']}";target.mkdir()
        for name in ['provenance.json','optical-failure.json','optical-window.json','patches.json','tracking.json']:
            if (camera/name).exists():shutil.copy2(camera/name,target/name)
        row.pop('processing_failure')
    report.setdefault('reanalysis_attempts',[]).append({'previous_analysis':str(archive.relative_to(root)),
        'scenes':[f"{row['repetition']}:{row['scene']}" for row in failed],
        'scope':'Reprocess identical collected footage with current analyzer; validity gates remain enforced'})

def prepare_reanalysis_copy(path,output):
    """Reanalyze accepted footage without changing the original campaign."""
    source=path.expanduser().resolve();target=output.expanduser().resolve()
    if target==source or source in target.parents or target in source.parents:
        raise ValueError('Reanalysis output must be separate from the source campaign')
    if target.exists():raise ValueError('Reanalysis output already exists')
    report=json.loads((source/'report.json').read_text())
    if report.get('status') not in {'complete_with_declared_limits','setup_check_passed'}:
        raise ValueError('Separate reanalysis requires a completed campaign')
    if not report.get('phases') or any(row.get('status')!='measured' for row in report['phases']):
        raise ValueError('Separate reanalysis requires every scene to be measured')
    inventory=json.loads((source/'manifest.json').read_text())['artifacts']
    if inventory.get('report.json')!=digest(source/'report.json'):
        raise RuntimeError('Original report integrity check failed')
    for name,expected in inventory.items():
        artifact=(source/name).resolve()
        if source not in artifact.parents or not artifact.is_file() or digest(artifact)!=expected:
            raise RuntimeError('Original artifact integrity check failed: '+name)
    videos=[]
    for row in report['phases']:
        if row['scene'] not in campaign_scenes(report['settings']) or type(row['repetition']) is not int:
            raise ValueError('Invalid saved scene')
        camera=source/f"{row['repetition']:02}-{row['scene']}"/'camera'
        prefix=str(camera.parent.relative_to(source))+'/'
        if any(prefix+name not in inventory for name in ['source.json','camera/raw/video.mp4','camera/summary.json']):
            raise RuntimeError('Original scene evidence missing')
        summary=json.loads((camera/'summary.json').read_text())
        video=camera/'raw/video.mp4'
        if digest(video)!=summary['video_sha256']:raise RuntimeError('Collected video integrity check failed')
        videos.append({'path':str(video.relative_to(source)),'sha256':summary['video_sha256']})
    target.parent.mkdir(parents=True,exist_ok=True)
    if sys.platform=='darwin':
        # APFS clones save disk space; never hard-link mutable analysis files.
        subprocess.run(['/bin/cp','-cR',str(source),str(target)],check=True)
    else:shutil.copytree(source,target)
    archive=target/'analysis-attempts'/('accepted-'+uuid.uuid4().hex);archive.mkdir(parents=True)
    write(archive/'checkpoint.json',report)
    for row in report['phases']:
        camera=target/f"{row['repetition']:02}-{row['scene']}"/'camera'
        saved=archive/f"{row['repetition']:02}-{row['scene']}";saved.mkdir()
        for name in ['provenance.json','optical-failure.json','optical-window.json','patches.json','tracking.json']:
            if (camera/name).exists():shutil.copy2(camera/name,saved/name)
        row['status']='captured'
        for key in ['optical','quality','signal_alignment','physical_cursor','physical_cursor_motion',
                    'registration_frame_timing','analysis_pipeline','source_patch_index','processing_failure']:
            row.pop(key,None)
    report['status']='running'
    report['offline_reanalysis']={'original_campaign':str(source),'original_report_sha256':digest(source/'report.json'),
        'previous_analysis':str(archive.relative_to(target)),'videos':videos,
        'scope':'Identical retained footage; new analysis only, no new recording or measurement session'}
    report['limits'].append('Offline reanalysis of retained footage; original campaign remains unchanged.')
    write(target/'report.json',report)
    write(target/'manifest.json',{'artifacts':artifact_inventory(target)})
    return target

def process_existing(path,no_open=False,reanalyze=False,reanalysis_output=None):
    if reanalysis_output:path=prepare_reanalysis_copy(path,reanalysis_output)
    root=path.expanduser().resolve();report=json.loads((root/'report.json').read_text())
    validate_resume(report,report,root)
    if reanalyze:schedule_reanalysis(root,report)
    ui=Progress();ui.heading(report['product'],'processing only')
    pending=[r for r in report['phases'] if r['status']=='captured' and r.get('processing_failure',{}).get('retryable') is not False]
    adb=phone=None
    if any(not (root/f"{r['repetition']:02}-{r['scene']}"/'camera/summary.json').exists() for r in pending):
        config=load();adb=adb_at(config.get('adb'))
        phone=select_phone(adb,remembered=report['context']['phone']['identity_sha256'])
    report['status']='running';report.pop('failure',None)
    try:
        process_captures(root,report,adb,phone,ui)
        total=len(campaign_scenes(report['settings']))*report['settings']['repetitions']
        if len(report['phases'])!=total:raise RuntimeError('Processed saved captures; missing scenes still need --resume')
        report['status']='setup_check_passed' if report['settings'].get('campaign_profile')=='setup_check' else 'complete_with_declared_limits'
        print(ui.paint('Benchmark processing completed successfully.','32'),flush=True)
    except BaseException as error:
        report['status']='failed';report['failure']=str(error);raise
    finally:
        write(root/'report.json',report);render(root,report)
        write(root/'manifest.json',{'schema_version':1,'kind':'optical-campaign-v1','status':report['status'],'context':report['context'],'settings':report['settings'],'artifacts':artifact_inventory(root)})
        print('Results: '+str(root),flush=True)
        if not no_open:open_report(root/'report.html')

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--product',type=str);p.add_argument('--pairing',type=Path,default=ROOT/'.cache/campaign/source-pairing.json')
    p.add_argument('--reanalyze',action='store_true',help='With --process, archive failed analysis and retry identical collected footage using the current analyzer')
    p.add_argument('--reanalysis-output',type=Path,help='With --process, reanalyze a completed campaign into a new folder, preserving the original')
    p.add_argument('--screen',default='auto');p.add_argument('--seconds',type=int,help='Development diagnostic duration (5..54 seconds); never a full benchmark profile')
    p.add_argument('--scene',action='append',choices=['static','scroll','panel','motion','recovery','cursor','cursor-optical','cursor-sweep'],help='Development diagnostic scene; repeat for a small selected suite')
    p.add_argument('--repetitions',type=int);p.add_argument('--output',type=Path)
    p.add_argument('--display-profile',help='Require logical and backing geometry, e.g. 1728x1085@2')
    p.add_argument('--include-optical-cursor',action='store_true',default=None,help='Add physical cursor jump-latency and continuous-motion phases')
    p.add_argument('--include-cursor',action='store_true',default=None,help='Add an optional bounded 120 Hz source-pointer motion phase')
    p.add_argument('--focus-distance-range',type=Path,help='Calibrated autofocus distance interval JSON; retained unchanged on resume')
    p.add_argument('--camera-zoom',type=float,help='Sensor crop ratio, 1..2; recorded in acquisition provenance')
    p.add_argument('--camera-exposure-compensation-steps',type=int,help='Explicit AE compensation in camera-supported steps; default zero, retained unchanged on resume')
    p.add_argument('--focus-regions',type=Path,help='Validated camera focus-region JSON; retained unchanged on resume')
    p.add_argument('--cursor-input-method',choices=['warp','mouse-events'],help='Warp-only or OS-posted mouse motion for cursor phases; retained in comparison context')
    p.add_argument('--process',type=Path,help='Finish transfer/analysis of an existing run without source access or new recordings')
    p.add_argument('--resume',type=Path,help='Continue a failed campaign, preserving measured scenes and old attempts')
    p.add_argument('--retry-scene',action='append',default=[],help='With --resume, archive and recapture a failed SCENE or REPETITION:SCENE')
    p.add_argument('--no-open',action='store_true',help='Do not open the HTML report automatically')
    modes=p.add_mutually_exclusive_group()
    modes.add_argument('--smoke',action='store_true',help='Five-second single-scene setup check; never a comparison result')
    modes.add_argument('--quick',action='store_true',help='Development run: selected campaign phases once, with normal durations and camera settings')
    p.add_argument('--check',action='store_true');p.add_argument('--source-index',type=int,choices=[0,1],default=None)
    a=p.parse_args()
    if a.process:
        if a.resume or a.output or a.quick or a.smoke or a.check or a.include_cursor or a.include_optical_cursor or a.cursor_input_method or a.focus_regions or a.focus_distance_range or a.camera_zoom is not None or a.camera_exposure_compensation_steps is not None or a.retry_scene or a.scene or a.seconds is not None or a.repetitions is not None:p.error('--process cannot be combined with capture mode options')
        if a.reanalyze and a.reanalysis_output:p.error('Choose failed-scene reanalysis or separate completed-campaign reanalysis')
        process_existing(a.process,a.no_open,a.reanalyze,a.reanalysis_output);return
    if a.reanalysis_output:p.error('--reanalysis-output requires --process')
    if a.reanalyze:p.error('--reanalyze requires --process')
    if not a.product:p.error('--product is required')
    if a.retry_scene and not a.resume:p.error('--retry-scene requires --resume')
    previous=None
    if a.resume:
        if a.output or a.smoke:p.error('--resume cannot be combined with --output or --smoke')
        a.resume=a.resume.expanduser().resolve();previous=json.loads((a.resume/'report.json').read_text())
        if a.display_profile is None:a.display_profile=previous['settings'].get('required_display_profile')
        if a.source_index is None:a.source_index=previous['settings'].get('source_patch_index_override')
        if a.include_cursor is None:a.include_cursor=bool(previous['settings'].get('include_cursor'))
        if a.include_optical_cursor is None:a.include_optical_cursor=bool(previous['settings'].get('include_optical_cursor'))
        if a.cursor_input_method is None:a.cursor_input_method=previous['settings'].get('cursor_input_method','warp')
    from capture_phone import validate_focus_regions,validate_focus_distance_range,camera_zoom
    zoom=camera_zoom(a.camera_zoom if a.camera_zoom is not None else previous['settings'].get('camera_zoom',1.0) if previous else 1.0)
    exposure_steps=a.camera_exposure_compensation_steps if a.camera_exposure_compensation_steps is not None else previous['settings'].get('camera_exposure_compensation_steps',0) if previous else 0
    focus_regions=validate_focus_regions(json.loads(a.focus_regions.read_text())) if a.focus_regions else previous['settings'].get('camera_focus_regions') if previous else None
    focus_distance_range=validate_focus_distance_range(json.loads(a.focus_distance_range.read_text())) if a.focus_distance_range else previous['settings'].get('camera_focus_distance_range') if previous else None
    if a.cursor_input_method is None:a.cursor_input_method='warp'
    if a.smoke and (a.include_cursor or a.include_optical_cursor):p.error('Optional cursor phases cannot be combined with --smoke')
    try:mode=campaign_mode(a,previous)
    except ValueError as error:p.error(str(error))
    ui=Progress();ui.heading(a.product,mode.replace('_',' '));started=time.monotonic()
    if a.seconds is None:a.seconds=previous['settings']['seconds'] if previous else 10 if mode=='diagnostic' else 30
    if a.repetitions is None:a.repetitions=previous['settings']['repetitions'] if previous else 3
    if a.smoke:a.seconds=5;a.repetitions=1
    if not 5<=a.seconds<=54 or not 1<=a.repetitions<=10:p.error('seconds must be 5..54; repetitions 1..10')
    with ui.step('preflight'):
        adapter=product_adapter()
        if a.product!='extend' and not (adapter and a.product in adapter.products()):raise RuntimeError('Product adapter unavailable')
        client=Client(a.pairing)
        health=client.request('/health');source=client.request('/context/'+a.product)
        tools=build_swift(ROOT.parents[2]);source_display=profile(source,a.screen)
        verify_display_profile(source_display,a.display_profile)
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
    planned_settings={'campaign_profile':mode,'scene_selection':getattr(a,'selected_scenes',None),'include_cursor':bool(a.include_cursor),'include_optical_cursor':bool(a.include_optical_cursor)}
    minimum_free=2*1024**3+len(campaign_scenes(planned_settings))*a.repetitions*(a.seconds+6)*20*1024**2
    if phone_free_bytes(adb,phone)<minimum_free:raise RuntimeError(f'Insufficient free phone storage; allow {minimum_free/1024**3:.1f} GiB for batched captures')
    disk_root=(a.resume if a.resume else a.output.parent if a.output else Path(os.environ.get('DISPLAY_BENCH_RUNS',str(ROOT/'runs'))))
    while not disk_root.exists():disk_root=disk_root.parent
    if shutil.disk_usage(disk_root).free<minimum_free:raise RuntimeError(f'Insufficient free disk space; allow at least {minimum_free/1024**3:.1f} GiB for recordings and analysis')
    if a.check and not a.resume:print('Preflight only; no workload or camera recording started.');return
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
    report['settings']['required_display_profile']=a.display_profile
    report['settings']['campaign_profile']=mode
    if mode=='diagnostic':
        report['settings']['scene_selection']=a.selected_scenes
        report['settings']['measurement_scope']='development-only'
        report['limits'].append('Targeted development diagnostic: selected scenes and/or custom duration; not comparable to full or quick campaigns.')
    report['settings']['include_cursor']=bool(a.include_cursor)
    report['settings']['include_optical_cursor']=bool(a.include_optical_cursor)
    report['settings']['cursor_input_method']=a.cursor_input_method
    report['settings']['camera_focus_regions']=focus_regions
    report['settings']['camera_focus_distance_range']=focus_distance_range
    report['settings']['camera_zoom']=zoom
    report['settings']['camera_exposure_compensation_steps']=exposure_steps
    distance_path=root/'camera-focus-distance-range.json'
    if focus_distance_range is not None:write(distance_path,focus_distance_range)
    focus_path=root/'camera-focus-regions.json'
    if focus_regions is not None:write(focus_path,focus_regions)
    if mode=='diagnostic' and 'cursor' in a.selected_scenes:report['settings']['include_cursor']=True
    if report['settings']['include_cursor']:report['limits'].append('Optional cursor phase: synthetic source pointer movement and host-local cursor hooks only; camera counters measure video, not physical cursor latency. Shape and visibility transitions are not exercised.')
    report['settings']['capture_schedule']='capture-first-v1'
    report['settings']['source_patch_index_override']=a.source_index
    report['settings']['baseline_seconds']=3 if a.smoke else 5 if mode=='diagnostic' else 20
    report['settings']['warmup_seconds']=3 if a.smoke else 5 if mode=='diagnostic' else 30
    report['limits'].append('Captures run back-to-back; transfer and optical analysis happen afterward. Inter-scene idle gaps differ from older interleaved campaigns.')
    if mode=='quick':report['limits'].append('Development quick run: one repetition; not a full repeated campaign.')
    completed=set();segment=root
    if previous:
        completed=validate_resume(previous,report,root)
        print(f'Resume verified: {len(completed)} measured scenes retained.',flush=True)
        if a.check:print('Preflight only; no workload or camera recording started.');return
        segment=root/'resumptions'/f'{stamp}-{uuid.uuid4().hex[:8]}';segment.mkdir(parents=True)
        write(segment/'previous-report.json',previous)
        current_context=report['context'];report=previous;report['status']='running';report.pop('failure',None)
        report.setdefault('resumptions',[]).append({'at_utc':stamp,'context':current_context,'preserved_scenes':len(completed),'segment':str(segment.relative_to(root))})
        if a.retry_scene:retry_saved_scenes(root,report,completed,a.retry_scene)
        report['limits'].append('Campaign resumed across separate measurement sessions; fresh contexts and idle baselines retained.')
    def phase(scene,seconds):
        result={'id':str(uuid.uuid4()),'product':a.product,'scene':scene,'seconds':seconds,'screen':a.screen}
        if scene in {'cursor','cursor-optical','cursor-sweep'}:result['input_method']=a.cursor_input_method
        return result
    try:
        base=phase('baseline',report['settings'].get('baseline_seconds',3 if a.smoke else 5 if mode=='diagnostic' else 20))
        with ui.step('connected-idle baseline'),Sampler(app) as sampler:
            client.request('/start',base);baseline=client.wait(base['id'],base['seconds'])
        write(segment/'baseline-source.json',baseline);write(segment/'baseline-receiver.json',sampler.rows)
        baseline_summary={'definition':'Connected product, no workload; product and host process CPU, memory, sensor overhead','source':resources(baseline['rows']),'receiver':sampler.summary()}
        if previous:report['resumptions'][-1]['baseline']=baseline_summary
        else:report['baseline']=baseline_summary
        warm=phase('warmup',report['settings'].get('warmup_seconds',3 if a.smoke else 5 if mode=='diagnostic' else 30))
        with ui.step('warm-up'):
            client.request('/start',warm);client.wait(warm['id'],warm['seconds'])
        for repetition in range(1,a.repetitions+1):
            # Reverse scene order each repetition to reduce ordering bias.
            scenes=scene_order(report['settings'],repetition)
            for scene in scenes:
                if (repetition,scene) in completed:continue
                ui.scene(len(report['phases'])+1,len(scenes)*a.repetitions,scene,repetition,a.repetitions)
                folder=root/f'{repetition:02}-{scene}'
                if folder.exists():
                    attempts=root/'attempts';attempts.mkdir(exist_ok=True)
                    folder.rename(attempts/f'{folder.name}-{stamp}-{uuid.uuid4().hex[:8]}')
                folder.mkdir();before_network=network(app,peer)
                if not (adapter and hasattr(adapter,'focus_viewer') and adapter.focus_viewer(app,tools/'campaign-workload')):
                    viewers=[h['pid'] for h in app.get('display_helpers',[]) if h['role']=='viewer']
                    subprocess.run([str(tools/'campaign-workload'),'--activate',str(viewers[0] if len(viewers)==1 else app['main_pid'])],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
                job=phase(scene,a.seconds);write(folder/'phase-request.json',job)
                argv=[ROOT/'.venv/bin/python3',ROOT/'camera_run.py','--serial',phone,'--adb',adb,'--extend-repo',ROOT.parents[2],'--product',a.product,'--phase-seconds',str(a.seconds),'--seconds',str(a.seconds+(2 if a.smoke else 6)),'--output',folder/'camera','--capture-only']
                focus_job=None
                if focus_regions is not None:argv+=['--focus-regions',str(focus_path)]
                if focus_distance_range is not None:argv+=['--focus-distance-range',str(distance_path)]
                argv+=['--zoom',str(zoom)]
                argv+=['--exposure-compensation-steps',str(exposure_steps)]
                if focus_regions is not None or focus_distance_range is not None or zoom>1 or exposure_steps!=0:
                    focus_job={'id':str(uuid.uuid4()),'product':a.product,'scene':'warmup','seconds':60,'screen':a.screen}
                    write(folder/'focus-preparation-request.json',focus_job)
                    client.request('/start',focus_job)
                if scene=='static':argv+=['--allow-static']
                argv+=['--ready-command',sys.executable,HERE/'client.py','--pairing',a.pairing,'--phase',folder/'phase-request.json','--ack',folder/'phase-ack.json']
                if focus_job is not None:argv+=['--replace-focus-phase-id',focus_job['id']]
                with Sampler(app) as sampler:
                    with (folder/'camera.log').open('w') as log:
                        try:result=subprocess.run([str(x) for x in argv],check=True,env={**os.environ,'DISPLAY_BENCH_PROGRESS':'1'},stderr=log)
                        except subprocess.CalledProcessError as error:raise RuntimeError('Camera step failed; diagnostics: '+str(folder/'camera.log')) from error
                    measured=client.wait(job['id'],a.seconds)
                verify_source(measured,source_display,a.screen,source['application']['executable_sha256']);write(folder/'source.json',measured);write(folder/'receiver-resources.json',sampler.rows)
                ack=json.loads((folder/'phase-ack.json').read_text())
                measured_rows=[r for r in sampler.rows if ack['request_start_ns']<=r['at_ns']<=ack['ack_ns']+a.seconds*1e9]
                sampler.summary()
                if len(measured_rows)<3:raise RuntimeError('Receiver measurement window missing')
                receiver_resources=resources(measured_rows)
                receiver_resources['window_uncertainty_ms']=(ack['ack_ns']-ack['request_start_ns'])/1e6
                current=application(a.product,adapter)
                if current['executable_sha256']!=app['executable_sha256']:raise RuntimeError('Receiver product build changed')
                record={'repetition':repetition,'scene':scene,'status':'captured','resources':{'source':resources(measured['rows']),'receiver':receiver_resources},
                        'network':{'source_start':measured['network_start'],'source_end':measured['network_end'],'receiver_start':before_network,'receiver_end':network(app,peer)},
                        'fault':measured.get('fault'),'stages':{'source':measured['stages'],'receiver':snapshot(app,adapter,ack['ack_ns'],int(ack['ack_ns']+a.seconds*1e9))},'source_cadence':measured['fixture']['source_callback_hz']}
                if scene=='recovery' and (not measured.get('fault') or not measured['fault']['scope_pids']):raise RuntimeError('Recovery fault was not injected')
                if scene in {'cursor','cursor-optical','cursor-sweep'}:
                    cursor=measured['fixture'].get('cursor',{})
                    if cursor.get('errors')!=0 or not cursor.get('position_restored'):raise RuntimeError('Cursor fixture failed to move or restore the source pointer')
                    if cursor.get('input_method','warp')!=a.cursor_input_method:raise RuntimeError('Cursor fixture input method differs from request')
                    record['cursor_fixture']=cursor
                record['network_summary']={'source':network_delta(record['network']['source_start'],record['network']['source_end']),'receiver':network_delta(before_network,record['network']['receiver_end'])}
                record['route']=classify_route(record['network_summary'])
                cfg=record['stages']['source'].get('configuration',{})
                codec=cfg.get('encoder_codec_fourcc')
                if codec is not None:report['settings']['codec']={'status':'observed','fourcc':int(codec).to_bytes(4,'big').decode('ascii'),'scope':'VideoToolbox encoder created by application'}
                if cfg.get('bitrate',cfg.get('requested_bitrate_bps')) is not None:report['settings']['bitrate']={'status':'observed_requested','bps':cfg.get('bitrate',cfg.get('requested_bitrate_bps'))}
                if cfg.get('fps',cfg.get('requested_fps')) is not None:report['settings']['requested_fps']={'status':'observed_requested','fps':cfg.get('fps',cfg.get('requested_fps'))}
                if cfg.get('width') is not None:record['display_configuration']=cfg
                report['phases'].append(record);write(root/'report.json',report)
                write(root/'manifest.json',{'artifacts':artifact_inventory(root)})
                ui.line('   '+ui.paint('✓','32')+'  capture saved')
        recordings_finished(ui)
        report['capture_finished_utc']=datetime.datetime.now(datetime.timezone.utc).isoformat()
        process_captures(root,report,adb,phone,ui)
        if len(report['phases'])!=len(campaign_scenes(report['settings']))*a.repetitions:raise RuntimeError('Campaign missing measured scenes')
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
    ui.line(ui.paint(f'Finished in {(time.monotonic()-started)/60:.1f} min','90'))
    print(ui.paint('Setup check passed.' if a.smoke else 'Development diagnostic completed successfully.' if mode=='diagnostic' else 'Benchmark completed successfully.','32'),flush=True)
if __name__=='__main__':
    try:main()
    except (Exception,KeyboardInterrupt) as e:print('Campaign stopped: '+str(e),file=sys.stderr);sys.exit(1)
