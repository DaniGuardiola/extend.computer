#!/usr/bin/env python3
"""Record with the installed benchmark camera helper; no camera UI navigation."""
import contextlib,sys, argparse, datetime, hashlib, json, math, statistics, struct, subprocess, time, uuid
from pathlib import Path
PACKAGE = 'computer.extend.benchmarkcamera'

def wait_receipt(remote, name, deadline, output, clock=time.monotonic, sleep=time.sleep):
    """Retain terminal camera failures even when recording never became ready."""
    while clock()<deadline:
        try:
            receipt=json.loads(remote(name))
        except (subprocess.CalledProcessError,json.JSONDecodeError):
            receipt=None
        if receipt is not None:return receipt
        if name=='ready.json':
            try:terminal=json.loads(remote('result.json'))
            except (subprocess.CalledProcessError,json.JSONDecodeError):terminal=None
            if terminal is not None:
                (output/'capture.json').write_text(json.dumps(terminal,indent=2))
                if terminal.get('status')!='complete':
                    raise ValueError(f'Camera preparation failed: {terminal.get("error") or terminal.get("status")}')
                raise ValueError('Camera completed without recording readiness receipt')
        sleep(.2)
    raise TimeoutError(f'Timed out waiting for {name}')

CAMERA_ORIENTATIONS = ('sensor-landscape','reverse-landscape')

def camera_orientation(value):
    if value not in CAMERA_ORIENTATIONS:raise ValueError('Invalid camera orientation request')
    return value

def validate_orientation(capture):
    if 'orientation_policy' not in capture:
        return {'status':'unverified_legacy_orientation'}
    if capture['orientation_policy']!='attached-landscape-stable-v1':
        raise ValueError('Unknown camera orientation policy')
    sensor,rotation,hint=(capture.get(k) for k in ('sensor_orientation','display_rotation','orientation_hint'))
    bounds=capture.get('window_bounds')
    request=camera_orientation(capture.get('requested_orientation','sensor-landscape'))
    if request=='reverse-landscape' and rotation!=3:raise ValueError('Camera did not acquire requested reverse landscape rotation')
    if any(type(v) is not int for v in (sensor,rotation,hint)) or sensor not in (0,90,180,270) or rotation not in range(4):
        raise ValueError('Invalid camera orientation provenance')
    if hint!=(sensor-rotation*90+360)%360:
        raise ValueError('Camera orientation hint differs from acquired rotation')
    if not isinstance(bounds,list) or len(bounds)!=4 or any(type(v) is not int for v in bounds) or bounds[:2]!=[0,0] or bounds[2]<=bounds[3] or bounds[3]<=0 or capture.get('orientation_stable_ms',0)<250:
        raise ValueError('Camera orientation was not acquired in stable landscape window')
    return {'status':'orientation_acquired','sensor_orientation':sensor,'display_rotation':rotation,'orientation_hint':hint,'window_bounds':bounds,'requested_orientation':request}

def exposure_steps(value):
    try:number=int(value)
    except (ValueError,TypeError):raise argparse.ArgumentTypeError('Exposure compensation must be integer -24..24 steps')
    if isinstance(value,bool) or str(number)!=str(value) or not -24<=number<=24:
        raise argparse.ArgumentTypeError('Exposure compensation must be integer -24..24 steps')
    return number

