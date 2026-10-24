"""Separate, acknowledged Extend development diagnostics for display settings."""
import argparse,datetime,html,json,os,subprocess,sys
from pathlib import Path
from bootstrap import ROOT
from client import Client
from progress import Progress
from settings import configuration

MODES=('responsive','balanced','detail','30fps','120fps','native','compact')

def mode_plan(profile,modes):
    if not modes or len(set(modes))!=len(modes):raise ValueError('Select distinct settings modes')
    result=[]
    for name in modes:
        if name not in MODES:raise ValueError('Unknown settings mode')
        selected=profile.rsplit('@',1)[0]+'@1' if name=='native' else '1280x800@2' if name=='compact' else profile
        fps=30 if name=='30fps' else 120 if name=='120fps' else 60
        quality=name if name in {'responsive','balanced','detail'} else 'balanced'
        result.append({'mode':name,'profile':selected,'configuration':configuration(selected,fps,quality)})
    return result

def apply(client,config):
    reply=client.request('/display-settings',{'product':'extend','config':config})
    if reply.get('ok') is not True or reply.get('configuration')!=config:
        raise RuntimeError('Source did not acknowledge the requested settings on both devices')

def validate_recording(report,config,scenes):
    phases=report.get('phases',[])
    if len(phases)!=len(scenes) or [p.get('scene') for p in phases]!=scenes:
        raise RuntimeError('Settings campaign did not measure every requested scene')
    for phase in phases:
        if phase.get('status')!='measured':raise RuntimeError('Settings campaign contains an unaccepted scene')
        observed=phase.get('display_configuration',{})
        for key in ('width','height','fps','bitrate','retina','codec','separate_cursor'):
            if key not in observed or type(observed[key]) is not type(config[key]) or observed[key]!=config[key]:
                raise RuntimeError('Recorded display configuration differs from requested settings: '+key)

def save(root,result):
    path=root/'sweep.json';temporary=path.with_suffix('.tmp')
    temporary.write_text(json.dumps(result,indent=2));temporary.replace(path)
    rows=[]
    for item in result['runs']:
        report=root/item['folder']/'report.json'
        measured=[phase for phase in json.loads(report.read_text()).get('phases',[]) if phase.get('status')=='measured'] if report.exists() else []
        for phase in measured or [{}]:
            optical=phase.get('optical') or {};delay=(optical.get('screen_to_screen_transition_ms') or {}).get('median')
            number=lambda value:'unavailable' if value is None else f'{value:.2f}'
            config=item['configuration'];video_delay=optical.get('screen_to_screen_transition_ms') or {}
            cursor_delay=(phase.get('physical_cursor') or {}).get('state_reference_to_cursor_ms') or {}
            source=(phase.get('stages') or {}).get('source') or {}
            cells=[item['mode'],item['profile'],config['fps'],config['bitrate']/1_000_000,
                   phase.get('scene',''),phase.get('route',{}).get('label','Unknown'),
                   number(source.get('encoded_payload_mbps')),number(optical.get('receiver_visible_hz')),
                   number(delay),number(video_delay.get('p95')),number(cursor_delay.get('median')),
                   number(cursor_delay.get('p95')),item['status']]
            rows.append('<tr>'+''.join('<td>'+html.escape(str(x))+'</td>' for x in cells)+
                '<td><a href="'+item['folder']+'/report.html">Evidence</a></td></tr>')
    body='<meta charset="utf-8"><title>Display settings sweep</title><style>body{font:16px system-ui;max-width:1100px;margin:40px auto;padding:20px}td,th{text-align:left;padding:10px;border-bottom:1px solid #ddd}table{border-collapse:collapse}</style>'
    body+='<h1>Display settings sweep</h1><p>'+html.escape(result['status'])+'</p><p>Separate development samples. Review each report’s context and optical validity; this is not a full comparison or a stable preset ranking.</p>'
    body+='<table><tr>'+''.join('<th>'+x+'</th>' for x in ['Mode','Geometry','Requested FPS','Ceiling Mbps','Scene','Route','Encoded Mbps','Visible updates/s','Video median ms','Video p95 ms','Cursor median ms','Cursor p95 ms','Status','Report'])+'</tr>'+''.join(rows)+'</table>'
    body+='<p>Encoded Mbps measures video payload, excluding transport overhead. Cursor timing measures software state-reference to visible cursor, not physical mouse input latency. Different geometries and frame rates are separate conditions.</p>'
    body+='<p>Baseline restoration: '+html.escape(result.get('restoration','pending'))+'</p>'
    (root/'report.html').write_text(body)

