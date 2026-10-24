"""Physical state-reference to visible cursor transitions, in one camera clock."""
import json,math,statistics
from pathlib import Path
from metrics import stats

def lane(scores):
    if not isinstance(scores,list) or len(scores)!=4:return None
    if any(type(v) not in (int,float) or not math.isfinite(v) or not 0<=v<=1 for v in scores):return None
    winners=[i for i,v in enumerate(scores) if v>=5/9]
    if len(winners)!=1:return None
    winner=winners[0]
    return winner if all(v<=2/9 for i,v in enumerate(scores) if i!=winner) else None

def event_value(row,index):
    try:
        events=row['cursor_events'][index];patch=row['patches'][index]
        if len(events)!=4 or len(patch)!=2 or any(len(p)!=16 for p in patch):return None
        borders=[]
        for event in events:
            samples=[event]+event.get('candidates',[])
            if len(samples)>26:return None
            scores=[sample['border_score'] for sample in samples]
            if any(type(v) not in (int,float) or not math.isfinite(v) or not 0<=v<=1 for v in scores):return None
            borders.append(max(scores))
        if any(type(v) not in (int,float) or not math.isfinite(v) or not 0<=v<=1 for v in borders):return None
        winners=[i for i,v in enumerate(borders) if v>=4/6]
        if len(winners)!=1:return None
        winner=winners[0]
        if any(v>1/6 for i,v in enumerate(borders) if i!=winner):return None
        values=sorted(v for p in patch for v in p)
        if any(type(v) not in (int,float) or not math.isfinite(v) or not 0<=v<=255 for v in values):return None
        black=statistics.median(values[:8]);white=statistics.median(values[-8:])
        if white-black<30:return None
        def decode(candidate):
            if candidate['border_score']<4/6:return None
            cells=candidate['cells']
            if len(cells)!=12:return None
            word=0
            for cell in cells:
                if type(cell) not in (int,float) or not math.isfinite(cell) or not 0<=cell<=255:return None
                value=(cell-black)/(white-black)
                if .25<value<.75:return None
                word=(word<<1)|int(value>=.75)
            event=word>>4;crc=0
            for bit in reversed(range(12)):
                feedback=((crc>>3)&1)^((event>>bit)&1);crc=(crc<<1)&15
                if feedback:crc^=3
            return event if word&15==crc and event%4==winner else None
        candidates=[events[winner]]+events[winner].get('candidates',[])
        if len(candidates)>26:return None
        for candidate in candidates[1:]:
            offset=candidate.get('sample_offset_camera_px')
            if not isinstance(offset,list) or len(offset)!=2 or any(type(v)!=int or abs(v)>2 for v in offset):return None
        decoded={v for candidate in candidates if (v:=decode(candidate)) is not None}
        return next(iter(decoded)) if len(decoded)==1 else None
    except (KeyError,IndexError,TypeError):return None

