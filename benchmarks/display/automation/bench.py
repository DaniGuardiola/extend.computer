#!/usr/bin/env python3
"""Set up and run the two-Mac optical benchmark without an agent."""
import argparse, getpass, hashlib, json, os, platform, re, shutil, subprocess, sys
from pathlib import Path
from bootstrap import ROOT, adb_at, build_swift, java_at, sdk_at, venv_at
import importlib.util

def product_adapter():
    path=os.environ.get('DISPLAY_BENCH_ADAPTER')
    if not path:return None
    spec=importlib.util.spec_from_file_location('display_product_adapter',path)
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    return module
CONFIG=ROOT/'hosts.local.json'

def load():return json.loads(CONFIG.read_text()) if CONFIG.exists() else {}
def save(config):
    CONFIG.write_text(json.dumps(config,indent=2));CONFIG.chmod(0o600)
def run(argv,**kwargs):return subprocess.run([str(x) for x in argv],check=True,**kwargs)
def read(argv):return run(argv,capture_output=True,text=True,timeout=15).stdout.strip()
def repository(value,config):
    path=Path(value or config.get('extend_repo') or ROOT.parents[2]).expanduser().resolve()
    if not (path/'benchmarks/display/optical/decode.py').is_file():raise RuntimeError('Extend benchmark core missing; pass --extend-repo PATH')
    return path

def select_phone(adb,explicit=None,remembered=None):
    listing=read([adb,'devices'])
    transports=[line.split()[0] for line in listing.splitlines()[1:] if len(line.split())>=2 and line.split()[1]=='device']
    if explicit:
        if explicit not in transports:
            if ':' in explicit:run([adb,'connect',explicit])
            if read([adb,'-s',explicit,'get-state'])!='device':raise RuntimeError('Selected phone is not connected')
        return explicit
    devices={}
    for transport in transports:
        serial=read([adb,'-s',transport,'shell','getprop','ro.serialno'])
        identity=hashlib.sha256((serial or transport).encode()).hexdigest()
        devices.setdefault(identity,[]).append(transport)
    if remembered:
        if remembered not in devices:raise RuntimeError('Configured phone is offline. Enable wireless debugging or pass --phone ENDPOINT')
        devices={remembered:devices[remembered]}
    if len(devices)!=1:raise RuntimeError('Connect one phone, or select it explicitly with --phone SERIAL')
    candidates=next(iter(devices.values()))
    return sorted(candidates,key=lambda name:('._adb-tls-connect.' not in name,name))[0]

def phone_identity(adb,serial):
    actual=read([adb,'-s',serial,'shell','getprop','ro.serialno'])
    return hashlib.sha256((actual or serial).encode()).hexdigest()

def doctor(config,extend,source=False):
    if platform.system()!='Darwin':raise RuntimeError('This live optical suite currently requires macOS on both hosts')
    print('Extend core: ready')
    print('Swift: '+read(['xcrun','--find','swiftc']))
    if source:return
    python=ROOT/'.venv/bin/python3'
    if not python.exists():raise RuntimeError('Run ./benchmark/display setup on this Mac first')
    read([python,'-c','import numpy, PIL'])
    adb=adb_at(config.get('adb'));serial=select_phone(adb,config.get('phone_override'),config.get('phone_identity'))
    if not read([adb,'-s',serial,'shell','pm','path','computer.extend.benchmarkcamera']):raise RuntimeError('Benchmark Camera missing; run setup --phone SERIAL')
    print('Phone: connected; Benchmark Camera installed')
    print('Dependencies: ready. Run check to verify unlocked camera capture.')

