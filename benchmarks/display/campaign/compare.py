"""Compare campaign reports only when their observed contexts are compatible."""
import argparse,json,math,re
from pathlib import Path
from metrics import stats


def planned_scenes(report):
    settings=report['settings']
    return settings.get('scene_selection') or ['static','scroll','panel','motion','recovery']+(['cursor'] if settings.get('include_cursor') else [])+(['cursor-optical','cursor-sweep'] if settings.get('include_optical_cursor') else [])


def verified_pipeline(value):
    required={'camera_sources':{'camera_run.py','capture_phone.py','find_patches.py'},
              'native_sources':{'probe','extract','read-cells','optical-patches'},
              'metric_sources':{'metrics.py','optical_cursor.py','optical_motion.py','optical_signal.py'}}
    if not isinstance(value,dict) or value.get('frame_selection')!='zero-tolerance-v1':return False
    for field,names in required.items():
        sources=value.get(field)
        if not isinstance(sources,dict) or not names<=sources.keys():return False
        if any(type(digest) is not str or re.fullmatch('[0-9a-f]{64}',digest) is None for digest in sources.values()):return False
    return True

def acquisition_profile(phase):
    value=phase.get('camera',{}).get('capture_profile')
    if not isinstance(value,dict):return None
    digest=value.get('helper_apk_sha256')
    if type(digest) is not str or re.fullmatch('[0-9a-f]{64}',digest) is None:return None
    if not isinstance(value.get('camera'),str):return None
    for field in ['width','height','requested_capture_fps','requested_exposure_compensation_steps']:
        if type(value.get(field)) is not int:return None
    if value['width']<=0 or value['height']<=0 or value['requested_capture_fps'] not in (120,240):return None
    ev=value.get('exposure_compensation_step_ev')
    if type(ev) not in (int,float) or not math.isfinite(ev) or ev<=0:return None
    policy=value.get('focus_policy') or 'legacy-continuous-autofocus'
    if policy not in {'legacy-continuous-autofocus','settle-and-lock-v1'}:return None
    if policy=='settle-and-lock-v1' and type(value.get('fixed_focus_lens')) is not bool:return None
    regions=value.get('requested_focus_regions_normalized')
    if regions is not None:
        if not isinstance(regions,list) or not 1<=len(regions)<=2:return None
        for r in regions:
            if not isinstance(r,list) or len(r)!=4 or any(type(v) not in (int,float) or not math.isfinite(v) or not 0<=v<=1 for v in r) or r[0]>=r[2] or r[1]>=r[3]:return None
    distance_range=value.get('requested_focus_distance_range_diopters')
    if distance_range is not None:
        from capture_phone import validate_focus_distance_range
        try:validate_focus_distance_range(distance_range)
        except ValueError:return None
    zoom=value.get('requested_zoom')
    crop=value.get('requested_crop_sensor')
    if zoom is not None:
        if type(zoom) not in (float,int) or not math.isfinite(zoom) or not 1<=zoom<=2:return None
        if zoom>1 and (not isinstance(crop,list) or len(crop)!=4 or any(type(v)!=int for v in crop) or crop[0]<0 or crop[1]<0 or crop[0]>=crop[2] or crop[1]>=crop[3]):return None
        if zoom==1 and crop is not None:return None
    elif crop is not None:return None
    return {**{key:value[key] for key in ['helper_apk_sha256','camera','width','height',
        'requested_capture_fps','requested_exposure_compensation_steps','exposure_compensation_step_ev']},
        'requested_focus_regions_normalized':value.get('requested_focus_regions_normalized'),
        'requested_focus_distance_range_diopters':distance_range,
        'requested_zoom':zoom,'requested_crop_sensor':crop,
        'focus_policy':policy,'fixed_focus_lens':value.get('fixed_focus_lens') if policy=='settle-and-lock-v1' else None}


