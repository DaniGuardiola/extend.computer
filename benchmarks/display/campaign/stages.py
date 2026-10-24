"""Numeric stage evidence. Frame IDs are joined only within one host clock."""
import collections,json,statistics
from pathlib import Path
from metrics import stats

def encoder_bitrate_evidence(rows,start,end):
    """Retain driver snapshots separately from requested or attempted bitrate."""
    names={'encoder_bitrate_read_status','encoder_bitrate_value_valid','encoder_bitrate_value','bitrate_set_status'}
    events=sorted((r for r in rows if r.get('stage') in names and r.get('at_ns',end+1)<=end),key=lambda r:r['at_ns'])
    readbacks=[];setters=[];pending=None
    def finish(group):
        value=group.get('bps')
        group['verified']=group.get('read_status')==0 and group.get('value_valid')==1 and type(value) is int and value>0
        readbacks.append(group)
    for row in events:
        stage=row['stage'];at=row['at_ns'];value=row.get('bytes')
        if stage=='bitrate_set_status':
            setters.append({'at_ns':at,'status':value});continue
        if stage=='encoder_bitrate_read_status':
            if pending is not None:finish(pending)
            pending={'at_ns':at,'read_status':value};continue
        if pending is None:
            pending={'at_ns':at}
        pending['at_ns']=at
        if stage=='encoder_bitrate_value_valid':
            pending['value_valid']=value
            if value!=1:finish(pending);pending=None
        elif stage=='encoder_bitrate_value':
            pending['bps']=value;finish(pending);pending=None
    if pending is not None:finish(pending)
    prior=[r for r in readbacks if r['at_ns']<=start]
    initial=prior[-1] if prior else None
    observed=[r for r in readbacks if start<r['at_ns']<=end]
    in_window=[r for r in setters if start<=r['at_ns']<=end]
    initial_usable=bool(initial and initial['verified'] and not any(initial['at_ns']<r['at_ns']<=start for r in setters))
    relevant_setters=[r for r in setters if r['at_ns']>=(initial['at_ns'] if initial else start)]
    unverified_setters=[]
    for setter in relevant_setters:
        following=next((r for r in readbacks if r['at_ns']>=setter['at_ns']),None)
        if following is None or not following['verified']:unverified_setters.append(setter)
    failures=[r for r in in_window if r['status']!=0]
    verified=initial_usable and all(r['verified'] for r in observed) and not failures and not unverified_setters
    valid=[r for r in ([initial] if initial else [])+observed if r['verified']]
    values=sorted({r['bps'] for r in valid})
    latest=(observed[-1] if observed else initial)
    final_usable=bool(latest and latest['verified'] and not any(r['at_ns']>latest['at_ns'] for r in setters))
    return {'status':'measured' if verified else 'not_verified' if readbacks else 'unavailable',
            'whole_window_verified':bool(verified),'unit':'bps',
            'initial_bps':initial['bps'] if initial_usable else None,
            'final_bps':latest['bps'] if final_usable else None,
            'constant_bps':values[0] if verified and len(values)==1 else None,
            'observed_values_bps':values,'initial_readback':initial,'readbacks_in_window':observed,
            'setter_failures_in_window':failures,'unverified_setters':unverified_setters,
            'reason':None if verified else ('No native AverageBitRate driver readback recorded' if not readbacks else 'Missing, failed or incomplete driver verification in measurement window'),
            'scope':'Host-local VideoToolbox AverageBitRate snapshots; value held between verified updates is inferred. Requested configuration and encoded payload rate remain separate.'}