def validate_sensor(rows, fps):
    if len(rows) < fps: raise ValueError('Insufficient sensor samples')
    times = [row['sensor_ns'] for row in rows]
    gaps = [(b-a)/1e6 for a,b in zip(times,times[1:])]
    if any(g <= 0 for g in gaps): raise ValueError('Non-increasing sensor timestamps')
    hz = (len(times)-1)*1e9/(times[-1]-times[0])
    if not fps*.97 <= hz <= fps*1.03: raise ValueError('Capture cadence outside tolerance')
    ordered=sorted(gaps)
    frame_numbers=[row.get('frame_number') for row in rows]
    known=all(type(value) is int for value in frame_numbers)
    repeated=sum(a==b for a,b in zip(frame_numbers,frame_numbers[1:])) if known else None
    evidence={'kind':'batched_camera2_callbacks' if repeated else 'camera2_callbacks' if known else 'legacy_frame_identity_unavailable',
              'adjacent_repeated_frame_numbers':repeated,
              'unique_frame_numbers':len(set(frame_numbers)) if known else None,
              'limits':['Camera2 high-speed callbacks may contain framework-derived timestamps within a batch; callback cadence does not independently prove every sensor exposure or encoded frame time.']}
    return {'sensor_samples':len(rows),'measured_sensor_hz':hz,'sensor_gap_p50_ms':statistics.median(gaps),'sensor_gap_p95_ms':ordered[int(.95*(len(ordered)-1))], 'sensor_gap_max_ms':max(gaps),'sensor_timestamp_evidence':evidence}

def validate_focus_regions(regions):
    if not isinstance(regions,list) or not 1<=len(regions)<=2:raise ValueError('Invalid focus regions')
    for r in regions:
        if not isinstance(r,list) or len(r)!=4 or any(type(v) not in (int,float) or not math.isfinite(v) or not 0<=v<=1 for v in r) or r[0]>=r[2] or r[1]>=r[3]:raise ValueError('Invalid focus region rectangle')
    return regions

def validate_focus_distance_range(value):
    if not isinstance(value,list) or len(value)!=2 or any(type(v) not in (int,float) or not math.isfinite(v) for v in value) or not 0<=value[0]<value[1]<=100:
        raise ValueError('Invalid calibrated focus distance range')
    return value

def camera_zoom(value):
    if isinstance(value,bool):raise ValueError('Camera zoom must be 1..2')
    number=float(value)
    if not math.isfinite(number) or not 1<=number<=2:raise ValueError('Camera zoom must be 1..2')
    # ADB --ef and Camera2 use float32; compare the exact transmitted value.
    return struct.unpack('f',struct.pack('f',number))[0]

def validate_crop(rows,capture):
    zoom=capture.get('requested_zoom')
    if zoom is None:return {'status':'unverified_legacy_crop'}
    camera_zoom(zoom)
    crop=capture.get('requested_crop_sensor')
    if zoom==1:
        if crop is not None:raise ValueError('Unexpected crop for unzoomed capture')
        return {'status':'default_crop','requested_zoom':zoom}
    if not isinstance(crop,list) or len(crop)!=4 or any(type(v)!=int for v in crop) or crop[0]<0 or crop[1]<0 or crop[0]>=crop[2] or crop[1]>=crop[3]:raise ValueError('Requested camera crop missing or invalid')
    if not rows or any(row.get('crop_sensor')!=crop for row in rows):raise ValueError('Observed camera crop differs from requested crop')
    return {'status':'crop_verified','requested_zoom':zoom,'sensor_rectangle':crop}