def main():
    if sys.version_info<(3,12):raise RuntimeError('Python 3.12+ required; install it and rerun with BENCHMARK_PYTHON=python3.12')
    if len(sys.argv)>1 and sys.argv[1]=='pair-source':
        import base64
        value=getpass.getpass('Paste source pairing code (not saved in shell history): ')
        data=json.loads(base64.urlsafe_b64decode(value))
        if set(data)!={'url','token','certificate'} or not data['url'].startswith('https://') or len(data['token'])<32:raise RuntimeError('Invalid pairing code')
        dest=ROOT/'.cache/campaign/source-pairing.json';dest.parent.mkdir(parents=True,exist_ok=True)
        dest.write_text(json.dumps(data));dest.chmod(0o600)
        print('Benchmark source paired. Run campaign --product PRODUCT --check after connecting.')
        return
    if len(sys.argv)>1 and sys.argv[1] in {'host','campaign','compare','launch'}:
        entry=sys.argv.pop(1)
        if entry=='campaign' and Path(sys.prefix).resolve()!=(ROOT/'.venv').resolve():
            python=ROOT/'.venv/bin/python3'
            if not python.exists():raise RuntimeError('Run setup on the receiving Mac first')
            os.execv(str(python),[str(python),sys.argv[0],entry,*sys.argv[1:]])
        extend=repository(None,load());tools=build_swift(extend)
        import runpy
        folder=extend/'benchmarks/display/campaign';sys.path.insert(0,str(folder))
        if entry=='host':
            import host as host_service
            host_service.main(tools/'campaign-workload',product_adapter(),ROOT/'.cache/campaign')
        else:runpy.run_path(str(folder/({'campaign':'run.py','compare':'compare.py','launch':'launch.py'}[entry])),run_name='__main__')
        return
    p=argparse.ArgumentParser(description=__doc__)
    sub=p.add_subparsers(dest='command',required=True)
    adapter=product_adapter()
    for command in ['setup','doctor','check','record','source','pair','host','campaign','compare','launch','pair-source']+(adapter.commands() if adapter else []):
        q=sub.add_parser(command);q.add_argument('--extend-repo',type=Path)
        if command in ['setup','doctor','check','record']:q.add_argument('--phone')
        if command=='doctor':q.add_argument('--source-only',action='store_true')
        if adapter:adapter.arguments(command,q)
        if command=='setup':
            q.add_argument('--no-phone',action='store_true',help='Prepare receiver tools without installing the phone helper');q.add_argument('--replace-helper',action='store_true');q.add_argument('--source-only',action='store_true');q.add_argument('--sdk',type=Path);q.add_argument('--java-bin',type=Path);q.add_argument('--adb',type=Path)
        if command=='pair':q.add_argument('endpoint')
        if command in ['source','record']:q.add_argument('--seconds',type=int,default=180 if command=='source' else 30)
        if command=='source':q.add_argument('--screen',default='auto');q.add_argument('--list-screens',action='store_true')
        if command=='record':q.add_argument('--product',required=True);q.add_argument('--output',type=Path)
    a=p.parse_args();config=load();extend=repository(a.extend_repo,config)
    if adapter and a.command in adapter.commands():adapter.execute(a);return
    if a.command=='pair':
        adb=adb_at(config.get('adb'))
        code=getpass.getpass('Wireless debugging pairing code (not saved): ')
        if not re.fullmatch(r'\d{6}',code):raise RuntimeError('Expected six-digit pairing code')
        run([adb,'pair',a.endpoint],input=code+'\n',text=True)
        print('Paired. Use setup --phone DEBUGGING_ENDPOINT, not the pairing endpoint.');return
    if a.command=='record' and not re.fullmatch(r'[a-z][a-z0-9-]{0,39}',a.product):raise RuntimeError('Use a short lowercase product label')
    if a.command=='setup':
        if platform.system()!='Darwin':raise RuntimeError('Live optical capture requires macOS')
        tools=build_swift(extend);config['extend_repo']=str(extend)
        if adapter:adapter.prepare()
        if not a.source_only:
            venv_at();adb=adb_at(a.adb or config.get('adb'));sdk=sdk_at(a.sdk);java=java_at(a.java_bin)
            print('Building Benchmark Camera...',flush=True)
            build_log=ROOT/'.cache/android-build.log';build_log.parent.mkdir(exist_ok=True)
            with build_log.open('w') as log:
                try:run([sys.executable,ROOT/'android-camera/build.py','--sdk',sdk,'--java-bin',java],stdout=log,stderr=subprocess.STDOUT)
                except subprocess.CalledProcessError:raise RuntimeError(f'Android build failed; see {build_log}')
            config['adb']=str(adb)
            if not a.no_phone:
                serial=select_phone(adb,a.phone,config.get('phone_identity'))
                if a.replace_helper:run([adb,'-s',serial,'uninstall','computer.extend.benchmarkcamera'])
                try:run([adb,'-s',serial,'install','-r',ROOT/'android-camera/benchmark-camera.apk'])
                except subprocess.CalledProcessError:
                    raise RuntimeError('Helper install failed. If signing identity changed, setup --replace-helper removes only the old helper and its internal recordings. Saved run folders are retained.')
                run([adb,'-s',serial,'shell','pm','grant','computer.extend.benchmarkcamera','android.permission.CAMERA'])
                config['phone_identity']=phone_identity(adb,serial)
        save(config);entry='./benchmark/display' if adapter else './benchmarks/display/run';print('Setup complete. Next: '+entry+' '+('host --bind SOURCE_LAN_IP' if a.source_only else 'check'));return
    if a.command=='source':
        if not 3<=a.seconds<=600:raise RuntimeError('Source duration must be 3..600 seconds')
        tools=build_swift(extend)
        argv=[tools/'optical-patches','--screens'] if a.list_screens else [tools/'optical-patches',a.screen,str(a.seconds),'60']
        run(argv);return
    if a.phone:config['phone_override']=a.phone
    doctor(config,extend,source=getattr(a,'source_only',False))
    if a.command=='doctor':return
    adb=adb_at(config.get('adb'));serial=select_phone(adb,a.phone,config.get('phone_identity'))
    config['adb']=str(adb);config['phone_identity']=phone_identity(adb,serial);save(config)
    argv=[ROOT/'.venv/bin/python3',ROOT/'camera_run.py','--serial',serial,'--adb',adb,'--extend-repo',extend]
    if a.command=='check':argv+=['--check']
    else:
        if not 3<=a.seconds<=60:raise RuntimeError('Capture duration must be 3..60 seconds')
        argv+=['--product',a.product,'--seconds',str(a.seconds)]
        if a.output:argv+=['--output',a.output]
    run(argv)

if __name__=='__main__':
    try:main()
    except (Exception,KeyboardInterrupt) as error:
        print(f'Benchmark stopped: {error}',file=sys.stderr);sys.exit(1)
