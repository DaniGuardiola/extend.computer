"""Numeric stage evidence. Frame IDs are joined only within one host clock."""
import json
from pathlib import Path
from metrics import stats

def summarize(rows,start,end):
    selected=[r for r in rows if start<=r.get('at_ns',-1)<=end and 'stage' in r]
    duration=(end-start)/1e9
    by={};config={}
    for r in rows:
        if r.get('kind')=='configuration' and r.get('at_ns',0)<=end:config[r['key']]=r['value']
    for r in selected:
        key=str(r.get('frame_ns',0));stage=r['stage'];by.setdefault(stage,{})
        # Zero is not a valid frame ID for a layer update.
        if key!='0':by[stage][key]=r
    def latency(before,after):
        a,b=by.get(before,{}),by.get(after,{})
        return stats([(b[k]['at_ns']-a[k]['at_ns'])/1e6 for k in a.keys()&b.keys() if b[k]['at_ns']>=a[k]['at_ns']])
    stages={s:len(v)/duration for s,v in by.items()} if duration>0 else {}
    layer=sum(r['stage']=='layer_submit' for r in selected)
    if layer:stages['layer_submit']=layer/duration
    return {'status':'measured' if selected else 'unavailable','samples':len(selected),'unique_stage_hz':stages,
            'encode_ms':latency('submitted','encoded'),'decode_ms':latency('decoder_submitted','decoded'),
            'decode_to_gpu_complete_ms':latency('decoded','presented'),'encoded_payload_mbps':sum(r.get('bytes',0) for r in selected if r['stage']=='encoded')*8/max(duration,1e-9)/1e6 if any(r['stage']=='encoded' for r in selected) else None,
            'configuration':config,'numeric_events':selected,'errors':sum(r.get('status',0)!=0 for r in selected),
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

def snapshot(app,adapter,start,end):
    if adapter and hasattr(adapter,'trace'):
        path=adapter.trace(app)
        if path:return summarize(read_numeric(path),start,end)
    if app['product']=='extend':
        from bootstrap import ROOT
        state=ROOT/'.cache/extend-session.json'
        if state.exists():
            data=json.loads(state.read_text())
            if data.get('pid')==app['main_pid']:return summarize(read_numeric(data['trace']),start,end)
    return {'status':'unavailable','reason':'Application was not launched with this benchmark session instrumentation'}