def comparable(a,b):
    problems=[]
    if a['settings'].get('scene_selection')!=b['settings'].get('scene_selection'):
        problems.append('Selected scenes differ')
    if a['settings'].get('campaign_profile','full')!=b['settings'].get('campaign_profile','full'):
        problems.append('Campaign profile differs')
    if a['settings'].get('capture_schedule','interleaved')!=b['settings'].get('capture_schedule','interleaved'):
        problems.append('Capture schedule differs')
    if bool(a['settings'].get('include_cursor'))!=bool(b['settings'].get('include_cursor')):
        problems.append('Optional cursor phase differs')
    if bool(a['settings'].get('include_optical_cursor'))!=bool(b['settings'].get('include_optical_cursor')):
        problems.append('Optical cursor phases differ')
    if any(scene.startswith('cursor') for scene in planned_scenes(a)) and a['settings'].get('cursor_input_method','warp')!=b['settings'].get('cursor_input_method','warp'):
        problems.append('Cursor input method differs')
    for role in ['source','receiver']:
        for field in ['model','chip','memory_bytes','os','architecture']:
            if a['context'][role]['host'][field]!=b['context'][role]['host'][field]:problems.append(role+' hardware/OS differs: '+field)
    for field in ['width','height','scale','pixel_width','pixel_height','refresh_hz']:
        if a['context']['source_display'][field]!=b['context']['source_display'][field]:problems.append('Source geometry differs: '+field)
    for role in ['source','receiver']:
        if a['context'][role]['host'].get('power',{}).get('source')!=b['context'][role]['host'].get('power',{}).get('source'):problems.append(role+' power source differs')
    if a['context'].get('phone')!=b['context'].get('phone'):problems.append('Camera device differs')
    for field in ['renderer','seconds','repetitions','mode','workload_sha256']:
        if a['settings'][field]!=b['settings'][field]:problems.append('Campaign setting differs: '+field)
    if a['settings'].get('requested_fps')!=b['settings'].get('requested_fps'):problems.append('Observed requested FPS differs or is unavailable on one product')
    def physical(report):return [{k:d.get(k) for k in ['width','height','scale','pixel_width','pixel_height','refresh_hz']} for d in report['context']['receiver'].get('displays',[]) if d.get('builtin')]
    if physical(a)!=physical(b):problems.append('Receiver physical display geometry differs')
    for report in [a,b]:
        expected={(repeat,scene) for repeat in range(1,report['settings']['repetitions']+1) for scene in planned_scenes(report)}
        phases=report['phases'];observed=[(p.get('repetition'),p.get('scene')) for p in phases]
        if set(observed)!=expected or len(observed)!=len(expected) or any(p.get('status')!='measured' for p in phases):
            problems.append(report['product']+' requested scenes not fully measured')
        if report.get('product')=='extend':
            for role in ['source','receiver']:
                build=report['context'][role].get('application',{}).get('instrumentation',{}).get('build',{})
                if build.get('status')!='observed_compile_flags' or build.get('debug_assertions') is not False:
                    problems.append('extend '+role+' non-debug build not confirmed')
        if any(p.get('network_summary',{}).get(role,{}).get('dominant_flow_matches_peer') is not True for p in report['phases'] for role in ['source','receiver']):problems.append(report['product']+' direct peer route not confirmed for every scene')
    for report in [a,b]:
        pipelines=[phase.get('analysis_pipeline') for phase in report['phases']]
        if not pipelines or any(not verified_pipeline(value) for value in pipelines):
            problems.append(report['product']+' optical analysis provenance unavailable or legacy')
    left={json.dumps(phase.get('analysis_pipeline'),sort_keys=True) for phase in a['phases']}
    right={json.dumps(phase.get('analysis_pipeline'),sort_keys=True) for phase in b['phases']}
    if len(left)!=1 or left!=right:problems.append('Optical analysis pipelines differ')
    profiles=[]
    for report in [a,b]:
        observed=[acquisition_profile(phase) for phase in report['phases']]
        if not observed or any(value is None for value in observed):
            problems.append(report['product']+' camera acquisition provenance unavailable or legacy')
        profiles.append({json.dumps(value,sort_keys=True) for value in observed})
    if len(profiles[0])!=1 or profiles[0]!=profiles[1]:problems.append('Camera acquisition profiles differ')
    if any(r['status']!='complete_with_declared_limits' for r in [a,b]):problems.append('Incomplete campaign')
    return problems