def summarize_events(rows,factor,hz,source_index,preparation_event_id=None):
    if source_index not in (0,1) or not 100<=hz<=1000 or not 0<factor<=1:
        raise ValueError('Unverified coded cursor timing context')
    if preparation_event_id not in (None,0):raise ValueError('Unsupported cursor preparation identity')
    prepared=[preparation_event_id is None]*2
    prior=[set(),set()];prepared_times=[None,None]
    transitions=[[],[]];valid=[0,0];candidate=[None,None];last=[None,None];previous=-1
    for row in rows:
        t=row['playback_s']*factor
        if not math.isfinite(t) or t<=previous:raise ValueError('Non-increasing cursor camera timestamps')
        previous=t
        for i in (0,1):
            value=event_value(row,i)
            if value is None:candidate[i]=None;continue
            if not prepared[i]:
                if value!=preparation_event_id:
                    prior[i].add(value);candidate[i]=None;continue
                if candidate[i] is None or candidate[i][0]!=value:
                    candidate[i]=(value,t);continue
                prepared[i]=True;prepared_times[i]=candidate[i][1]
            valid[i]+=1
            if candidate[i] is not None and candidate[i][0]==value:
                if last[i]!=value:
                    if last[i] is not None and value<=last[i]:raise ValueError('Coded cursor event identity regressed or wrapped')
                    transitions[i].append((candidate[i][1],value));last[i]=value
            else:candidate[i]=(value,t)
    if not all(prepared):raise ValueError('Cursor preparation identity was not verified on both screens')
    if not rows or min(valid)<len(rows)*.5:raise ValueError('Insufficient coded cursor coverage')
    if preparation_event_id is not None:
        if preparation_event_id!=0:raise ValueError('Unsupported cursor preparation identity')
        transitions=[[entry for entry in values if entry[1]!=preparation_event_id] for values in transitions]
    source=transitions[source_index];receiver=transitions[1-source_index]
    if len(source)<12:raise ValueError('Insufficient unique source cursor events')
    if any(b[1]!=a[1]+1 or not .2<=b[0]-a[0]<=.3 for a,b in zip(source,source[1:])):
        raise ValueError('Source coded cursor cadence or event continuity invalid')
    refs={value:t for t,value in source};used=set();delays=[]
    for t,value in receiver[1:-1]:
        if value not in refs:raise ValueError('Receiver cursor event absent from physical source reference')
        delay=t-refs[value]
        if not -.125<=delay<=.5:raise ValueError('Coded cursor signed delay exceeds matching bounds')
        used.add(value);delays.append(delay*1000)
    eligible={v for _,v in source[2:-2]}
    coverage=len(used&eligible)/len(eligible) if eligible else 0
    if not delays or coverage<.95:
        missing=sorted(eligible-used)
        raise ValueError(f'Coded cursor event loss exceeds acceptance bound: {coverage:.1%} matched; missing confirmed receiver IDs {missing}')
    return {'status':'measured','pattern':'coded-four-lane-v2','state_reference_to_cursor_ms':stats(delays),
        'source_transitions':len(source),'receiver_transitions':len(receiver),
        'preparation_observation':{'preexisting_ids_left_to_right':[sorted(v) for v in prior],'established_at_camera_s_left_to_right':prepared_times} if preparation_event_id is not None else None,
        'matched_transition_coverage':coverage,'readable_coverage':[v/len(rows) for v in valid],
        'negative_delay_count':sum(v<0 for v in delays),'unique_event_ids_verified':True,
        'camera_quantization_bound_ms':2000/hz,'matching_signed_bounds_ms':[-125,500],
        'definition':'Receiver cursor shape-and-position event appearance minus the same uniquely coded physical source reference event.',
        'limits':['Software cursor shape-and-position changes at 4 Hz; not hardware input-to-photon or continuous movement cadence.',
            'Signed delays can be negative because source reference panel displays after the software warp; negative does not mean negative network latency.',
            'Eight-bit identities do not wrap within an accepted scene; CRC and position must agree.',
            'Reference panel scanout, camera exposure and rolling-shutter bias remain included.']}

def summarize(rows,factor,hz,source_index):
    if source_index not in (0,1) or not 100<=hz<=1000 or not 0<factor<=1:
        raise ValueError('Unverified optical cursor timing context')
    transitions=[[],[]];valid=[0,0];candidate=[None,None];last=[None,None]
    last_time=-1
    for row in rows:
        t=row['playback_s']*factor
        if not math.isfinite(t) or t<=last_time:raise ValueError('Non-increasing cursor camera timestamps')
        last_time=t
        scores=row.get('cursor_scores')
        for i in (0,1):
            value=lane(scores[i]) if isinstance(scores,list) and len(scores)==2 else None
            if value is None:candidate[i]=None;continue
            valid[i]+=1
            if candidate[i] is not None and candidate[i][0]==value:
                if last[i]!=value:
                    transitions[i].append((candidate[i][1],value));last[i]=value
            else:candidate[i]=(value,t)
    if not rows or min(valid)<len(rows)*.5:raise ValueError('Insufficient readable physical cursor coverage')
    source=transitions[source_index];receiver=transitions[1-source_index]
    if len(source)<12 or {v for _,v in source}!={0,1,2,3}:raise ValueError('Insufficient source cursor transitions')
    gaps=[b[0]-a[0] for a,b in zip(source,source[1:])]
    if any(gap>.4 for gap in gaps) or not .2<=statistics.median(gaps)<=.3:
        raise ValueError('Source cursor step timing invalid')
    # Labels repeat once per second. A bounded half-second matching window
    # prevents pairing a stalled cursor with the next cycle of the same label.
    # Out-of-range transitions invalidate the result instead of being discarded.
    bound=2/hz;used=set();delays=[];unmatched=[];last_match=-1
    if any(b[0]-a[0]>.5+bound for a,b in zip(receiver,receiver[1:])):
        raise ValueError('Physical cursor transition gap exceeds 500 ms')
    for t,value in receiver[1:]:
        candidates=[j for j,(s,v) in enumerate(source) if j>last_match and v==value and -bound<=t-s<=.5]
        if len(candidates)!=1:unmatched.append([t,value]);continue
        j=candidates[0];used.add(j);last_match=j;delays.append((t-source[j][0])*1000)
    eligible=set(range(1,len(source)-1))
    coverage=len(used&eligible)/len(eligible)
    if unmatched or coverage<.9:raise ValueError('Physical cursor transition loss, ambiguity or delay beyond 500 ms')
    return {'status':'measured','state_reference_to_cursor_ms':stats(delays),
        'source_transitions':len(source),'receiver_transitions':len(receiver),
        'matched_transition_coverage':coverage,'readable_coverage':[v/len(rows) for v in valid],
        'unmatched_interior_source_transitions':len(eligible-used),
        'unmatched_receiver_transitions':len(unmatched),
        'receiver_transition_gap_ms':stats([(b[0]-a[0])*1000 for a,b in zip(receiver,receiver[1:])]),
        'camera_quantization_bound_ms':bound*1000,'matching_bound_ms':500,
        'definition':'First readable receiver pointer lane minus the matching physical source state-reference lane, in one camera timeline.',
        'limits':['Software pointer warp, not physical mouse input-to-photon latency.',
            'Source reference is drawn after a successful warp; its panel/scanout timing is included.',
            'Ambiguous, lost or over-bound transitions fail acceptance; delays are not silently filtered.',
            'Four discrete positions at 4 Hz; continuous cursor cadence and other shapes remain separate.',
            'Camera exposure and rolling-shutter row bias remain in the observation.']}