def summarize(rows,start,end):
    selected=[r for r in rows if start<=r.get('at_ns',-1)<=end and 'stage' in r]
    duration=(end-start)/1e9
    by={};config={}
    for r in rows:
        if r.get('kind')=='configuration' and r.get('at_ns',0)<=end:config[r['key']]=r['value']
        if r.get('stage')=='codec' and r.get('at_ns',0)<=end:config['codec']='hevc' if r.get('bytes')==265 else 'h264'
        if r.get('stage')=='separate_cursor' and r.get('at_ns',0)<=end:config['separate_cursor']=bool(r.get('bytes'))
        if r.get('stage','').startswith('config_') and r.get('at_ns',0)<=end:
            key=r['stage'][7:];config[key]=bool(r.get('bytes')) if key=='retina' else r.get('bytes')
        if r.get('stage') in {'encoder_hardware','decoder_hardware'} and r.get('at_ns',0)<=end:config[r['stage']]=bool(r.get('bytes'))
        if r.get('stage') in {'encoder_low_latency_requested','encoder_low_latency_auto','encoder_low_latency_fallback','encoder_low_latency'} and r.get('at_ns',0)<=end:
            config[r['stage']]=bool(r.get('bytes'))
        if r.get('stage') in {'encoder_low_latency_create_status','encoder_standard_create_status'} and r.get('at_ns',0)<=end:
            config[r['stage']]=r.get('bytes')
    for r in selected:
        key=str(r.get('frame_ns',0));stage=r['stage']
        if stage.endswith(('_us','_pixel_format')):continue
        by.setdefault(stage,{})
        # Zero is not a valid frame ID for a layer update.
        if key!='0':by[stage][key]=r
    def latency(before,after):
        a,b=by.get(before,{}),by.get(after,{})
        return stats([(b[k]['at_ns']-a[k]['at_ns'])/1e6 for k in a.keys()&b.keys() if b[k]['at_ns']>=a[k]['at_ns']])
    def datagrams(stage):
        packets=sorted([r for r in selected if r['stage']==stage],key=lambda r:r['at_ns'])
        if not packets:return None
        return {'count':len(packets),'udp_payload_bytes':sum(r.get('bytes',0) for r in packets),
                'inter_datagram_ms':stats([(b['at_ns']-a['at_ns'])/1e6 for a,b in zip(packets,packets[1:])])}
    stages={s:len(v)/duration for s,v in by.items()} if duration>0 else {}
    layer=sum(r['stage']=='layer_submit' for r in selected)
    if layer:stages['layer_submit']=layer/duration
    explicit_gpu=any(r.get('stage')=='gpu_completed' for r in selected)
    legacy_presentation=any(r.get('stage')=='presented' for r in selected) and not explicit_gpu
    gpu_stage='gpu_completed' if explicit_gpu else 'presented'
    timestamps=[r['at_ns'] for r in rows if 'stage' in r and isinstance(r.get('at_ns'),int)]
    cursor_stages={}
    for name in sorted({r['stage'] for r in selected if r['stage'].startswith('cursor_') and not r['stage'].endswith('_us')}):
        times=sorted(r['at_ns'] for r in selected if r['stage']==name)
        cursor_stages[name]={'events':len(times),'hz':len(times)/duration if duration>0 else None,
            'inter_update_ms':stats([(b-a)/1e6 for a,b in zip(times,times[1:])]),
            'last_update_age_ms':(end-times[-1])/1e6}
    marker_stages={}
    for name in ('capture_marker','decoded_marker'):
        observed=sorted((r for r in selected if r['stage']==name),key=lambda r:r['at_ns'])
        invalid=sum(r['stage']==name+'_invalid' for r in selected)
        if not observed and not invalid:continue
        transitions=[b['at_ns'] for a,b in zip(observed,observed[1:]) if a.get('bytes')!=b.get('bytes')]
        marker_stages[name]={'valid_samples':len(observed),'invalid_samples':invalid,
            'read_us':stats([r.get('bytes',0) for r in selected if r['stage']==name+'_read_us']),
            'content_changes':len(transitions),'content_change_hz':len(transitions)/duration if duration>0 else None,
            'repeated_samples':max(0,len(observed)-1-len(transitions)),
            'inter_content_change_ms':stats([(b-a)/1e6 for a,b in zip(transitions,transitions[1:])])}
        if name=='decoded_marker' and explicit_gpu:
            presented=by.get('presented',{})
            marker_stages[name]['drawable_matched_samples']=sum(str(r.get('frame_ns',0)) in presented for r in observed)
    return {'status':'measured' if selected else 'unavailable','samples':len(selected),'unique_stage_hz':stages,
            'event_counts':dict(collections.Counter(r['stage'] for r in selected)),
            'encode_ms':latency('submitted','encoded'),'decode_ms':latency('decoder_submitted','decoded'),
            'decode_to_viewer_selection_ms':latency('decoded','viewer_selected'),
            'viewer_selection_to_render_ms':latency('viewer_selected','render_started'),
            'render_to_gpu_complete_ms':latency('render_started',gpu_stage),
            'render_draw_us':stats([r.get('bytes',0) for r in selected if r['stage']=='render_draw_us']),
            'cursor_dispatch_wait_us':stats([r.get('bytes',0) for r in selected if r['stage']=='cursor_dispatch_wait_us']),
            'cursor_shape_query_us':stats([r.get('bytes',0) for r in selected if r['stage']=='cursor_shape_query_us']),
            'cursor_location_query_us':stats([r.get('bytes',0) for r in selected if r['stage']=='cursor_location_query_us']),
            'cursor_sample_interval_us':stats([r.get('bytes',0) for r in selected if r['stage']=='cursor_sample_interval_us']),
            'decoder_call_us':stats([r.get('bytes',0) for r in selected if r['stage']=='decoder_call_us']),
            'dtls_datagrams':{'sent':datagrams('dtls_datagram_sent'),'received':datagrams('dtls_datagram_received'),
                'scope':'Host socket timestamps and encrypted UDP payload lengths. Includes channel control and acknowledgments; no packet identity join or network latency claim.'},
            'helper_pipe':{name:stats([r.get('bytes',0) for r in selected if r['stage']==name])
                for name in ('pipe_video_wait_us','pipe_video_write_us','pipe_cursor_wait_us',
                    'pipe_cursor_write_us','pipe_control_wait_us','pipe_control_write_us')},
            'decode_to_gpu_complete_ms':latency('decoded',gpu_stage),
            'decode_to_drawable_present_ms':latency('decoded','presented') if explicit_gpu else None,
            'presentation_evidence':'drawable callback and separate GPU completion' if explicit_gpu else ('legacy presented stage; GPU completion only' if legacy_presentation else 'unavailable'),
            'window':{'start_ns':start,'end_ns':end,'trace_first_ns':min(timestamps) if timestamps else None,'trace_last_ns':max(timestamps) if timestamps else None},
            'reason':None if selected else 'Trace has no stage events in the measurement window; check helper telemetry and session freshness',
            'encoded_payload_mbps':sum(r.get('bytes',0) for r in selected if r['stage']=='encoded')*8/max(duration,1e-9)/1e6 if any(r['stage']=='encoded' for r in selected) else None,
            'configuration':config,'encoder_target_bitrate':encoder_bitrate_evidence(rows,start,end),'numeric_events':selected,'errors':sum(r.get('status',0)!=0 for r in selected),
            'cursor':{'status':'measured' if cursor_stages else 'unavailable','stages':cursor_stages,
                'source_sample_to_transport_ms':latency('cursor_position_sampled','cursor_position_transport_submitted'),
                'source_sample_to_submit_ms':latency('cursor_position_sampled','cursor_position_submitted'),
                'source_pipe_wait_ms':latency('cursor_position_pipe_queued','cursor_position_pipe_acquired'),
                'source_pipe_write_ms':latency('cursor_position_pipe_acquired','cursor_position_pipe_written'),
                # The reader can finish before the writer returns. Join from
                # pre-write enqueue, not writer-return, to avoid causal inversion.
                'source_pipe_to_helper_ms':latency('cursor_position_pipe_queued','cursor_position_helper_pipe_received'),
                'source_helper_to_runtime_ms':latency('cursor_position_helper_pipe_received','cursor_position_runtime_taken'),
                'source_runtime_to_transport_ms':latency('cursor_position_runtime_taken','cursor_position_transport_submitted'),
                'shape_query_to_transport_ms':latency('cursor_shape_query_completed','cursor_shape_transport_submitted'),
                'source_scope':'Source sample/pipe/helper/runtime joins use one Mac host clock and matching sequence or shape revision. Coalesced unsent updates are excluded. Transport submission is enqueue evidence, not USB delivery or optical presentation.',
                'capture_to_transport_ms':latency('cursor_position_submitted','cursor_position_transport_submitted'),
                'transport_to_runtime_ms':latency('cursor_position_transport_received','cursor_position_runtime_received'),
                'runtime_to_pipe_ms':latency('cursor_position_runtime_received','cursor_position_pipe_started'),
                'pipe_write_ms':latency('cursor_position_pipe_started','cursor_position_pipe_written'),
                'pipe_to_native_ms':latency('cursor_position_pipe_written','cursor_position_received'),
                'native_to_layer_ms':latency('cursor_position_received','cursor_position_layer_submit'),
                'shape_native_to_layer_ms':latency('cursor_shape_received','cursor_shape_layer_submit'),
                'shape_to_visible_layer_ms':latency('cursor_shape_received','cursor_shape_visible_layer_submit'),
                'shape_transport_to_visible_layer_ms':latency('cursor_shape_transport_received','cursor_shape_visible_layer_submit'),
                'shape_capture_to_transport_ms':latency('cursor_shape_submitted','cursor_shape_transport_submitted'),
                'shape_transport_to_runtime_ms':latency('cursor_shape_transport_received','cursor_shape_runtime_received'),
                'shape_runtime_to_pipe_ms':latency('cursor_shape_runtime_received','cursor_shape_pipe_started'),
                'shape_pipe_write_ms':latency('cursor_shape_pipe_started','cursor_shape_pipe_written'),
                'shape_pipe_to_native_ms':latency('cursor_shape_pipe_written','cursor_shape_received'),
                'delivery_latency_ms':None,'limits':['Host-local event cadence and age at window end only; not transport or physical cursor latency.','Missing cursor hooks remain unavailable.']},
            'synthetic_marker':{'status':'measured' if marker_stages else 'unavailable','stages':marker_stages,
                'limits':['Known fixture counter only; invalid samples can hide content transitions.','Content changes are consecutive valid counter differences, not inferred missing frames.','No cross-host timestamp join or physical visibility claim.']},
            'limits':['Host-local timestamps only; no cross-host frame join.','Layer submission and GPU completion are not physical scanout.','Hooks add observer overhead; use optical results for independent delivery evidence.']}

