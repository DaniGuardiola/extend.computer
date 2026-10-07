#!/usr/bin/env python3
"""Capture and validate optical benchmark footage without camera UI navigation."""
import argparse, datetime, hashlib, importlib.metadata, json, os, platform, subprocess, sys
from pathlib import Path
from bootstrap import build_swift
ROOT=Path(__file__).resolve().parent
RUN_DEST=None

def scene_window(samples, reference, seconds, factor):
    present=[i for i,item in enumerate(samples) if item['counter_ids'][reference] is not None]
    if not present:raise RuntimeError('Source counter was never readable')
    first,last=present[0],present[-1]
    duration=(samples[last]['playback_s']-samples[first]['playback_s'])*factor
    if duration < seconds*.9:raise RuntimeError('Source fixture visible for too little of the requested scene')
    return samples[first:last+1],{'definition':'First through last readable source counter; camera pre/post margins excluded','first_sample':first,'last_sample':last,'observed_seconds':duration,'expected_seconds':seconds,'raw_samples':len(samples)}

def validate_tracking(samples, count):
    coverage=[sum(len(row.get('tracked_corners',[[]]*count)[i])==4 for row in samples)/len(samples) for i in range(count)]
    if any(value<.95 for value in coverage):raise RuntimeError('Timing markers could not be tracked through the measured scene')
    return {'definition':'Per-frame rigid translation from four colored markers; maximum 6 pixels, corner residual at most 3 pixels','coverage_per_patch':coverage}

