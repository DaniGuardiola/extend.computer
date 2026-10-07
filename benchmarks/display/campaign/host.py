"""Authenticated, fixed-operation source host; never accepts shell commands."""
import argparse,base64,json,os,secrets,signal,ssl,subprocess,threading,time,uuid
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from pathlib import Path
from context import host,application,sample,network
from stages import snapshot

SCENES={'warmup','static','scroll','panel','motion','recovery'}
class Session:
    def __init__(self,root,tool,adapter):
        self.root=Path(root);self.tool=tool;self.adapter=adapter;self.lock=threading.Lock();self.job=None;self.process=None;self.cancel=threading.Event();self.contexts={};self.ready=threading.Event()
    def context(self,product,peer=None):
        if product!='extend' and not (self.adapter and product in self.adapter.products()):raise ValueError('Unsupported product adapter')
        app=application(product,self.adapter)
        result={'host':host(),'application':app,'displays':self.displays(),'network':network(app,peer),'peer_ipv4':peer}
        self.contexts[product]=result;return result
    def displays(self):
        output=subprocess.run([str(self.tool),'--context'],capture_output=True,text=True,timeout=15,check=True).stdout
        return json.loads(output)
    def start(self,body):
        if set(body)!={'id','product','scene','seconds','screen'}:raise ValueError('Unexpected fields')
        ident=str(uuid.UUID(body['id']))
        if body['scene'] not in SCENES|{'baseline'} or type(body['seconds']) is not int or not 3<=body['seconds']<=60:raise ValueError('Invalid phase')
        if body['product']!='extend' and not (self.adapter and body['product'] in self.adapter.products()):raise ValueError('Unsupported product adapter')
        if not isinstance(body['screen'],str) or not (body['screen']=='auto' or body['screen'].isdigit()):raise ValueError('Invalid display')
        with self.lock:
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
                p=subprocess.Popen([str(self.tool),body['screen'],body['scene'],str(body['seconds']),str(dest/'fixture.json')],stdout=log,stderr=subprocess.STDOUT,start_new_session=True);log.close();self.process=p
            if p is not None:
                ready_deadline=time.monotonic()+10
                while time.monotonic()<ready_deadline and p.poll() is None:
                    if 'ready' in (dest/'fixture.log').read_text():break
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
                if abs(data['duration_s']-body['seconds'])>1:raise RuntimeError('Wrong scene duration')
            end=time.monotonic_ns()
            current=application(body['product'],self.adapter)
            if current['main_pid']!=app['main_pid'] or current['executable_sha256']!=app['executable_sha256']:
                raise RuntimeError('Source application restarted or changed during measurement')
            result={**body,'status':'complete','start_ns':start,'end_ns':end,'rows':rows,'fixture':data,'fault':fault,'stages':snapshot(app,self.adapter,start,end),
                    'context':{**context,'application':current,'displays':self.displays()},'network_start':self.job['network_start'],'network_end':network(app,context.get('peer_ipv4'))}
        except BaseException as error:result={**body,'status':'failed','reason':str(error)}
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
    def stop(self):self.cancel.set();return {'status':'cancel_requested'}

def main(tool,adapter,cache):
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--bind');p.add_argument('--port',type=int,default=8877);p.add_argument('--show-pairing',action='store_true')
    a=p.parse_args();import ipaddress
    existing=Path(cache)/'source-pairing.json'
    if not a.bind and existing.exists():
        from urllib.parse import urlsplit
        a.bind=urlsplit(json.loads(existing.read_text())['url']).hostname
    if not a.bind:p.error('First setup requires --bind LAN_IP; subsequent runs reuse paired address')
    ip=ipaddress.ip_address(a.bind)
    if not ip.is_private or ip.is_loopback or ip.version!=4:raise ValueError('Bind to a private LAN IPv4 address')
    cache=Path(cache);cache.mkdir(parents=True,exist_ok=True);config=cache/'source-pairing.json'
    cert=cache/'source-cert.pem';key=cache/'source-key.pem'
    previous=json.loads(config.read_text()) if config.exists() else {}
    if not cert.exists() or previous.get('url')!=f'https://{a.bind}:{a.port}':
        subprocess.run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-keyout',str(key),'-out',str(cert),'-days','365','-subj',f'/CN={a.bind}','-addext',f'subjectAltName=IP:{a.bind}'],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        key.chmod(0o600)
    previous=json.loads(config.read_text()) if config.exists() else {}
    cfg={'url':f'https://{a.bind}:{a.port}','token':previous.get('token') or secrets.token_urlsafe(32),'certificate':cert.read_text()}
    config.write_text(json.dumps(cfg));config.chmod(0o600)
    session=Session(cache/'source-runs',tool,adapter)
    class Handler(BaseHTTPRequestHandler):
        def log_message(self,*_):pass
        def answer(self,code,data):
            raw=json.dumps(data).encode();self.send_response(code);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(raw)));self.end_headers();self.wfile.write(raw)
        def authorized(self):return secrets.compare_digest(self.headers.get('Authorization',''),'Bearer '+cfg['token'])
        def do_GET(self):
            if not self.authorized():self.answer(401,{'error':'Unauthorized'});return
            try:
                if self.path=='/health':data={'status':'ready','protocol':1,'workload_sha256':json.loads((Path(tool).parents[0]/'campaign-workload.build.json').read_text())['source_sha256'],'displays':session.displays()}
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
                if not 0<length<=2048:raise ValueError('Invalid request size')
                body=json.loads(self.rfile.read(length))
                if self.path=='/start':data=session.start(body)
                elif self.path=='/stop' and body=={}:data=session.stop()
                else:raise ValueError('Unknown operation')
                self.answer(200,data)
            except Exception as error:self.answer(409,{'error':str(error)})
    server=ThreadingHTTPServer((a.bind,a.port),Handler)
    context=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);context.load_cert_chain(cert,key);server.socket=context.wrap_socket(server.socket,server_side=True)
    print('Source host ready. Pairing file: '+str(config),flush=True)
    if a.show_pairing:print('Pairing code: '+base64.urlsafe_b64encode(json.dumps(cfg).encode()).decode(),flush=True)
    print('Leave this terminal open. Ctrl-C stops only benchmark workload.',flush=True)
    def shutdown_signal(*_):raise KeyboardInterrupt
    signal.signal(signal.SIGTERM,shutdown_signal)
    try:server.serve_forever()
    except KeyboardInterrupt:pass
    finally:session.stop();server.server_close()
