"""Measured optical and resource summaries with explicit evidence limits."""
import json,math,statistics
from pathlib import Path

def stats(values):
    if not values:return None
    v=sorted(values)
    return {'count':len(v),'median':statistics.median(v),'p95':v[math.ceil(.95*len(v))-1],'min':v[0],'max':v[-1]}

def resources(rows):
    cpu=[];system=[];memory=[];overhead=[];last=None
    for row in rows:
        memory.append(sum(p['rss_bytes'] for p in row['processes']));overhead.append(row['collection_ms'])
        if last:
            dt=(row['at_ns']-last['at_ns'])/1e9
            previous={p['pid']:p['cpu_s'] for p in last['processes']}
            delta=sum(max(0,p['cpu_s']-previous[p['pid']]) for p in row['processes'] if p['pid'] in previous)
            if dt>0:
                cpu.append(delta/dt*100)
                system.append(max(0,row['system_process_cpu_s']-last['system_process_cpu_s'])/dt*100)
        last=row
    gpu_values=[d['Device Utilization %'] for r in rows for d in r.get('gpu',{}).get('devices',[]) if isinstance(d.get('Device Utilization %'),(int,float))]
    return {'cpu_percent_one_core':stats(cpu),'resident_bytes':stats(memory),'host_process_cpu_percent_one_core':stats(system),'sensor_collection_ms':stats(overhead),
            'scope':'product app and observed descendants; disappearing/new processes excluded from CPU deltas',
            'gpu':{'status':'measured' if gpu_values else 'unavailable','device_utilization_percent':stats(gpu_values),'scope':'whole-host GPU; not attributed per application'},
            'energy':{'status':'unavailable','reason':'No calibrated per-product energy sensor; CPU is not energy'}}

