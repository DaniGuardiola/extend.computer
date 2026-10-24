#!/usr/bin/env python3
"""Capture and validate optical benchmark footage without camera UI navigation."""
import contextlib, argparse, datetime, hashlib, importlib.metadata, json, os, platform, subprocess, sys
from pathlib import Path
from bootstrap import build_swift
ROOT=Path(__file__).resolve().parent
RUN_DEST=None
RUN_STAGE='setup'

def registration_fractions(capture_seconds, phase_seconds=None):
    # Post-recording margins intentionally outlive the source fixture. Do not
    # mistake their disappearing counters for a failure inside the scene.
    fractions=[.2,.65] if capture_seconds<15 else [.2,.8]
    if phase_seconds is not None:
        fractions[1]=min(fractions[1],phase_seconds*.8/capture_seconds)
    return fractions

def scene_window(samples, reference, seconds, factor):
    # Counter zero is the stationary preparation phase, before the measured
    # workload begins. Its window-appearance frames are not scene samples.
    present=[i for i,item in enumerate(samples) if item['counter_ids'][reference] is not None and item['counter_ids'][reference]>0]
    if not present:raise RuntimeError('Source counter was never readable')
    first,last=present[0],present[-1]
    duration=(samples[last]['playback_s']-samples[first]['playback_s'])*factor
    if duration < seconds*.9:raise RuntimeError('Source fixture visible for too little of the requested scene')
    return samples[first:last+1],{'definition':'First through last readable positive source counter; preparation counter zero and camera margins excluded','first_sample':first,'last_sample':last,'observed_seconds':duration,'expected_seconds':seconds,'raw_samples':len(samples)}

def validate_tracking(samples, count):
    coverage=[sum(len(row.get('tracked_corners',[[]]*count)[i])==4 for row in samples)/len(samples) for i in range(count)]
    if any(value<.95 for value in coverage):raise RuntimeError('Timing markers could not be tracked through the measured scene')
    return {'definition':'Per-frame rigid translation from four colored markers; maximum 6 pixels, corner residual at most 3 pixels','coverage_per_patch':coverage}

def preserve_cursor_preparation(directory, samples, cursor):
    # Event zero precedes positive scene counters. Keep its actual observations
    # for the cursor acceptance gate while content metrics retain their window.
    if cursor.get('pattern') not in {'coded-four-lane-v4','coded-four-lane-v5'}:
        return
    path=Path(directory)/'cursor-optical-frames.jsonl'
    path.write_text(''.join(json.dumps(row)+'\n' for row in samples))

def source_workload_provenance(command, repo, build):
    """Snapshot a direct local fixture before launch; never infer wrapped commands."""
    unknown={'status':'unknown','reason':'Ready command is not the direct local campaign workload'}
    if not command or Path(command[0]).resolve() != (Path(build)/'campaign-workload').resolve():
        return unknown
    binary=Path(command[0]).resolve()
    source=Path(repo)/'benchmarks/display/workloads/campaign-workload.swift'
    record=Path(build)/'campaign-workload.build.json'
    digest=lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
    receipt=json.loads(record.read_text())
    source_hash,binary_hash=digest(source),digest(binary)
    if receipt.get('source_sha256') != source_hash or receipt.get('binary_sha256') != binary_hash:
        raise ValueError('Direct source workload differs from its build receipt before capture')
    return {'status':'verified_direct_local','source_sha256':source_hash,'binary_sha256':binary_hash,
            'build_receipt_sha256':digest(record),'build':receipt}


