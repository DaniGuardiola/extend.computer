"""Allowlisted host, product and resource evidence; no account tokens or serials."""
import hashlib,json,os,platform,plistlib,re,subprocess,time
from pathlib import Path

def command(argv):
    result=subprocess.run(argv,capture_output=True,text=True,timeout=15,check=True)
    return result.stdout.strip()

def digest(path):
    h=hashlib.sha256()
    with Path(path).open('rb') as f:
        for block in iter(lambda:f.read(1048576),b''):h.update(block)
    return h.hexdigest()

def host():
    def sysctl(key):
        try:return command(['/usr/sbin/sysctl','-n',key])
        except subprocess.SubprocessError:return None
    return {'model':sysctl('hw.model'),'chip':sysctl('machdep.cpu.brand_string'),
            'memory_bytes':int(sysctl('hw.memsize') or 0),'logical_cpus':int(sysctl('hw.ncpu') or 0),
            'os':platform.mac_ver()[0],'architecture':platform.machine(),
            'power':power(),'serial_numbers_collected':False}

def power():
    try:
        text=command(['/usr/bin/pmset','-g','batt'])
        return {'source':'ac' if 'AC Power' in text else 'battery','battery_percent':int(re.search(r'(\d+)%',text)[1]) if re.search(r'(\d+)%',text) else None}
    except subprocess.SubprocessError:return {'source':'unavailable','battery_percent':None}

def processes():
    rows=[]
    for line in command(['/bin/ps','-axo','pid=,ppid=,time=,rss=,comm=']).splitlines():
        bits=line.strip().split(None,4)
        if len(bits)!=5:continue
        pid,parent,cpu,rss,path=bits
        try:
            days,clock=cpu.split('-',1) if '-' in cpu else ('0',cpu)
            parts=[float(x) for x in clock.split(':')]
            seconds=int(days)*86400+sum(v*60**i for i,v in enumerate(reversed(parts)))
            rows.append({'pid':int(pid),'parent':int(parent),'cpu_s':seconds,'rss_bytes':int(rss)*1024,'path':path})
        except ValueError:continue
    return rows

def build_evidence(log):
    """Read explicit build flags from this launch's app-owned numeric log."""
    try:
        for line in Path(log).read_text().splitlines():
            if not line.startswith('Display build '):continue
            value=json.loads(line[len('Display build '):])
            expected={'debug_assertions','development_identity','benchmark_feature'}
            # Startup diagnostics may also describe created webviews. Preserve
            # the strict boolean build evidence without rejecting extra metadata.
            if isinstance(value,dict) and expected.issubset(value) and all(type(value[k]) is bool for k in expected):
                return {'status':'observed_compile_flags',**{k:value[k] for k in expected}}
    except (OSError,ValueError,TypeError):pass
    return {'status':'unavailable'}

