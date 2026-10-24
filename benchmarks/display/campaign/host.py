"""Authenticated, fixed-operation source host; never accepts shell commands."""
import argparse,base64,json,os,secrets,signal,ssl,subprocess,threading,time,uuid
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from pathlib import Path
from context import host,application,sample,network
from network_route import private_ipv4,bind_interface
from stages import snapshot

SCENES={'warmup','static','scroll','panel','motion','recovery','cursor','cursor-optical','cursor-sweep'}
class Session:
    def __init__(self,root,tool,adapter,settings_file=None,patch_sides='left,right'):
        if patch_sides not in {'left,right','right,left'}:raise ValueError('Invalid timing patch sides')
        self.patch_sides=patch_sides
        self.settings_file=Path(settings_file) if settings_file else None
        self.root=Path(root);self.tool=tool;self.adapter=adapter;self.lock=threading.Lock();self.job=None;self.process=None;self.cancel=threading.Event();self.contexts={};self.ready=threading.Event()
        from quality_session import QualityController
        self.quality=QualityController(self)
        from cursor_session import CursorController
        self.cursor=CursorController(self)
        from interaction_session import InteractionController
        self.interaction=InteractionController(self)
    def context(self,product,peer=None):
        if product!='extend' and not (self.adapter and product in self.adapter.products()):raise ValueError('Unsupported product adapter')
        app=application(product,self.adapter)
        result={'host':host(),'application':app,'displays':self.displays(),'network':network(app,peer),'peer_ipv4':peer,'fixture_patch_sides':self.patch_sides.split(',')}
        self.contexts[product]=result;return result
    def displays(self):
        output=subprocess.run([str(self.tool),'--context'],capture_output=True,text=True,timeout=15,check=True).stdout
        return json.loads(output)
    def start(self,body):
        if not {'id','product','scene','seconds','screen'}<=set(body) or set(body)-{'id','product','scene','seconds','screen','input_method'}:raise ValueError('Unexpected fields')
        if body.get('input_method','warp') not in ('warp','mouse-events') or 'input_method' in body and body['scene'] not in {'cursor','cursor-optical','cursor-sweep'}:raise ValueError('Invalid cursor input method')
        ident=str(uuid.UUID(body['id']))
        maximum=600 if body['scene']=='motion' else 60
        if body['scene'] not in SCENES|{'baseline'} or type(body['seconds']) is not int or not 3<=body['seconds']<=maximum:raise ValueError('Invalid phase')
        if body['product']!='extend' and not (self.adapter and body['product'] in self.adapter.products()):raise ValueError('Unsupported product adapter')
        if not isinstance(body['screen'],str) or not (body['screen']=='auto' or body['screen'].isdigit()):raise ValueError('Invalid display')
        with self.lock:
            if self.quality.active() or self.cursor.active() or self.interaction.active():raise ValueError('Another fixture running')
            if self.job and self.job['status']=='running':raise ValueError('Another phase running')
            context=self.contexts.get(body['product'])
            if not context:raise ValueError('Preflight context required')
            app=context['application'];os.kill(app['main_pid'],0)
            destination=self.root/ident;destination.mkdir(parents=True,exist_ok=False)
            self.cancel.clear();self.ready.clear();self.job={**body,'status':'running','context':context,'rows':[],'network_start':context['network']}
            threading.Thread(target=self.measure,args=(dict(body),app,destination,context),daemon=True).start()
        if not self.ready.wait(12):self.cancel.set();raise RuntimeError('Fixture did not acknowledge readiness')
        return {'id':ident,'status':'running'}
    def measure(self,body,app,dest,context):
        p=None;data=None;resumer=None;paused=[];fault=None
        try:
            if body['scene']!='baseline':
                log=(dest/'fixture.log').open('w')
                command=[str(self.tool),body['screen'],body['scene'],str(body['seconds']),str(dest/'fixture.json')]
                if 'input_method' in body:command.append(body['input_method'])
                if self.patch_sides!='left,right':command.extend(['--patch-sides',self.patch_sides])
                p=subprocess.Popen(command,stdout=log,stderr=subprocess.STDOUT,start_new_session=True);log.close();self.process=p
            if p is not None:
                ready_deadline=time.monotonic()+10
                while time.monotonic()<ready_deadline:
                    if 'ready' in (dest/'fixture.log').read_text():break
                    if p.poll() is not None:raise RuntimeError('Workload exited before readiness')
                    time.sleep(.02)
                else:raise RuntimeError('Workload did not become ready')
            self.ready.set()
            start=time.monotonic_ns();rows=[];deadline=time.monotonic()+body['seconds']+3
            while time.monotonic()<deadline and not self.cancel.is_set():
                rows.append(sample(app))
                if body['scene']=='recovery' and fault is None and (time.monotonic_ns()-start)/1e9>=body['seconds']/3:
                    # Independent child resumes even if the host is interrupted.
                    owned=[r['pid'] for r in __import__('context').processes() if r['pid'] in app['pids'] and r['path'].startswith(app['bundle_path']+'/')]
                    if not owned:raise RuntimeError('No owned application processes available for recovery scenario')
                    resume_code='import os,signal,time\ntime.sleep(2)\nfor pid in '+repr(owned)+':\n try: os.kill(pid,signal.SIGCONT)\n except ProcessLookupError: pass\n'
                    resumer=subprocess.Popen([__import__('sys').executable,'-c',resume_code],start_new_session=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
                    for pid in owned:
                        os.kill(pid,signal.SIGSTOP);paused.append(pid)
                    fault={'scenario':'two_second_sender_process_pause','at_ns':time.monotonic_ns(),'duration_s':2,'scope_pids':owned,'network_fault':False}

                if p is None and (time.monotonic_ns()-start)/1e9>=body['seconds']:break
                if p is not None and p.poll() is not None:break
                self.cancel.wait(.5)
            if self.cancel.is_set():raise RuntimeError('Cancelled')
            if p is not None:
                if p.poll() is None:raise TimeoutError('Fixture deadline exceeded')
                if p.returncode!=0:raise RuntimeError('Fixture failed; ensure exactly one extended display')
                data=json.loads((dest/'fixture.json').read_text())
                if not data['valid_geometry']:raise RuntimeError('Display geometry changed')
                if data.get('valid_fixture') is False:raise RuntimeError(data.get('failure_reason') or 'Source fixture validation failed')
                if abs(data['duration_s']-body['seconds'])>1:raise RuntimeError('Wrong scene duration')
            end=time.monotonic_ns()
            current=application(body['product'],self.adapter)
            if current['main_pid']!=app['main_pid'] or current['executable_sha256']!=app['executable_sha256']:
                raise RuntimeError('Source application restarted or changed during measurement')
            result={**body,'status':'complete','start_ns':start,'end_ns':end,'rows':rows,'fixture':data,'fault':fault,'stages':snapshot(app,self.adapter,start,end),
                    'context':{**context,'application':current,'displays':self.displays()},'network_start':self.job['network_start'],'network_end':network(app,context.get('peer_ipv4'))}
        except BaseException as error:
            result={**body,'status':'failed','reason':str(error)}
            fixture_path=dest/'fixture.json'
            if fixture_path.exists():
                try:result['fixture']=json.loads(fixture_path.read_text())
                except (OSError,ValueError):pass
        finally:
            for pid in paused:
                try:os.kill(pid,signal.SIGCONT)
                except ProcessLookupError:pass
            self.ready.set()
            if p and p.poll() is None:
                os.killpg(p.pid,signal.SIGTERM)
                try:p.wait(timeout=5)
                except subprocess.TimeoutExpired:os.killpg(p.pid,signal.SIGKILL);p.wait()
            self.process=None
        (dest/'result.json').write_text(json.dumps(result,indent=2))
        with self.lock:
            self.job=result
            if result['status']=='complete':self.contexts[body['product']]['network']=result['network_end']
    def display_settings(self,body):
        if not self.settings_file:raise ValueError('Source host was not started with --display-settings-file')
        if set(body)!={'product','config'} or body['product']!='extend':raise ValueError('Unsupported display settings operation')
        config=body['config']
        if not isinstance(config,dict) or set(config)!={'mode','width','height','fps','bitrate','retina','auto_size','codec','separate_cursor'}:raise ValueError('Invalid display settings fields')
        if config['mode'] not in {'extend','mirror'} or config['codec'] not in {'hevc','h264'}:raise ValueError('Invalid display mode or codec')
        if any(type(config[key]) is not bool for key in ['retina','auto_size','separate_cursor']):raise ValueError('Invalid display settings flags')
        if any(type(config[key]) is not int for key in ['width','height','fps','bitrate']):raise ValueError('Invalid display settings dimensions')
        scale=2 if config['retina'] else 1
        if not (640<=config['width']<=4096//scale and 480<=config['height']<=4096//scale and 30<=config['fps']<=120 and 1_000_000<=config['bitrate']<=80_000_000):raise ValueError('Display settings outside supported bounds')
        with self.lock:
            if self.job and self.job['status']=='running':raise ValueError('Cannot change settings during a measured phase')
            if self.quality.active() or self.cursor.active() or self.interaction.active():raise ValueError('Cannot change settings during a fixture')
            application('extend',self.adapter) # Require exactly one live prepared application.
            sequence=time.time_ns()
            temporary=self.settings_file.with_suffix('.pending')
            temporary.write_text(json.dumps({'sequence':sequence,'config':config}));temporary.chmod(0o600)
            temporary.replace(self.settings_file)
            deadline=time.monotonic()+15
            while time.monotonic()<deadline:
                try:
                    ack=json.loads(self.settings_file.with_suffix('.ack.json').read_text())
                    if ack.get('sequence')==sequence:
                        if ack.get('ok') is not True:raise ValueError(ack.get('error') or 'Display settings rejected')
                        return ack
                except (OSError,json.JSONDecodeError):pass
                time.sleep(.1)
            raise TimeoutError('Source app did not acknowledge settings; launch it with --settings-file at the same path')

    def stop(self):
        self.cancel.set()
        if self.quality.ident:self.quality.stop({'id':self.quality.ident})
        if self.cursor.ident:self.cursor.stop({'id':self.cursor.ident})
        if self.interaction.ident:self.interaction.stop({'id':self.interaction.ident})
        return {'status':'cancel_requested'}

def main(tool,adapter,cache):
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--bind');p.add_argument('--port',type=int,default=8877);p.add_argument('--show-pairing',action='store_true')
    p.add_argument('--interface',help='Pin the listening socket to this macOS interface after binding its address')
    p.add_argument('--patch-sides',choices=['left,right','right,left'],default='left,right',help='Physical source and virtual timing-patch edges; choose the two inner edges visible to the camera')
    p.add_argument('--display-settings-file',type=Path,help='Explicit fixed development settings path shared with launch --settings-file')
    a=p.parse_args()
    existing=Path(cache)/'source-pairing.json'
    if not a.bind and existing.exists():
        from urllib.parse import urlsplit
        a.bind=urlsplit(json.loads(existing.read_text())['url']).hostname
    if not a.bind:p.error('First setup requires --bind PRIVATE_OR_LINK_LOCAL_IP; subsequent runs reuse paired address')
    a.bind=private_ipv4(a.bind)
    cache=Path(cache);cache.mkdir(parents=True,exist_ok=True);config=cache/'source-pairing.json'
    cert=cache/'source-cert.pem';key=cache/'source-key.pem'
    previous=json.loads(config.read_text()) if config.exists() else {}
    if not cert.exists() or previous.get('url')!=f'https://{a.bind}:{a.port}':
        subprocess.run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-keyout',str(key),'-out',str(cert),'-days','365','-subj',f'/CN={a.bind}','-addext',f'subjectAltName=IP:{a.bind}'],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        key.chmod(0o600)
    previous=json.loads(config.read_text()) if config.exists() else {}
    cfg={'url':f'https://{a.bind}:{a.port}','token':previous.get('token') or secrets.token_urlsafe(32),'certificate':cert.read_text()}
    config.write_text(json.dumps(cfg));config.chmod(0o600)
    session=Session(cache/'source-runs',tool,adapter,a.display_settings_file,a.patch_sides)
    class Handler(BaseHTTPRequestHandler):
        def log_message(self,*_):pass
        def answer(self,code,data):
            raw=json.dumps(data).encode();self.send_response(code);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(raw)));self.end_headers();self.wfile.write(raw)
        def authorized(self):return secrets.compare_digest(self.headers.get('Authorization',''),'Bearer '+cfg['token'])
        def do_GET(self):
            if not self.authorized():self.answer(401,{'error':'Unauthorized'});return
            try:
                if self.path=='/health':data={'status':'ready','protocol':1,'workload_sha256':json.loads((Path(tool).parents[0]/'campaign-workload.build.json').read_text())['source_sha256'],'displays':session.displays(),'fixture_patch_sides':session.patch_sides.split(',')}
                elif self.path.startswith('/context/') and self.path.count('/')==2:data=session.context(self.path.rsplit('/',1)[1],self.client_address[0])
                elif self.path=='/status':
                    with session.lock:data=session.job or {'status':'idle'}
                else:self.answer(404,{'error':'Unknown operation'});return
                self.answer(200,data)
            except Exception as error:self.answer(409,{'error':str(error)})
        def do_POST(self):
            if not self.authorized():self.answer(401,{'error':'Unauthorized'});return
            try:
                length=int(self.headers.get('Content-Length','0'))
                limit=4_002_048 if self.path=='/quality/start' else 2048
                if not 0<length<=limit:raise ValueError('Invalid request size')
                body=json.loads(self.rfile.read(length))
                if self.path=='/start':data=session.start(body)
                elif self.path=='/display-settings':data=session.display_settings(body)
                elif self.path=='/quality/start':data=session.quality.start(body)
                elif self.path=='/quality/stop':data=session.quality.stop(body)
                elif self.path=='/interaction/start':data=session.interaction.start(body)
                elif self.path=='/interaction/state':data=session.interaction.state(body)
                elif self.path=='/interaction/stop':data=session.interaction.stop(body)
                elif self.path=='/cursor/start':data=session.cursor.start(body)
                elif self.path=='/cursor/state':data=session.cursor.state(body)
                elif self.path=='/cursor/stop':data=session.cursor.stop(body)
                elif self.path=='/stop' and body=={}:data=session.stop()
                else:raise ValueError('Unknown operation')
                self.answer(200,data)
            except Exception as error:self.answer(409,{'error':str(error)})
    server=ThreadingHTTPServer((a.bind,a.port),Handler)
    if a.interface:bind_interface(server.socket,a.interface)
    context=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);context.load_cert_chain(cert,key);server.socket=context.wrap_socket(server.socket,server_side=True)
    print('Source host ready. Pairing file: '+str(config),flush=True)
    if a.show_pairing:print('Pairing code: '+base64.urlsafe_b64encode(json.dumps(cfg).encode()).decode(),flush=True)
    print('Leave this terminal open. Ctrl-C stops only benchmark workload.',flush=True)
    def shutdown_signal(*_):raise KeyboardInterrupt
    signal.signal(signal.SIGTERM,shutdown_signal)
    try:server.serve_forever()
    except KeyboardInterrupt:pass
    finally:session.stop();server.server_close()