def main():
    global RUN_DEST,RUN_STAGE
    from find_patches import find
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--serial')
    modes=p.add_mutually_exclusive_group()
    modes.add_argument('--capture-only',action='store_true')
    modes.add_argument('--collect-existing',action='store_true')
    modes.add_argument('--analyze-existing',action='store_true',help='Analyze already collected local footage without a phone')
    p.add_argument('--adb',default='adb')
    p.add_argument('--seconds',type=int,default=30)
    p.add_argument('--output',type=Path)
    p.add_argument('--allow-static',action='store_true')
    p.add_argument('--focus-regions',type=Path)
    p.add_argument('--focus-distance-range',type=Path)
    p.add_argument('--zoom',type=float)
    p.add_argument('--exposure-compensation-steps',type=int,help='Explicit camera AE compensation; helper verifies the supported range')
    p.add_argument('--preparation-command-json',type=Path)
    p.add_argument('--camera-orientation',choices=('sensor-landscape','reverse-landscape'))
    p.add_argument('--phase-seconds',type=int,help='Expected fixture duration, excluding camera margins')
    p.add_argument('--ready-command',nargs=argparse.REMAINDER)
    p.add_argument('--check',action='store_true',help='Camera-only local patch test, not a product benchmark')
    p.add_argument('--product',help='Label for an operator-established display connection')
    p.add_argument('--extend-repo',type=Path,default=ROOT.parents[2])
    a=p.parse_args()
    sys.path.insert(0,str(ROOT.parent/'campaign'))
    from progress import camera_progress
    ui=camera_progress()
    if not a.analyze_existing and not a.serial:p.error('--serial is required for phone operations')
    if (a.collect_existing or a.analyze_existing) and not a.output:p.error('Existing capture needs --output')
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
    if a.preparation_command_json:
        if a.collect_existing or a.analyze_existing:p.error('Cannot prepare during collection or analysis')
        cmd+=['--preparation-command-json',str(a.preparation_command_json)]
    if a.camera_orientation is not None:
        if a.collect_existing or a.analyze_existing:p.error('Cannot change orientation during collection or analysis')
        cmd+=['--camera-orientation',a.camera_orientation]
    if a.zoom is not None:
        if a.collect_existing or a.analyze_existing:p.error('Cannot change zoom during collection or analysis')
        cmd+=['--zoom',str(a.zoom)]
    if a.exposure_compensation_steps is not None:
        if a.collect_existing or a.analyze_existing:p.error('Cannot change exposure during collection or analysis')
        cmd+=['--exposure-compensation-steps',str(a.exposure_compensation_steps)]
    if a.focus_distance_range:
        if a.collect_existing or a.analyze_existing:p.error('Cannot change focus during collection or analysis')
        cmd+=['--focus-distance-range',str(a.focus_distance_range)]
    if a.focus_regions:
        if a.collect_existing or a.analyze_existing:p.error('Cannot change focus during collection or analysis')
        cmd+=['--focus-regions',str(a.focus_regions)]
    if a.capture_only:cmd+=['--capture-only']
    if a.collect_existing:cmd+=['--collect-existing']
    if a.check:
        target=build/'optical-patches'
        cmd+=['--ready-command',str(target),'calibration','15','30']
    if a.ready_command:cmd+=['--ready-command',*a.ready_command]
    # Hash the actual direct fixture before camera/source processes can start.
    workload_receipt = source_workload_provenance(a.ready_command,a.extend_repo,build) if not (a.collect_existing or a.analyze_existing) else None
    RUN_STAGE='collection'
    workload_path=dest/'source-workload.json'
    if not a.analyze_existing:
        try:
            subprocess.run(cmd,check=True)
        finally:
            # Retain the pre-capture snapshot even when capture fails after
            # creating its output directory; do not create a conflicting folder.
            if workload_receipt is not None and dest.is_dir():
                workload_path.write_text(json.dumps(workload_receipt,indent=2))
    if workload_receipt is not None:
        workload_path.write_text(json.dumps(workload_receipt,indent=2))
    else:
        workload_receipt=json.loads(workload_path.read_text()) if workload_path.exists() else {'status':'unknown','reason':'Original capture has no source workload receipt'}
    if a.capture_only:return
    RUN_STAGE='analysis'
    with ui.step('optical analysis') if ui else contextlib.nullcontext():
        fixture=a.extend_repo/'benchmarks/display/workloads/optical-patches.swift'
        provenance={'schema_version':1,'python':platform.python_version(),'macos':platform.mac_ver()[0],'cpu_architecture':platform.machine(),'fixture_sha256':hashlib.sha256(fixture.read_bytes()).hexdigest(),'product_label':a.product,'connection_setup':'operator-established','route_verified':False,'source_geometry_verified':False,'packages':{name:importlib.metadata.version(name) for name in ['numpy','Pillow']},'native_tools':{name:json.loads((build/f'{name}.build.json').read_text()) for name in ['probe','extract','read-cells','optical-patches']},'tool_source_sha256':{name:hashlib.sha256((ROOT/name).read_bytes()).hexdigest() for name in ['camera_run.py','capture_phone.py','find_patches.py']}}
        provenance['source_workload']=workload_receipt
        (dest/'provenance.json').write_text(json.dumps(provenance,indent=2))
        metadata=json.loads((dest/'video.json').read_text())
        frame=dest/'raw'/'registration.png'
        subprocess.run([str(tools['extract']),str(dest/'raw/video.mp4'),str(frame),str(metadata['duration_s']/2)],check=True,stdout=subprocess.DEVNULL if ui else None)
        patches=find(frame)
        required=1 if a.check else 2
        if len(patches)!=required:raise RuntimeError(f'Expected {required} visible timing patches, found {len(patches)}. Keep both inner screen corners in view.')
        stability=[]
        if a.product:
            import numpy as np
            for fraction in registration_fractions(a.seconds,a.phase_seconds):
                sample_image=dest/'raw'/f'registration-{fraction}.png'
                subprocess.run([str(tools['extract']),str(dest/'raw/video.mp4'),str(sample_image),str(metadata['duration_s']*fraction)],check=True,stdout=subprocess.DEVNULL if ui else None)
                observed=find(sample_image)
                if len(observed)!=2:raise RuntimeError('Timing patches not visible across measured phase')
                movement=float(np.max(np.linalg.norm(np.asarray(observed)-np.asarray(patches),axis=2)))
                if movement>6:raise RuntimeError('Camera or timing patch geometry moved; repeat with a fixed phone')
                stability.append({'sample_fraction':fraction,'max_corner_movement_pixels':movement,
                    'frame_timing':json.loads(Path(str(sample_image)+'.timing.json').read_text())})
        (dest/'registration-stability.json').write_text(json.dumps(stability,indent=2))
        (dest/'patches.json').write_text(json.dumps(patches))
        reader_args=[str(tools['read-cells']),str(dest/'raw/video.mp4'),str(dest/'patches.json'),str(dest/'cells.jsonl')]
        source_path=dest.parent/'source.json'
        cursor={}
        if source_path.exists():
            source=json.loads(source_path.read_text())
            cursor=source.get('fixture',source).get('cursor',{})
            if cursor.get('pattern') in {'coded-four-lane-v3','coded-four-lane-v4','coded-four-lane-v5'}:
                if cursor.get('center_y_logical')!=148 or cursor.get('reference_origin_y_logical')!=132:
                    raise ValueError('Unsupported coded cursor placement')
                reader_args.append('148')
                if cursor.get('pattern')=='coded-four-lane-v5':
                    if cursor.get('reference_center_y_logical')!=164 or cursor.get('reference_shape_logical_size')!=[96,64]:raise ValueError('Unsupported enlarged cursor reference')
                    from metrics import source_patch
                    reader_args+=['2',str(source_patch(frame,patches))]
        subprocess.run(reader_args,check=True,stdout=subprocess.DEVNULL if ui else None)
        samples=[];counts=[0]*len(patches);values=[set() for _ in patches]
        for line in (dest/'cells.jsonl').read_text().splitlines():
            item=json.loads(line);decoded=[decode(rows) for rows in item['patches']]
            samples.append({'sample':item['sample'],'playback_s':item['playback_s'],'counter_ids':decoded,'tracked_corners':item['tracked_corners'],
                'cursor_scores':item.get('cursor_scores'),'cursor_positions':item.get('cursor_positions'),
                'cursor_events':item.get('cursor_events'),'patches':item['patches']})
            for i,value in enumerate(decoded):
                if value is not None:counts[i]+=1;values[i].add(value)
        preserve_cursor_preparation(dest,samples,cursor)
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
    if not ui:print(f'Timing patches verified: {len(patches)}; readable frames: {counts}.')
    if not ui:print('Saved raw footage and counter IDs. Physical latency awaits calibration.')
    if not ui:print(f'Results: {dest}')
if __name__=='__main__':
    try:main()
    except BaseException as error:
        if RUN_DEST and RUN_DEST.is_dir():
            (RUN_DEST/'optical-failure.json').write_text(json.dumps({'status':'invalid','error':str(error),
                'stage':RUN_STAGE,'error_type':type(error).__name__,
                'retryable':isinstance(error,(subprocess.SubprocessError,OSError))},indent=2))
            print(f'Invalid run retained: {RUN_DEST}',file=sys.stderr)
        print(f'Optical capture failed: {error}',file=sys.stderr)
        sys.exit(1)
