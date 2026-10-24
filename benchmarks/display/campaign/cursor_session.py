"""Fixed synthetic cursor operations on one owned external-display fixture."""
import base64,hashlib,json,subprocess,time,uuid
from pathlib import Path

STATES=('box-left','cross-center','box-right','hidden','outside')
class CursorController:
    def __init__(self,session):self.session=session;self.process=None;self.ident=None;self.output=None;self.sequence=0
    def active(self):return self.process is not None and self.process.poll() is None
    def read(self,suffix,sequence=None):
        deadline=time.monotonic()+8
        while time.monotonic()<deadline:
            try:
                reply=json.loads(Path(str(self.output)+suffix).read_text())
                if sequence is None or reply.get('sequence')==sequence:
                    if reply.get('status')=='failed':raise RuntimeError(reply.get('error','Cursor fixture failed'))
                    return reply
            except (OSError,json.JSONDecodeError):pass
            if not self.active():raise RuntimeError('Cursor fixture exited before acknowledgment')
            time.sleep(.05)
        raise TimeoutError('Cursor fixture did not acknowledge')
    def start(self,body):
        if not {'id','product','screen'}<=set(body) or set(body)-{'id','product','screen','input_method'}:raise ValueError('Unexpected cursor fields')
        input_method=body.get('input_method','warp')
        if input_method not in ('warp','mouse-events'):raise ValueError('Unsupported cursor input method')
        ident=str(uuid.UUID(body['id']));screen=body['screen']
        if not isinstance(screen,str) or not (screen=='auto' or screen.isdigit()):raise ValueError('Invalid display')
        with self.session.lock:
            if self.active() or getattr(self.session,'interaction',None) and self.session.interaction.active() or self.session.quality.active() or self.session.job and self.session.job['status']=='running':raise ValueError('Another fixture running')
            if body['product'] not in self.session.contexts:raise ValueError('Preflight context required')
            dest=self.session.root/ident;dest.mkdir(parents=True,exist_ok=False,mode=0o700)
            self.ident=ident;self.output=dest/'cursor';self.sequence=0
            with (dest/'fixture.log').open('w') as log:
                self.process=subprocess.Popen([str(Path(self.session.tool).parent/'cursor-fixture'),str(self.output),screen,input_method],stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        try:
            reply=self.read('.ready.json')
            if reply.get('input_method')!=input_method:raise RuntimeError('Cursor fixture input method differs from request')
            tool=Path(self.session.tool).parent/'cursor-fixture'
            reply['fixture_sha256']=hashlib.sha256(tool.read_bytes()).hexdigest()
            return reply
        except BaseException:self.stop({'id':ident});raise
    def state(self,body):
        if set(body)!={'id','state'} or body['id']!=self.ident or body['state'] not in STATES:raise ValueError('Invalid cursor state')
        with self.session.lock:
            if not self.active():raise ValueError('Cursor fixture not running')
            self.sequence+=1;sequence=self.sequence
            p=Path(str(self.output)+'.control.json');temp=p.with_suffix('.pending')
            temp.write_text(json.dumps({'sequence':sequence,'state':body['state']}));temp.replace(p)
            reply=self.read('.state.json',sequence)
            raw=base64.b64decode(reply.get('cursor_png_base64',''),validate=True)
            if not raw.startswith(b'\x89PNG\r\n\x1a\n') or len(raw)>90*1024:raise RuntimeError('Invalid synthetic cursor PNG')
            return reply
    def stop(self,body):
        if set(body)!={'id'} or body['id']!=self.ident:raise ValueError('Cursor fixture ID mismatch')
        with self.session.lock:
            if self.active():
                self.process.terminate()
                try:self.process.wait(timeout=3)
                except subprocess.TimeoutExpired:self.process.kill();self.process.wait(timeout=3)
            cleanup=Path(str(self.output)+'.cleanup.json')
            return json.loads(cleanup.read_text()) if cleanup.exists() else {'position_restored':False,'visibility_restored':False}