def optical_cursor(directory,source_index):
    root=Path(directory);summary=json.loads((root/'summary.json').read_text())
    hz=summary['measured_sensor_hz'];playback=summary['video_metadata']['nominal_playback_fps']
    if not 20<=playback<=60:raise ValueError('Unverified cursor playback timing')
    source=json.loads((root.parent/'source.json').read_text())
    fixture=source.get('fixture',source).get('cursor',{})
    pattern=fixture.get('pattern')
    frames=root/'optical-frames.jsonl'
    prepared=root/'cursor-optical-frames.jsonl'
    if pattern in {'coded-four-lane-v4','coded-four-lane-v5'} and prepared.exists():
        frames=prepared
    rows=[json.loads(line) for line in frames.read_text().splitlines()]
    if pattern in {'coded-four-lane-v2','coded-four-lane-v3','coded-four-lane-v4','coded-four-lane-v5'}:
        if pattern in {'coded-four-lane-v3','coded-four-lane-v4','coded-four-lane-v5'} and (fixture.get('center_y_logical')!=148 or fixture.get('reference_origin_y_logical')!=132):
            raise ValueError('Unsupported coded cursor placement')
        if fixture.get('shape_logical_size')!=[48,32]:
            raise ValueError('Unsupported coded cursor fixture geometry')
        observed=fixture.get('observed_events')
        if fixture.get('errors')!=0 or fixture.get('position_restored') is not True or not isinstance(observed,list) or len(observed)<12:
            raise ValueError('Source coded cursor raster verification missing or failed')
        steps=fixture.get('steps')
        if type(steps)!=int or not len(observed)<=steps<=len(observed)+1:
            raise ValueError('Source coded cursor verification coverage invalid')
        previous=-1
        for event in observed:
            identity=event.get('event');expected=event.get('expected_raster_sha256');actual=event.get('observed_raster_sha256')
            if type(identity)!=int or not 0<=identity<256 or identity!=previous+1 or event.get('matches') is not True or not isinstance(expected,str) or len(expected)!=64 or any(c not in '0123456789abcdef' for c in expected) or actual!=expected or event.get('observed_size')!=[48,32] or event.get('observed_hotspot')!=[24,16]:
                raise ValueError('Source coded cursor raster verification mismatch')
            previous=identity
        preparation=None
        if pattern in {'coded-four-lane-v4','coded-four-lane-v5'}:
            if fixture.get('preparation_event_id')!=0 or fixture.get('first_measured_event_id')!=1 or fixture.get('preparation_s')!=.5:
                raise ValueError('Unverified cursor preparation phase')
            if pattern=='coded-four-lane-v5' and (fixture.get('reference_center_y_logical')!=164 or fixture.get('reference_shape_logical_size')!=[96,64]):raise ValueError('Unsupported enlarged cursor reference')
            preparation=0
        result=summarize_events(rows,playback/hz,hz,source_index,preparation)
        if preparation is not None:result['excluded_preparation_event_id']=preparation
        result['pattern']=pattern
        return result
    return summarize(rows,playback/hz,hz,source_index)