def execute(client,root,plan,baseline,scenes,seconds,pairing,runner=subprocess.run):
    result={'schema_version':1,'measurement_scope':'development-only','status':'running','runs':[],
            'restore_configuration':baseline,'seconds':seconds,'scenes':scenes,'created_utc':datetime.datetime.now(datetime.timezone.utc).isoformat()}
    root.mkdir(parents=True,exist_ok=False,mode=0o700)
    progress=Progress();progress.heading('extend','settings sweep');save(root,result)
    try:
        for index,item in enumerate(plan,1):
            row={**item,'folder':f'{index:02}-{item["mode"]}','status':'applying'};result['runs'].append(row);save(root,result)
            progress.scene(index,len(plan),item['mode'],1,1)
            apply(client,item['configuration']);row['status']='recording';save(root,result)
            argv=[sys.executable,str(ROOT/'bench.py'),'campaign','--product','extend','--pairing',str(pairing),
                  '--display-profile',item['profile'],'--seconds',str(seconds),'--repetitions','1','--output',str(root/row['folder']),'--no-open']
            for scene in scenes:argv+=['--scene',scene]
            runner(argv,check=True,env={**os.environ,'EXTEND_DISPLAY_SETTINGS_SWEEP':'1'})
            report=root/row['folder']/'report.json'
            if not report.exists():raise RuntimeError('Settings campaign report is missing')
            validate_recording(json.loads(report.read_text()),item['configuration'],scenes)
            row['status']='complete';save(root,result)
        result['status']='complete'
    except BaseException as error:
        result['status']='failed';result['error']=str(error) or type(error).__name__
        if result['runs']:result['runs'][-1]['status']='failed'
        raise
    finally:
        try:apply(client,baseline);result['restoration']='acknowledged on both devices'
        except Exception as error:
            result['restoration']='failed: '+str(error)
            if result['status']=='complete':result['status']='failed'
        save(root,result)
    if result['status']!='complete':raise RuntimeError(result['restoration'])
    return result

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--display-profile',required=True)
    parser.add_argument('--mode',action='append',choices=MODES,required=True)
    parser.add_argument('--scene',action='append',choices=['static','scroll','panel','motion','recovery','cursor','cursor-optical','cursor-sweep'])
    parser.add_argument('--seconds',type=int,default=10)
    parser.add_argument('--pairing',type=Path,default=ROOT/'.cache/campaign/source-pairing.json')
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--no-open',action='store_true')
    args=parser.parse_args()
    try:
        plan=mode_plan(args.display_profile,args.mode);baseline=configuration(args.display_profile)
        scenes=args.scene or ['motion','scroll']
        if len(set(scenes))!=len(scenes) or not 5<=args.seconds<=54:raise ValueError('Select distinct scenes and 5..54 seconds')
    except ValueError as error:parser.error(str(error))
    root=args.output.expanduser().resolve()
    try:execute(Client(args.pairing),root,plan,baseline,scenes,args.seconds,args.pairing)
    finally:
        if (root/'report.html').exists() and not args.no_open:subprocess.run(['open',str(root/'report.html')],check=False)
    print('Settings sweep completed successfully. Results: '+str(root/'report.html'))
if __name__=='__main__':main()
