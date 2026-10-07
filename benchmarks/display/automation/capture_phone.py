#!/usr/bin/env python3
"""Record with the installed benchmark camera helper; no camera UI navigation."""
import argparse, datetime, hashlib, json, statistics, subprocess, time, uuid
from pathlib import Path
PACKAGE = 'computer.extend.benchmarkcamera'

def validate_sensor(rows, fps):
    if len(rows) < fps: raise ValueError('Insufficient sensor samples')
    times = [row['sensor_ns'] for row in rows]
    gaps = [(b-a)/1e6 for a,b in zip(times,times[1:])]
    if any(g <= 0 for g in gaps): raise ValueError('Non-increasing sensor timestamps')
    hz = (len(times)-1)*1e9/(times[-1]-times[0])
    if not fps*.97 <= hz <= fps*1.03: raise ValueError('Capture cadence outside tolerance')
    ordered=sorted(gaps)
    return {'sensor_samples':len(rows),'measured_sensor_hz':hz,'sensor_gap_p50_ms':statistics.median(gaps),'sensor_gap_p95_ms':ordered[int(.95*(len(ordered)-1))], 'sensor_gap_max_ms':max(gaps)}

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--serial',required=True);p.add_argument('--adb',default='adb')
    p.add_argument('--output',required=True,type=Path);p.add_argument('--seconds',type=int,default=10)
    p.add_argument('--fps',type=int,choices=[120,240],default=240)
    p.add_argument('--probe',type=Path)
    p.add_argument('--quiet',action='store_true')
    p.add_argument('--ready-command',nargs=argparse.REMAINDER,help='Optional argv launched once first camera sensor sample arrives')
    a=p.parse_args()
    if not 1<=a.seconds<=60:p.error('seconds must be 1..60')
    a.output.mkdir(parents=True,exist_ok=False)
    raw=a.output/'raw';raw.mkdir()
    run_id=str(uuid.uuid4()); result={}; command=None
    def adb(*args, timeout=20):
        return subprocess.run([a.adb,'-s',a.serial,*args],capture_output=True,check=True,timeout=timeout).stdout
    def remote(name):return adb('exec-out','run-as',PACKAGE,'cat',f'files/{run_id}/{name}',timeout=90 if name=='video.mp4' else 10)
    def wait(name, deadline):
        while time.monotonic()<deadline:
            try:return json.loads(remote(name))
            except (subprocess.CalledProcessError,json.JSONDecodeError):time.sleep(.2)
        raise TimeoutError(f'Timed out waiting for {name}')
    try:
        adb('shell','am','start','-S','-n',PACKAGE+'/.CaptureActivity','--es','operation','record','--es','run_id',run_id,'--ei','fps',str(a.fps),'--ei','seconds',str(a.seconds),'--ei','width','1920','--ei','height','1080')
        deadline=time.monotonic()+a.seconds+30
        ready=wait('ready.json',deadline)
        if ready.get('status')!='recording':raise ValueError('Camera did not acknowledge recording')
        (a.output/'ready.json').write_text(json.dumps(ready,indent=2))
        print('Camera recording; ready for workload.',flush=True)
        if a.ready_command:command=subprocess.Popen(a.ready_command)
        result=wait('result.json',deadline)
        (a.output/'capture.json').write_text(json.dumps(result,indent=2))
        if result.get('status')!='complete':raise ValueError(f'Capture failed: {result.get("status")}')
        if result['requested_capture_fps']!=a.fps or result['width']!=1920 or result['height']!=1080:raise ValueError('Wrong capture profile')
        sensor=remote('sensor.jsonl');(a.output/'sensor.jsonl').write_bytes(sensor)
        metrics=validate_sensor([json.loads(row) for row in sensor.splitlines()],a.fps)
        video=remote('video.mp4'); path=raw/'video.mp4';path.write_bytes(video)
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
        (a.output/'summary.json').write_text(json.dumps(summary,indent=2))
        print(f'Capture verified: 1920x1080, {metrics["measured_sensor_hz"]:.2f} sensor fps.',flush=True)
        if not a.quiet:print(json.dumps(summary,indent=2))
        if command and command.wait(timeout=15)!=0:raise ValueError('Workload start command failed')
        adb('shell','run-as',PACKAGE,'rm','-rf',f'files/{run_id}')
    except BaseException as error:
        (a.output/'failure.json').write_text(json.dumps({'status':'failed','error':str(error),'run_id':run_id},indent=2))
        raise
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