def optical(directory,source_index=0):
    p=Path(directory);summary=json.loads((p/'summary.json').read_text());meta=summary['video_metadata']
    rows=[json.loads(x) for x in (p/'optical-frames.jsonl').read_text().splitlines()]
    hz=summary['measured_sensor_hz'];playback=meta['nominal_playback_fps'];factor=playback/hz
    if not 20<=playback<=60 or hz<100:raise ValueError('Unverified camera timing scale')
    first=[{},{}];bounds=[{},{}];updates=[[],[]];valid=[0,0];last=[None,None]
    prior_readable=[None,None];missing=[False,False];unreadable_gaps=[[],[]]
    for row in rows:
        if len(row['counter_ids'])!=2:raise ValueError('Two physical timing patches required')
        t=row['playback_s']*factor
        for i,value in enumerate(row['counter_ids']):
            if value is None:
                missing[i]=True;continue
            valid[i]+=1
            if missing[i] and prior_readable[i] is not None:
                unreadable_gaps[i].append((t-prior_readable[i])*1000)
            missing[i]=False
            if value!=last[i]:
                if value in first[i]:raise ValueError('Counter wrapped or moved backwards in one short recording')
                first[i][value]=t;updates[i].append(t);last[i]=value
                bounds[i][value]=(prior_readable[i],t)
            prior_readable[i]=t
    if any(c<len(rows)*.5 for c in valid):raise ValueError('Insufficient optical coverage')
    a=source_index;b=1-a
    common=set(first[a])&set(first[b]);delays=[(first[b][v]-first[a][v])*1000 for v in sorted(common) if v not in {0,min(common,default=0)}]
    if len(delays)<20:raise ValueError('Insufficient matched physical transitions')
    intervals_of_delay=[]
    for value in sorted(common):
        if value in {0,min(common,default=0)}:continue
        s0,s1=bounds[a][value];r0,r1=bounds[b][value]
        if s0 is not None and r0 is not None:
            intervals_of_delay.append({'counter':value,'lower_ms':(r0-s1)*1000,'upper_ms':(r1-s0)*1000})
    if statistics.median(delays)<-1000/hz*2:raise ValueError('Source/receiver patches reversed or source reference invalid')
    # Zero is the fixture's stationary preparation counter. It is already
    # excluded from transition latency; including its hold in frame cadence
    # would report setup time as a one-second display stall.
    preparation_to_first_update=[]
    for i in range(2):
        positive=[t for value,t in first[i].items() if value!=0]
        preparation_to_first_update.append((positive[0]-first[i][0])*1000
            if positive and 0 in first[i] else None)
        updates[i]=positive
    intervals=[(y-x)*1000 for x,y in zip(updates[b],updates[b][1:])]
    duration=rows[-1]['playback_s']*factor-rows[0]['playback_s']*factor
    sensor=[json.loads(x) for x in (p/'sensor.jsonl').read_text().splitlines()]
    exposure=[r.get('exposure_ns')/1e6 for r in sensor if isinstance(r.get('exposure_ns'),(int,float))]
    skew=[r.get('rolling_shutter_skew_ns')/1e6 for r in sensor if isinstance(r.get('rolling_shutter_skew_ns'),(int,float))]
    return {'camera_exposure_ms':stats(exposure),'reported_rolling_shutter_skew_ms':stats(skew),'source_visible_updates':len(updates[a]),'receiver_visible_updates':len(updates[b]),'recorded_seconds':duration,
            'cadence_scope':'Positive workload counters only; stationary preparation counter zero excluded',
            'preparation_to_first_update_ms':{'source':preparation_to_first_update[a],'receiver':preparation_to_first_update[b]},
            'source_visible_hz':(len(updates[a])-1)/(updates[a][-1]-updates[a][0]) if len(updates[a])>1 else None,
            'receiver_visible_hz':(len(updates[b])-1)/(updates[a][-1]-updates[a][0]) if len(updates[a])>1 else None,
            'terminal_receiver_stall_ms':max(0,(updates[a][-1]-updates[b][-1])*1000) if updates[b] else None,
            'receiver_update_intervals_ms':stats(intervals),'intervals_over_50ms':sum(v>50 for v in intervals),
            'screen_to_screen_transition_ms':stats(delays),'camera_quantization_bound_ms':2000/hz,
            'transition_delay_bounds_ms':intervals_of_delay,
            'transition_delay_interval_width_ms':stats([r['upper_ms']-r['lower_ms'] for r in intervals_of_delay]),
            'unreadable_counter_intervals_ms':{'source':stats(unreadable_gaps[a]),'receiver':stats(unreadable_gaps[b])},
            'detected_update_rate_is_lower_bound':True,
            'counter_readable_coverage':{'source':valid[a]/len(rows),'receiver':valid[b]/len(rows)},
            'physical_latency_definition':'First readable occurrence of the same generated counter on receiver minus source, in one camera timeline',
            'limits':['Includes physical panels and scanout; not input-to-photon latency.','Camera rolling-shutter row bias is not removed.','Ambiguous transitions are excluded; remaining sample count and coverage are retained.','Detected update rate is a lower bound when counter frames are unreadable.','Per-transition bounds use the preceding readable counter and first readable new counter; they exclude exposure and rolling-shutter bias. Camera sample quantization alone does not bound unreadable intervals.'],
            'readable_coverage':[c/len(rows) for c in valid]}

def quality(image,patches):
    """Perspective-normalized optical comparison of generated detail strip."""
    import numpy as np
    from PIL import Image
    im=np.asarray(Image.open(image).convert('RGB'),dtype=float);gray=im@np.array([.2126,.7152,.0722])
    crops=[]
    for corners in patches:
        tl,tr,bl,br=np.asarray(corners)
        # Fixed synthetic strip in patch coordinates x=32..352,y=4..24.
        u=np.linspace(32/364,352/364,320);v=np.linspace(116/140,136/140,20)
        uu,vv=np.meshgrid(u,v)
        xy=(1-vv)[...,None]*((1-uu)[...,None]*tl+uu[...,None]*tr)+vv[...,None]*((1-uu)[...,None]*bl+uu[...,None]*br)
        x=np.rint(xy[...,0]).astype(int);y=np.rint(xy[...,1]).astype(int)
        if x.min()<0 or y.min()<0 or x.max()>=im.shape[1] or y.max()>=im.shape[0]:raise ValueError('Quality target outside camera view')
        crop=gray[y,x];lo,hi=np.percentile(crop,[5,95])
        if hi-lo<30:raise ValueError('Insufficient optical quality contrast')
        crops.append(np.clip((crop-lo)/(hi-lo),0,1))
    a,b=crops;rmse=float(np.sqrt(np.mean((a-b)**2)))
    ma,mb=a.mean(),b.mean();va,vb=a.var(),b.var();cov=((a-ma)*(b-mb)).mean()
    ssim=float(((2*ma*mb+.01**2)*(2*cov+.03**2))/((ma*ma+mb*mb+.01**2)*(va+vb+.03**2)))
    return {'normalized_optical_rmse':rmse,'global_optical_ssim':ssim,'target':'generated black/white detail strip; per-patch contrast normalized',
            'limits':['Includes phone optics, perspective registration and both physical panels.','Not codec-only image quality, full-screen SSIM, or a perceptual quality score.']}


