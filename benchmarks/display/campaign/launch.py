"""Launch the selected Extend app with a fresh numeric timing session."""
import argparse,json,os,signal,subprocess,time,uuid
from pathlib import Path
from context import processes
from bootstrap import ROOT

def main():
    p=argparse.ArgumentParser();p.add_argument('--app',type=Path,default=Path('/Applications/extend.computer.app'));a=p.parse_args()
    app=a.app.resolve();exe=app/'Contents/MacOS/extend.computer'
    if not exe.exists():raise RuntimeError('Extend app missing; supply --app PATH')
    selected=[r for r in processes() if r['path']==str(exe)]
    for r in selected:os.kill(r['pid'],signal.SIGTERM)
    end=time.monotonic()+10
    while selected and time.monotonic()<end:
        live={r['pid'] for r in processes()};selected=[r for r in selected if r['pid'] in live]
        if selected:time.sleep(.1)
    if selected:raise RuntimeError('Existing Extend app did not quit; close it normally')
    out=ROOT/'.cache';out.mkdir(exist_ok=True);trace=out/('extend-'+uuid.uuid4().hex+'.jsonl')
    log=(out/'extend-launch.log').open('a')
    process=subprocess.Popen([str(exe)],env={**os.environ,'EXTEND_DISPLAY_METRICS':str(trace)},stdout=log,stderr=log,start_new_session=True);log.close()
    state=out/'extend-session.json';state.write_text(json.dumps({'pid':process.pid,'trace':str(trace),'app':str(app)}));state.chmod(0o600)
    print('Extend launched with numeric stage instrumentation. Establish the display connection normally.')
if __name__=='__main__':main()