def read_numeric(path):
    result=[]
    if not Path(path).is_file():return result
    with Path(path).open() as f:
        for line in f:
            try:
                row=json.loads(line)
                if isinstance(row,dict) and set(row)<={'stage','at_ns','frame_ns','bytes','status','wall_ns','kind','key','value'}:result.append(row)
            except json.JSONDecodeError:pass # An active writer may have one partial tail record.
    return result

def transport_windows(path,rows,start,end):
    """Retain whole local transport windows that overlap the measured phase."""
    offsets=[r['wall_ns']-r['at_ns'] for r in rows
             if start<=r.get('at_ns',-1)<=end and type(r.get('wall_ns')) is int and type(r.get('at_ns')) is int]
    evidence={'status':'unavailable','windows':[],
        'scope':'Whole transport reporting windows overlapping this host phase; boundary windows include activity outside the phase. Clock mapping uses this host numeric events. Not cross-host latency or exact phase packet totals.'}
    if not offsets or not path or not Path(path).is_file():return evidence
    offset=int(statistics.median(offsets));first,last=start+offset,end+offset
    evidence['phase_wall_start_ns']=first;evidence['phase_wall_end_ns']=last
    with Path(path).open() as f:
        for line in f:
            if not line.startswith('Display transport timing: '):continue
            try:value=json.loads(line.split(': ',1)[1])
            except json.JSONDecodeError:continue
            if not isinstance(value,dict):continue
            finish=value.get('wall_end_ns');duration=value.get('window_ms')
            if type(finish) is not int or type(duration) is not int or not 0<duration<=60000:continue
            if finish>=first and finish-duration*1_000_000<=last:evidence['windows'].append(value)
    if evidence['windows']:evidence['status']='observed_overlapping_windows'
    return evidence

def snapshot(app,adapter,start,end):
    if adapter and hasattr(adapter,'trace'):
        path=adapter.trace(app)
        if path:return summarize(read_numeric(path),start,end)
    if app['product']=='extend':
        from bootstrap import ROOT
        state=ROOT/'.cache/extend-session.json'
        if state.exists():
            data=json.loads(state.read_text())
            if data.get('pid')==app['main_pid']:
                if data.get('diagnostics',{}).get('stage_timing') is False:
                    return {'status':'unavailable','reason':'Per-frame timing intentionally disabled for context-only launch'}
                rows=read_numeric(data['trace']);result=summarize(rows,start,end)
                result['transport_observation']=transport_windows(data.get('log'),rows,start,end)
                return result
    return {'status':'unavailable','reason':'Application was not launched with this benchmark session instrumentation'}
