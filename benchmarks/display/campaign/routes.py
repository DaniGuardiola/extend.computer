"""Add route classifications to saved evidence without changing raw measurements."""
import argparse,datetime,json,shutil,uuid
from pathlib import Path
from metrics import classify_route
from run import artifact_inventory,digest,render,write

def reclassify(root,evidence=None):
    root=Path(root).resolve()
    report=json.loads((root/'report.json').read_text())
    manifest=json.loads((root/'manifest.json').read_text())
    if report['status'] not in {'failed','complete_with_declared_limits','setup_check_passed'}:raise RuntimeError('Stop the campaign before reclassifying')
    for name in ['report.json']:
        if manifest['artifacts'].get(name)!=digest(root/name):raise RuntimeError('Saved report integrity check failed')
    backup=root/'route-history'/uuid.uuid4().hex;backup.mkdir(parents=True)
    for name in ['report.json','report.html','manifest.json']:shutil.copy2(root/name,backup/name)
    for phase in report['phases']:phase['route']=classify_route(phase.get('network_summary',{}),evidence)
    report.setdefault('postprocessing',[]).append({'operation':'classify-routes','at_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'previous_report':str(backup.relative_to(root)/'report.json')})
    write(root/'report.json',report);render(root,report)
    manifest['artifacts']=artifact_inventory(root);write(root/'manifest.json',manifest)
    return report

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('run',type=Path);parser.add_argument('--evidence',type=Path,help='Separately verified two-host relay socket evidence')
    args=parser.parse_args()
    evidence=json.loads(args.evidence.read_text()) if args.evidence else None
    report=reclassify(args.run,evidence)
    counts={}
    for phase in report['phases']:
        label=phase['route']['label'];counts[label]=counts.get(label,0)+1
    print('Route classifications: '+json.dumps(counts))
    print('Report: '+str(args.run/'report.html'))