def compare(a,b):
    reasons=comparable(a,b);rows=[]
    for scene in dict.fromkeys(planned_scenes(a)+planned_scenes(b)):
        for metric,path in [('receiver_visible_hz',['optical','receiver_visible_hz']),('screen_to_screen_ms',['optical','screen_to_screen_transition_ms','median']),('screen_to_screen_p95_ms',['optical','screen_to_screen_transition_ms','p95']),('receiver_frame_gap_p95_ms',['optical','receiver_update_intervals_ms','p95']),('receiver_frame_gap_max_ms',['optical','receiver_update_intervals_ms','max']),('receiver_intervals_over_50ms',['optical','intervals_over_50ms']),('optical_ssim',['quality','global_optical_ssim']),('source_cpu',['resources','source','cpu_percent_one_core','median']),('receiver_cpu',['resources','receiver','cpu_percent_one_core','median'])]:
            values=[]
            for report in [a,b]:
                samples=[]
                for p in report['phases']:
                    if p['scene']!=scene:continue
                    value=p
                    for key in path:value=value.get(key) if isinstance(value,dict) else None
                    if type(value) in (int,float) and math.isfinite(value):samples.append(value)
                values.append(stats(samples))
            rows.append({'scene':scene,'metric':metric,'left':values[0],'right':values[1]})
        if scene=='cursor':
            for metric,path in [('source_cursor_submitted_hz',['source','cursor','stages','cursor_position_submitted','hz']),
                    ('receiver_cursor_layer_update_hz',['receiver','cursor','stages','cursor_layer_submit','hz']),
                    ('receiver_cursor_inter_update_p95_ms',['receiver','cursor','stages','cursor_layer_submit','inter_update_ms','p95']),
                    ('receiver_cursor_last_update_age_ms',['receiver','cursor','stages','cursor_layer_submit','last_update_age_ms'])]:
                values=[]
                for report in [a,b]:
                    samples=[]
                    for phase in report['phases']:
                        if phase['scene']!='cursor':continue
                        value=phase.get('stages',{})
                        for key in path:value=value.get(key) if isinstance(value,dict) else None
                        if type(value) in (int,float) and math.isfinite(value):samples.append(value)
                    values.append(stats(samples))
                rows.append({'scene':scene,'metric':metric,'left':values[0],'right':values[1]})
        extra=[('optical_timing_interval_width_p95_ms',['optical','transition_delay_interval_width_ms','p95']),
               ('optical_signal_alignment_correlation',['signal_alignment','correlation']),
               ('source_counter_readable_coverage',['optical','counter_readable_coverage','source']),
               ('receiver_counter_readable_coverage',['optical','counter_readable_coverage','receiver'])]
        if scene=='cursor-optical':
            extra+=[('physical_cursor_median_ms',['physical_cursor','state_reference_to_cursor_ms','median']),
                   ('physical_cursor_p95_ms',['physical_cursor','state_reference_to_cursor_ms','p95']),
                   ('physical_cursor_max_ms',['physical_cursor','state_reference_to_cursor_ms','max']),
                   ('physical_cursor_matched_coverage',['physical_cursor','matched_transition_coverage'])]
        if scene=='cursor-sweep':
            extra+=[('receiver_resolvable_motion_gap_p95_ms',['physical_cursor_motion','receiver','inter_resolvable_change_ms','p95']),
                   ('receiver_confirmed_hold_max_ms',['physical_cursor_motion','longest_observed_receiver_hold_ms']),
                   ('joint_cursor_readable_coverage',['physical_cursor_motion','joint_readable_coverage']),
                   ('receiver_unresolved_motion_gap_max_ms',['physical_cursor_motion','receiver','change_gaps_containing_unreadable_frames_ms','max'])]
        for metric,path in extra:
            values=[]
            for report in [a,b]:
                samples=[]
                for phase in report['phases']:
                    if phase['scene']!=scene:continue
                    value=phase
                    for key in path:value=value.get(key) if isinstance(value,dict) else None
                    if type(value) in (int,float) and math.isfinite(value):samples.append(value)
                values.append(stats(samples))
            rows.append({'scene':scene,'metric':metric,'left':values[0],'right':values[1]})
    return {'schema_version':1,'left':a['product'],'right':b['product'],'comparable_context':not reasons,'reasons':reasons,'metrics':rows,
            'limits':['Same physical camera placement and stable transport still require reviewing run evidence.','No overall winner inferred from frame rate alone.','Optical SSIM includes physical panels and camera; do not compare to codec-only quality scores.','Cursor hooks remain host-local. Optical cursor phases separately measure software-reference latency or resolvable motion, not physical input latency. Zero confirmed holds does not prove no stalls.']}

def main():
    p=argparse.ArgumentParser();p.add_argument('left',type=Path);p.add_argument('right',type=Path);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
    data=compare(json.loads((a.left/'report.json').read_text()),json.loads((a.right/'report.json').read_text()))
    a.output.mkdir(parents=True,exist_ok=False);(a.output/'comparison.json').write_text(json.dumps(data,indent=2))
    lines=['# Display comparison','',f"Comparable observed context: {data['comparable_context']}",'']
    lines+=data['reasons'];lines+=['','| Scene | Metric | '+data['left']+' | '+data['right']+' |','|---|---|---:|---:|']
    for row in data['metrics']:
        def fmt(v):return 'unavailable' if v is None else f"{v['median']:.2f} (n={v['count']})"
        lines.append('| '+row['scene']+' | '+row['metric']+' | '+fmt(row['left'])+' | '+fmt(row['right'])+' |')
    lines+=['',*data['limits']];(a.output/'comparison.md').write_text('\n'.join(lines)+'\n')
    print('Comparison: '+str(a.output))
if __name__=='__main__':main()