def validate_focus(rows, capture):
    if capture.get('focus_policy') is None:
        return {'status':'unverified_legacy_autofocus'}
    if capture.get('focus_policy')!='settle-and-lock-v1':raise ValueError('Unknown focus policy')
    locked=capture.get('locked_focus_distance_diopters')
    if type(locked) not in (float,int) or not math.isfinite(locked) or locked<0:
        raise ValueError('Camera focus lock distance missing')
    distance_range=capture.get('requested_focus_distance_range_diopters')
    if distance_range is not None:
        validate_focus_distance_range(distance_range)
        if not distance_range[0]<=locked<=distance_range[1]:raise ValueError('Focus lock outside calibrated distance range')
    fixed=capture.get('fixed_focus_lens') is True
    if fixed and (capture.get('minimum_focus_distance_diopters')!=0 or locked!=0):raise ValueError('Fixed-focus lens metadata mismatch')
    requested=capture.get('requested_focus_regions_sensor')
    distances=[]
    for row in rows:
        value=row.get('focus_distance_diopters')
        if type(value) not in (float,int) or not math.isfinite(value) or abs(value-locked)>.02:
            raise ValueError('Observed camera focus drifted after lock')
        if distance_range is not None and not distance_range[0]<=value<=distance_range[1]:raise ValueError('Observed focus outside calibrated distance range')
        if not fixed and (row.get('af_mode')!=3 or row.get('af_state')!=4 or row.get('lens_state')!=0):
            raise ValueError('Observed camera autofocus was not locked')
        if requested is not None and row.get('af_regions_sensor')!=requested:
            raise ValueError('Observed autofocus regions differ from requested regions')
        distances.append(value)
    if not distances:raise ValueError('Camera focus telemetry missing')
    return {'status':'fixed_focus_verified' if fixed else 'autofocus_lock_verified',
        'locked_distance_diopters':locked,'observed_min_diopters':min(distances),
        'observed_max_diopters':max(distances),'preparation_s':capture.get('focus_preparation_s'),
        'limits':['Focus lock does not prove both screen planes are optimally sharp.']}

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--serial',required=True);p.add_argument('--adb',default='adb')
    p.add_argument('--output',required=True,type=Path);p.add_argument('--seconds',type=int,default=10)
    p.add_argument('--fps',type=int,choices=[120,240],default=240)
    p.add_argument('--exposure-compensation-steps',type=exposure_steps,default=0,help='Explicit camera diagnostic; camera verifies its own supported range. Default 0.')
    p.add_argument('--probe',type=Path)
    p.add_argument('--preparation-command-json',type=Path,help='Static fixture argv JSON; owned process stops before measured ready command')
    p.add_argument('--focus-distance-range',type=Path,help='Calibration JSON with accepted autofocus distance interval in diopters')
    p.add_argument('--camera-orientation',choices=CAMERA_ORIENTATIONS,help='Activity orientation request; default sensor-landscape')
    p.add_argument('--zoom',type=camera_zoom,default=None,help='Explicit sensor crop ratio, 1..2; keep identical for compared runs')
    p.add_argument('--focus-regions',type=Path,help='Calibration JSON containing one or two normalized sensor-output AF rectangles')
    modes=p.add_mutually_exclusive_group()
    modes.add_argument('--capture-only',action='store_true',help='Leave footage on phone for later collection')
    modes.add_argument('--collect-existing',action='store_true',help='Collect the completed capture described by OUTPUT/phone-session.json')
    p.add_argument('--quiet',action='store_true')
    p.add_argument('--ready-command',nargs=argparse.REMAINDER,help='Optional argv launched once first camera sensor sample arrives')
    a=p.parse_args()
    sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'campaign'))
    from progress import camera_progress
    ui=camera_progress()
    def step(label):return ui.step(label) if ui else contextlib.nullcontext()
    if not 1<=a.seconds<=60:p.error('seconds must be 1..60')
    a.output.mkdir(parents=True,exist_ok=a.collect_existing)
    raw=a.output/'raw';raw.mkdir(exist_ok=a.collect_existing)
    session_path=a.output/'phone-session.json'
    session=json.loads(session_path.read_text()) if a.collect_existing else {'run_id':str(uuid.uuid4()),'seconds':a.seconds,'fps':a.fps,'exposure_compensation_steps':a.exposure_compensation_steps}
    if a.zoom is not None and a.collect_existing:raise ValueError('Cannot change zoom during collection')
    if a.camera_orientation is not None and a.collect_existing:raise ValueError('Cannot change orientation during collection')
    if not a.collect_existing:session['camera_orientation']=a.camera_orientation or 'sensor-landscape'
    camera_orientation(session.get('camera_orientation','sensor-landscape'))
    if not a.collect_existing:session['zoom']=a.zoom if a.zoom is not None else 1.0
    if a.focus_regions:
        if a.collect_existing:raise ValueError('Cannot change focus regions during collection')
        regions=json.loads(a.focus_regions.read_text())
        validate_focus_regions(regions)
        session['focus_regions']=regions
    if a.focus_distance_range:
        if a.collect_existing:raise ValueError('Cannot change focus distance during collection')
        session['focus_distance_range']=validate_focus_distance_range(json.loads(a.focus_distance_range.read_text()))
    run_id=str(uuid.UUID(session['run_id']));a.seconds=int(session['seconds']);a.fps=int(session['fps'])
    compensation=exposure_steps(session.get('exposure_compensation_steps',0))
    if not 1<=a.seconds<=60 or a.fps not in (120,240):raise ValueError('Invalid saved capture profile')
    if not a.collect_existing:session_path.write_text(json.dumps(session,indent=2))
    if a.preparation_command_json and a.collect_existing:raise ValueError('Cannot prepare an existing capture')
    result={};command=None;preparation=None
    def adb(*args, timeout=20):
        return subprocess.run([a.adb,'-s',a.serial,*args],capture_output=True,check=True,timeout=timeout).stdout
    def remote(name):return adb('exec-out','run-as',PACKAGE,'cat',f'files/{run_id}/{name}',timeout=90 if name=='video.mp4' else 10)
    def wait(name, deadline):
        return wait_receipt(remote,name,deadline,a.output)
    try:
        if not a.collect_existing:
            with step('recording'):
                if a.preparation_command_json:
                    from preparation import Preparation
                    preparation=Preparation(a.preparation_command_json,a.output)
                    preparation.start()
                extra=[]
                if session.get('focus_regions'):extra=['--es','focus_regions',json.dumps(session['focus_regions'],separators=(',',':'))]
                if session.get('focus_distance_range'):extra+=['--es','focus_distance_range',json.dumps(session['focus_distance_range'],separators=(',',':'))]
                extra+=['--ef','zoom',str(session['zoom']),'--es','camera_orientation',session['camera_orientation']]
                adb('shell','am','start','-S','-n',PACKAGE+'/.CaptureActivity','--es','operation','record','--es','run_id',run_id,'--ei','fps',str(a.fps),'--ei','seconds',str(a.seconds),'--ei','width','1920','--ei','height','1080','--ei','exposure_compensation_steps',str(compensation),*extra)
                deadline=time.monotonic()+a.seconds+30
                ready=wait('ready.json',deadline)
                if ready.get('status')!='recording':raise ValueError('Camera did not acknowledge recording')
                (a.output/'ready.json').write_text(json.dumps(ready,indent=2))
                if not ui:print('Camera recording; ready for workload.',flush=True)
                if preparation:preparation.stop('camera_ready_before_measured_scene',require_live=True)
                if a.ready_command:command=subprocess.Popen(a.ready_command)
                result=wait('result.json',deadline)
                (a.output/'capture.json').write_text(json.dumps(result,indent=2))
                if result.get('status')!='complete':raise ValueError(f'Capture failed: {result.get("status")}')
                if result['requested_capture_fps']!=a.fps or result['width']!=1920 or result['height']!=1080:raise ValueError('Wrong capture profile')
                if result.get('requested_exposure_compensation_steps',0)!=compensation:raise ValueError('Camera did not acknowledge exposure compensation')
        else:
            result=json.loads((a.output/'capture.json').read_text())
            if json.loads(remote('result.json'))!=result:raise ValueError('Phone capture receipt differs from saved run')
            if result.get('requested_exposure_compensation_steps',0)!=compensation:raise ValueError('Camera did not acknowledge exposure compensation')
        if result.get('requested_focus_distance_range_diopters')!=session.get('focus_distance_range'):
            raise ValueError('Camera did not acknowledge calibrated focus distance range')
        if 'camera_orientation' in session and result.get('requested_orientation')!=session['camera_orientation']:raise ValueError('Camera did not acknowledge requested orientation')
        requested_zoom=session.get('zoom')
        if result.get('requested_zoom')!=(camera_zoom(requested_zoom) if requested_zoom is not None else None):raise ValueError('Camera did not acknowledge zoom')
        if result.get('requested_focus_regions_normalized')!=session.get('focus_regions'):
            raise ValueError('Camera did not acknowledge focus regions')
        if a.capture_only:
            if command and command.wait(timeout=15)!=0:raise ValueError('Workload start command failed')
            return
        with step('phone transfer'):
            sensor=remote('sensor.jsonl');(a.output/'sensor.jsonl').write_bytes(sensor)
            sensor_rows=[json.loads(row) for row in sensor.splitlines()]
            # Preserve the original video even if telemetry fails acceptance.
            # Invalid recordings must remain inspectable without the phone.
            video=remote('video.mp4'); path=raw/'video.mp4';path.write_bytes(video)
            metrics=validate_sensor(sensor_rows,a.fps)
            metrics['focus']=validate_focus(sensor_rows,result)
            metrics['crop']=validate_crop(sensor_rows,result)
            metrics['orientation']=validate_orientation(result)
        metadata=None
        if a.probe:
            subprocess.run([str(a.probe),str(path),str(a.output/'video.json')],check=True,timeout=120)
            metadata=json.loads((a.output/'video.json').read_text())
            if metadata['width']!=1920 or metadata['height']!=1080:raise ValueError('Wrong recorded dimensions')
            if metadata['video_sample_count'] < a.fps*max(1,a.seconds-.75):raise ValueError('Insufficient encoded samples')
            capture_tags=[float(tag['value']) for tag in metadata['capture_metadata'] if 'capture.fps' in tag['key']]
            if capture_tags != [float(a.fps)]:raise ValueError('Missing or mismatched original capture-rate metadata')
        phone={'model':adb('shell','getprop','ro.product.model').decode().strip(),'android_sdk':adb('shell','getprop','ro.build.version.sdk').decode().strip()}
        summary={'phone':phone,'schema_version':1,'run_id':run_id,'completed_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'status':'camera_check_passed','capture_fps':a.fps,'requested_seconds':a.seconds,**metrics,'video_sha256':hashlib.sha256(video).hexdigest(),'video_metadata':metadata,'optical_latency_calibrated':False}
        summary['capture_profile']={key:result.get(key) for key in ['helper_apk_sha256','requested_orientation','orientation_policy','sensor_orientation','display_rotation','orientation_hint','window_bounds','camera','width','height','requested_capture_fps','requested_exposure_compensation_steps','exposure_compensation_step_ev','focus_policy','fixed_focus_lens','focus_regions_coordinate_basis','requested_focus_regions_normalized','requested_focus_distance_range_diopters','requested_zoom','requested_crop_sensor']}
        (a.output/'summary.json').write_text(json.dumps(summary,indent=2))
        if not ui:print(f'Capture verified: 1920x1080, {metrics["measured_sensor_hz"]:.2f} callback fps.',flush=True)
        if not a.quiet:print(json.dumps(summary,indent=2))
        if command and command.wait(timeout=15)!=0:raise ValueError('Workload start command failed')
        adb('shell','run-as',PACKAGE,'rm','-rf',f'files/{run_id}')
    except BaseException as error:
        (a.output/'failure.json').write_text(json.dumps({'status':'failed','error':str(error),'error_type':type(error).__name__,
            'retryable':not isinstance(error,ValueError),'run_id':run_id},indent=2))
        raise
    finally:
        try:
            if preparation:preparation.stop()
        finally:
            if command and command.poll() is None:
                command.terminate()
                try:command.wait(timeout=5)
                except subprocess.TimeoutExpired:command.kill();command.wait()
            try:adb('shell','am','force-stop',PACKAGE)
            except (subprocess.SubprocessError,OSError):pass
if __name__=='__main__':
    try:main()
    except (Exception,KeyboardInterrupt) as error:
        import sys
        print(f'Camera capture failed: {error}',file=sys.stderr);sys.exit(1)