def source_patch(image,patches):
    import numpy as np
    from PIL import Image
    pixels=np.asarray(Image.open(image).convert('RGB'))
    tags=[]
    for corners in patches:
        tl,tr,bl,br=np.asarray(corners);u=182/364;v=0.0
        xy=(1-v)*((1-u)*tl+u*tr)+v*((1-u)*bl+u*br)
        x,y=np.rint(xy).astype(int)
        tags.append(float(np.median(pixels[max(0,y-1):y+2,max(0,x-1):x+2,:])))
    if len(tags)!=2 or abs(tags[0]-tags[1])<80:raise ValueError('Cannot distinguish physical source reference marker')
    return 0 if tags[0]>tags[1] else 1


def network_delta(before,after):
    old={r['flow_sha256']:r for r in before['flows']};rates=[]
    for row in after['flows']:
        previous=old.get(row['flow_sha256'])
        if not previous:continue
        def num(value):
            try:return float(value or 0)
            except ValueError:return None
        values={}
        for key in ['bytes_in','bytes_out','re-tx']:
            a,b=num(previous.get(key)),num(row.get(key))
            values[key]=max(0,b-a) if a is not None and b is not None else None
        if row.get('selected_tuple_sha256')!=previous.get('selected_tuple_sha256'):
            row={**row,'matches_peer':None}
        rates.append({**row,'delta':values})
    active=[r for r in rates if (r['delta']['bytes_in'] or 0)+(r['delta']['bytes_out'] or 0)>1024]
    primary=max(active,key=lambda r:(r['delta']['bytes_in'] or 0)+(r['delta']['bytes_out'] or 0)) if active else None
    return {'flow_deltas':rates,'dominant_flow_matches_peer':primary['matches_peer'] if primary else None,
            'dominant_protocol':primary['protocol'] if primary else None,'dominant_interface':primary['interface'] if primary else None,
            'stable_flow_set':set(old)=={r['flow_sha256'] for r in after['flows']},'sensor_errors':before['errors']+after['errors'],
            'limits':['Includes all selected application socket bytes, not video-only payload.','Dominant peer flow is route evidence; it does not prove every data channel uses that route.']}

def classify_route(summary,evidence=None):
    """Classify observed dominant traffic without inferring relay from a public IP."""
    sides=[summary.get(role,{}) for role in ['source','receiver']]
    flags=[side.get('dominant_flow_matches_peer') for side in sides]
    result={'classification':'unknown','label':'Unknown','protocols':{role:summary.get(role,{}).get('dominant_protocol') for role in ['source','receiver']},
            'interfaces':{role:summary.get(role,{}).get('dominant_interface') for role in ['source','receiver']},
            'scope':'Dominant application socket traffic; not every data channel'}
    if any(side.get('sensor_errors') for side in sides) or any(flag is None for flag in flags):
        result['reason']='Missing or failed socket evidence on at least one host'
    elif all(flag is True for flag in flags):
        result.update(classification='direct_lan',label='Direct LAN',reason='Dominant flows match the opposite LAN peer on both hosts')
    elif all(flag is False for flag in flags):
        result.update(classification='non_lan',label='Non-LAN (relay unconfirmed)',reason='Neither dominant flow matches the LAN peer; public-address P2P is not excluded')
        if evidence and evidence.get('kind')=='both_hosts_same_relay_endpoint':
            matched=True
            for role,side in zip(['source','receiver'],sides):
                flows=side.get('flow_deltas',[])
                primary=max(flows,key=lambda f:(f['delta']['bytes_in'] or 0)+(f['delta']['bytes_out'] or 0),default={})
                if not primary or primary.get('flow_sha256')!=evidence.get('dominant_flow_sha256',{}).get(role):matched=False
            if matched:result.update(classification='relay',label='Relay (confirmed)',reason='Both recorded dominant flows match separately verified sockets to the same relay endpoint',confirmation=evidence)
    else:
        result.update(classification='mixed',label='Mixed route evidence',reason='Source and receiver peer-match evidence disagree')
    return result