def application(product,adapter=None):
    rows=processes()
    bundle=adapter.application(product) if adapter and hasattr(adapter,'application') else None
    if bundle:
        info=plistlib.loads((Path(bundle)/'Contents/Info.plist').read_bytes())
        executable=str(Path(bundle)/'Contents/MacOS'/info['CFBundleExecutable'])
        candidates=[r for r in rows if r['path']==executable]
    else:
        candidates=[]
        for r in rows:
            if '/extend.computer.app/Contents/MacOS/' not in r['path']:continue
            candidate_bundle=Path(r['path'].split('.app/')[0]+'.app')
            try:
                info=plistlib.loads((candidate_bundle/'Contents/Info.plist').read_bytes())
                if r['path']==str(candidate_bundle/'Contents/MacOS'/info['CFBundleExecutable']):candidates.append(r)
            except (OSError,plistlib.InvalidFileException,KeyError):continue
        if len(candidates)==1:bundle=Path(candidates[0]['path'].split('.app/')[0]+'.app')
    if len(candidates)!=1:raise RuntimeError(f'Expected exactly one running prepared {product} app; open it and close other copies')
    root=candidates[0];bundle=Path(bundle)
    info=plistlib.loads((bundle/'Contents/Info.plist').read_bytes())
    related={root['pid']}
    for _ in range(10):related.update(r['pid'] for r in rows if r['parent'] in related)
    # Also include separately launched native helpers inside the same bundle.
    related.update(r['pid'] for r in rows if str(bundle)+'/' in r['path'])
    display_helpers=[];instrumentation={"status":"unavailable"}
    for r in rows:
        if r['pid'] not in related or 'display' not in Path(r['path']).name.lower():continue
        try:
            args=command(['/bin/ps','-p',str(r['pid']),'-o','command='])
            if not args.startswith(r['path']):continue
            fields=args[len(r['path']):].strip().split()
            if len(fields)>=6 and fields[0] in {'extend','mirror','viewer'} and all(x.isdigit() for x in fields[1:5]) and fields[5] in {'retina','standard'}:
                display_helpers.append({'pid':r['pid'],'role':fields[0],'logical_width':int(fields[1]),'logical_height':int(fields[2]),'requested_fps':int(fields[3]),'requested_bitrate_bps':int(fields[4]),'scale':2 if fields[5]=='retina' else 1,'codec':fields[7] if len(fields)>7 else 'h264','separate_cursor':len(fields)>8 and fields[8]=='cursor','evidence':'active display helper numeric argv'})
        except subprocess.SubprocessError:pass
    # A running helper's argv describes startup only. Successful in-session
    # updates emit acknowledged numeric configuration in this app's own trace.
    if product=='extend' and display_helpers:
        from bootstrap import ROOT
        from stages import read_numeric, summarize
        state=ROOT/'.cache/extend-session.json'
        try:
            current=json.loads(state.read_text())
            if current.get('pid')==root['pid']:
                instrumentation={'status':'observed_launch_flags','fixed_bitrate':False,'kernel_arrival':False,**current.get('diagnostics',{}),'build':build_evidence(current['log'])}
                configuration=summarize(read_numeric(current['trace']),0,time.monotonic_ns())['configuration']
                for helper in display_helpers:
                    for source,target in [('width','logical_width'),('height','logical_height'),('fps','requested_fps'),('bitrate','requested_bitrate_bps')]:
                        if source in configuration:helper[target]=configuration[source]
                    if 'retina' in configuration:helper['scale']=2 if configuration['retina'] else 1
                    if 'width' in configuration:helper['evidence']='active helper and acknowledged numeric configuration'
        except (OSError,ValueError,KeyError):pass
    return {'display_helpers':display_helpers,'instrumentation':instrumentation,'product':product,'version':info.get('CFBundleShortVersionString'),'build':info.get('CFBundleVersion'),
            'executable_sha256':digest(bundle/'Contents/MacOS'/info['CFBundleExecutable']),
            'bundle_path':str(bundle),'main_pid':root['pid'],'pids':sorted(related),
            'helpers':[{'pid':r['pid'],'executable':Path(r['path']).name,'sha256':digest(r['path']) if Path(r['path']).is_file() else None} for r in rows if r['pid'] in related],
            'scope':'main application, descendants and executables inside its app bundle'}

def sample(app):
    started=time.monotonic_ns();rows=processes();roots=set(app['pids'])
    for _ in range(10):roots.update(r['pid'] for r in rows if r['parent'] in roots)
    selected=[r for r in rows if r['pid'] in roots or r['path'].startswith(app['bundle_path']+'/')]
    system_cpu=sum(r['cpu_s'] for r in rows)
    gpu_sample=gpu()
    return {'at_ns':time.monotonic_ns(),'processes':[{'pid':r['pid'],'cpu_s':r['cpu_s'],'rss_bytes':r['rss_bytes']} for r in selected],
            'system_process_cpu_s':system_cpu,'collection_ms':(time.monotonic_ns()-started)/1e6,
            'gpu':gpu_sample,'main_alive':any(r['pid']==app['main_pid'] for r in selected)}