def main():
    global RUN_DEST
    from find_patches import find
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--serial',required=True)
    p.add_argument('--adb',default='adb')
    p.add_argument('--seconds',type=int,default=30)
    p.add_argument('--output',type=Path)
    p.add_argument('--allow-static',action='store_true')
    p.add_argument('--phase-seconds',type=int,help='Expected fixture duration, excluding camera margins')
    p.add_argument('--ready-command',nargs=argparse.REMAINDER)
    p.add_argument('--check',action='store_true',help='Camera-only local patch test, not a product benchmark')
    p.add_argument('--product',help='Label for an operator-established display connection')
    p.add_argument('--extend-repo',type=Path,default=ROOT.parents[2])
    a=p.parse_args()
    if not 1<=a.seconds<=60:p.error('seconds must be 1..60')
    if not a.check and not a.product:p.error('Use --check or label the actual connection with --product')
    sys.path.insert(0,str(a.extend_repo))
    from benchmarks.display.optical.decode import decode
    build=build_swift(a.extend_repo)
    tools={name:build/name for name in ['probe','extract','read-cells']}
    stamp=datetime.datetime.now().strftime('%Y%m%d-%H%M%S-%f')
    dest=a.output or Path(os.environ.get('DISPLAY_BENCH_RUNS',str(ROOT/'runs')))/f'{a.product or "camera-check"}-{stamp}'
    RUN_DEST=dest
    cmd=[sys.executable,str(ROOT/'capture_phone.py'),'--serial',a.serial,'--adb',a.adb,'--seconds',str(6 if a.check else a.seconds),'--output',str(dest),'--quiet','--probe',str(tools['probe'])]
    if a.check:
        target=build/'optical-patches'
        cmd+=['--ready-command',str(target),'calibration','15','30']
    if a.ready_command:cmd+=['--ready-command',*a.ready_command]
    subprocess.run(cmd,check=True)
    fixture=a.extend_repo/'benchmarks/display/workloads/optical-patches.swift'
    provenance={'schema_version':1,'python':platform.python_version(),'macos':platform.mac_ver()[0],'cpu_architecture':platform.machine(),'fixture_sha256':hashlib.sha256(fixture.read_bytes()).hexdigest(),'product_label':a.product,'connection_setup':'operator-established','route_verified':False,'source_geometry_verified':False,'packages':{name:importlib.metadata.version(name) for name in ['numpy','Pillow']},'native_tools':{name:json.loads((build/f'{name}.build.json').read_text()) for name in ['probe','extract','read-cells','optical-patches']},'tool_source_sha256':{name:hashlib.sha256((ROOT/name).read_bytes()).hexdigest() for name in ['camera_run.py','capture_phone.py','find_patches.py']}}
    (dest/'provenance.json').write_text(json.dumps(provenance,indent=2))
    metadata=json.loads((dest/'video.json').read_text())
    frame=dest/'raw'/'registration.png'
    subprocess.run([str(tools['extract']),str(dest/'raw/video.mp4'),str(frame),str(metadata['duration_s']/2)],check=True)
    patches=find(frame)
    required=1 if a.check else 2
    if len(patches)!=required:raise RuntimeError(f'Expected {required} visible timing patches, found {len(patches)}. Keep both inner screen corners in view.')
    stability=[]
    if a.product:
        import numpy as np
        for fraction in ([.2,.65] if a.seconds<15 else [.2,.8]):
            sample_image=dest/'raw'/f'registration-{fraction}.png'
            subprocess.run([str(tools['extract']),str(dest/'raw/video.mp4'),str(sample_image),str(metadata['duration_s']*fraction)],check=True)
            observed=find(sample_image)
            if len(observed)!=2:raise RuntimeError('Timing patches not visible across measured phase')
            movement=float(np.max(np.linalg.norm(np.asarray(observed)-np.asarray(patches),axis=2)))
            if movement>6:raise RuntimeError('Camera or timing patch geometry moved; repeat with a fixed phone')
            stability.append({'sample_fraction':fraction,'max_corner_movement_pixels':movement})
    (dest/'registration-stability.json').write_text(json.dumps(stability,indent=2))
    (dest/'patches.json').write_text(json.dumps(patches))
    subprocess.run([str(tools['read-cells']),str(dest/'raw/video.mp4'),str(dest/'patches.json'),str(dest/'cells.jsonl')],check=True)
    samples=[];counts=[0]*len(patches);values=[set() for _ in patches]
    for line in (dest/'cells.jsonl').read_text().splitlines():
        item=json.loads(line);decoded=[decode(rows) for rows in item['patches']]
        samples.append({'sample':item['sample'],'playback_s':item['playback_s'],'counter_ids':decoded,'tracked_corners':item['tracked_corners']})
        for i,value in enumerate(decoded):
            if value is not None:counts[i]+=1;values[i].add(value)
    if a.phase_seconds and a.product:
        sys.path.insert(0,str(ROOT.parent/'campaign'))
        from metrics import source_patch
        reference=source_patch(frame,patches)
        sensor=json.loads((dest/'summary.json').read_text())
        factor=sensor['video_metadata']['nominal_playback_fps']/sensor['measured_sensor_hz']
        samples,window=scene_window(samples,reference,a.phase_seconds,factor)
        (dest/'optical-window.json').write_text(json.dumps(window,indent=2))
        counts=[sum(row['counter_ids'][i] is not None for row in samples) for i in range(len(patches))]
    tracking=validate_tracking(samples,len(patches))
    (dest/'tracking.json').write_text(json.dumps(tracking,indent=2))
    values=[{row['counter_ids'][i] for row in samples if row['counter_ids'][i] is not None} for i in range(len(patches))]
    if any(c<len(samples)*.5 or len(v)<(1 if a.allow_static else 20) for c,v in zip(counts,values)):raise RuntimeError('Insufficient readable counter coverage')
    (dest/'optical-frames.jsonl').write_text(''.join(json.dumps(s)+'\n' for s in samples))
    report={'status':'optical_camera_check_passed' if a.check else 'optical_recording_ready_for_review','product':a.product,'decoded_camera_frames':len(samples),'readable_frames_per_patch':counts,'distinct_counter_ids_per_patch':[len(v) for v in values],'physical_latency_calibrated':False,'notes':['Counter IDs identify visible updates; null means ambiguous or corrupt.','Product benchmark requires both patches generated by the same source fixture.','Cell positions track small common image shifts in each frame; large movement and missing markers remain invalid.']}
    (dest/'optical.json').write_text(json.dumps(report,indent=2))
    print(f'Timing patches verified: {len(patches)}; readable frames: {counts}.')
    print('Saved raw footage and counter IDs. Physical latency awaits calibration.')
    print(f'Results: {dest}')
if __name__=='__main__':
    try:main()
    except BaseException as error:
        if RUN_DEST and RUN_DEST.is_dir():
            (RUN_DEST/'optical-failure.json').write_text(json.dumps({'status':'invalid','error':str(error)},indent=2))
            print(f'Invalid run retained: {RUN_DEST}',file=sys.stderr)
        print(f'Optical capture failed: {error}',file=sys.stderr)
        sys.exit(1)
