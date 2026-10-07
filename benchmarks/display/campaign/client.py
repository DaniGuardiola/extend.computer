"""Certificate-pinned source client. Configuration stays in ignored local state."""
import json,ssl,time,urllib.request,urllib.error
from pathlib import Path

class Client:
    def __init__(self,path):
        self.config=json.loads(Path(path).read_text())
        if not self.config['url'].startswith('https://'):raise ValueError('HTTPS source required')
        self.context=ssl.create_default_context(cadata=self.config['certificate'])
    def request(self,path,body=None):
        data=None if body is None else json.dumps(body).encode()
        request=urllib.request.Request(self.config['url']+path,data=data,headers={'Authorization':'Bearer '+self.config['token'],'Content-Type':'application/json'})
        try:
            with urllib.request.urlopen(request,context=self.context,timeout=60) as response:return json.load(response)
        except urllib.error.HTTPError as e:
            try:message=json.load(e).get('error','Source rejected operation')
            except Exception:message='Source rejected operation'
            raise RuntimeError(message) from None
    def wait(self,ident,seconds):
        end=time.monotonic()+seconds+90
        while time.monotonic()<end:
            state=self.request('/status')
            if state.get('id')!=ident:raise RuntimeError('Source phase ID mismatch')
            if state['status']=='failed':raise RuntimeError(state.get('reason','Source phase failed'))
            if state['status']=='complete':return state
            time.sleep(.5)
        raise TimeoutError('Source phase did not finish')

def trigger():
    import argparse
    p=argparse.ArgumentParser();p.add_argument('--pairing',type=Path,required=True);p.add_argument('--phase',type=Path,required=True);p.add_argument('--ack',type=Path,required=True)
    a=p.parse_args();client=Client(a.pairing);start=time.monotonic_ns();data=client.request('/start',json.loads(a.phase.read_text()))
    a.ack.write_text(json.dumps({'request_start_ns':start,'ack_ns':time.monotonic_ns(),'source':data}))
if __name__=='__main__':trigger()