def network(app,peer=None):
    # OS counters, never packet contents. Keep raw output out of reports.
    flows=[];errors=[]
    selected=selected_udp_endpoint(app,peer)
    for pid in app['pids']:
        try:
            text=command(['/usr/bin/nettop','-n','-p',str(pid),'-L','1','-x','-J','interface,state,bytes_in,bytes_out,re-tx,rtt_avg'])
            import csv,io
            rows=list(csv.reader(io.StringIO(text)))
            if not rows:continue
            header=rows[0]
            for row in rows[1:]:
                if not row or '->' not in ','.join(row):continue
                values={k:v for k,v in zip(header,row)}
                label=next((v for v in row if '->' in v),'')
                match=peer_match(label,peer);basis='os_remote_endpoint';route_hash=None;ice_hash=None
                if match is None and selected and pid==app['main_pid'] and 'udp' in label.lower():
                    local=label.split('<->',1)[0].split()[-1]
                    if local==selected['local']:
                        match=selected['matches_peer'];basis=selected['basis']
                        route_hash=selected['tuple_sha256']
                        ice_hash=selected['ice_tuple_sha256']
                flows.append({'pid':pid,'flow_sha256':hashlib.sha256(label.encode()).hexdigest(),
                              'protocol':'udp' if 'udp' in label.lower() else 'tcp','matches_peer':match,
                              'matches_peer_basis':basis,'selected_tuple_sha256':route_hash,'ice_tuple_sha256':ice_hash,
                              'interface':values.get('interface'),'state':values.get('state'),
                              **{k:values.get(k) for k in ['bytes_in','bytes_out','re-tx','rtt_avg']}})
        except (OSError,subprocess.SubprocessError):errors.append({'pid':pid,'reason':'os_network_sensor_unavailable'})
    return {'flows':flows,'errors':errors,'method':'nettop; peer addresses hashed; no payload inspected'}

def selected_udp_endpoint(app,peer):
    """Fresh first-party socket tuple; usable only with the same owning OS socket."""
    if app.get('product')!='extend' or not peer:return None
    from bootstrap import ROOT
    import ipaddress
    try:
        state=json.loads((ROOT/'.cache/extend-session.json').read_text())
        if state['pid']!=app['main_pid']:return None
        with Path(state['log']).open('rb') as f:
            f.seek(0,2);f.seek(max(0,f.tell()-65536));lines=f.read().decode('utf8',errors='replace').splitlines()
        for line in reversed(lines):
            if not line.startswith('Display route '):continue
            row=json.loads(line[len('Display route '):])
            age=(time.time_ns()-row['wall_ns'])/1e9
            if not 0<=age<=12 or row['protocol']!='udp' or row['responses_received']<1:return None
            explicit_socket='socket_local' in row or 'socket_remote' in row
            local=row['socket_local'] if explicit_socket else row['local']
            remote=row['socket_remote'] if explicit_socket else row['remote']
            if not all(isinstance(value,str) for value in [local,remote,row['local'],row['remote']]):return None
            local_ip,local_port=local.rsplit(':',1);remote_ip,remote_port=remote.rsplit(':',1)
            # This fallback currently covers IPv4-bound host sockets only.
            if ipaddress.ip_address(local_ip).version!=4 or ipaddress.ip_address(remote_ip).version!=4:return None
            if not all(0<int(port)<65536 for port in [local_port,remote_port]):return None
            return {'local':local,'matches_peer':ipaddress.ip_address(remote_ip)==ipaddress.ip_address(peer),
                    'basis':'selected_socket_pair_and_os_local_endpoint' if explicit_socket else 'selected_candidate_pair_and_os_local_endpoint',
                    'tuple_sha256':hashlib.sha256((local+'|'+remote).encode()).hexdigest(),
                    'ice_tuple_sha256':hashlib.sha256((row['local']+'|'+row['remote']).encode()).hexdigest()}
    except (OSError,ValueError,KeyError,TypeError):pass
    return None

def peer_match(label,peer):
    """Unconnected socket counters do not reveal their datagram destination."""
    remote=label.rsplit('->',1)[-1] if '->' in label else ''
    addresses=re.findall(r'(?:[0-9]{1,3}\.){3}[0-9]{1,3}',remote)
    if not peer or not addresses:return None
    return peer in addresses


def gpu():
    try:
        raw=subprocess.run(['/usr/sbin/ioreg','-r','-c','AGXAccelerator','-a'],capture_output=True,check=True,timeout=5).stdout
        devices=[]
        for item in plistlib.loads(raw):
            values=item.get('PerformanceStatistics',{})
            devices.append({key:values.get(key) for key in ['Device Utilization %','Renderer Utilization %','Tiler Utilization %']})
        return {'status':'measured' if devices else 'unavailable','devices':devices,'scope':'whole host GPU; includes product, workload and other apps'}
    except (OSError,subprocess.SubprocessError,plistlib.InvalidFileException):return {'status':'unavailable','reason':'AGX counters unavailable on this host'}
