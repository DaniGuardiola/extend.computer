"""Compare campaign reports only when their observed contexts are compatible."""
import argparse,json
from pathlib import Path
from metrics import stats


def comparable(a,b):
    problems=[]
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
        if any(p.get('network_summary',{}).get(role,{}).get('dominant_flow_matches_peer') is not True for p in report['phases'] for role in ['source','receiver']):problems.append(report['product']+' direct peer route not confirmed for every scene')
    if any(r['status']!='complete_with_declared_limits' for r in [a,b]):problems.append('Incomplete campaign')
    return problems

def compare(a,b):
    reasons=comparable(a,b);rows=[]
    for scene in ['static','scroll','panel','motion','recovery']:
        for metric,path in [('receiver_visible_hz',['optical','receiver_visible_hz']),('screen_to_screen_ms',['optical','screen_to_screen_transition_ms','median']),('optical_ssim',['quality','global_optical_ssim']),('source_cpu',['resources','source','cpu_percent_one_core','median']),('receiver_cpu',['resources','receiver','cpu_percent_one_core','median'])]:
            values=[]
            for report in [a,b]:
                samples=[]
                for p in report['phases']:
                    if p['scene']!=scene:continue
                    value=p
                    for key in path:value=value.get(key) if isinstance(value,dict) else None
                    if isinstance(value,(int,float)):samples.append(value)
                values.append(stats(samples))
            rows.append({'scene':scene,'metric':metric,'left':values[0],'right':values[1]})
    return {'schema_version':1,'left':a['product'],'right':b['product'],'comparable_context':not reasons,'reasons':reasons,'metrics':rows,
            'limits':['Same physical camera placement and stable transport still require reviewing run evidence.','No overall winner inferred from frame rate alone.','Optical SSIM includes physical panels and camera; do not compare to codec-only quality scores.']}

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
