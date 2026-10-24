"""Certificate-pinned source client. Configuration stays in ignored local state."""
import http.client,json,ssl,time,urllib.request,urllib.error
from pathlib import Path
from urllib.parse import urlsplit
from network_route import private_ipv4,routed_connection

class RoutedHTTPSConnection(http.client.HTTPSConnection):
    def __init__(self,host,*,source_ipv4,interface,**kwargs):
        super().__init__(host,source_address=(source_ipv4,0),**kwargs)
        self._create_connection=lambda address,timeout,source_address:routed_connection(address,timeout,source_address,interface)

class RoutedHTTPSHandler(urllib.request.HTTPSHandler):
    def __init__(self,context,source_ipv4,interface):
        super().__init__(context=context)
        self.source_ipv4=source_ipv4;self.interface=interface
    def https_open(self,request):
        def connection(host,**kwargs):
            return RoutedHTTPSConnection(host,source_ipv4=self.source_ipv4,interface=self.interface,**kwargs)
        return self.do_open(connection,request,context=self._context)

class Client:
    def __init__(self,path):
        self.config=json.loads(Path(path).read_text())
        if not self.config['url'].startswith('https://'):raise ValueError('HTTPS source required')
        self.context=ssl.create_default_context(cadata=self.config['certificate'])
        self.opener=None
        route=self.config.get('routing')
        if route is not None:
            if not isinstance(route,dict) or set(route)!={'source_ipv4','interface'}:raise ValueError('Routing requires source_ipv4 and interface')
            source=private_ipv4(route['source_ipv4']);private_ipv4(urlsplit(self.config['url']).hostname)
            if not isinstance(route['interface'],str) or not route['interface']:raise ValueError('Routing requires a network interface name')
            # A scoped local link must bypass environment HTTP proxy settings.
            self.opener=urllib.request.build_opener(urllib.request.ProxyHandler({}),RoutedHTTPSHandler(self.context,source,route['interface']))
    def request(self,path,body=None):
        data=None if body is None else json.dumps(body).encode()
        request=urllib.request.Request(self.config['url']+path,data=data,headers={'Authorization':'Bearer '+self.config['token'],'Content-Type':'application/json'})
        try:
            response=self.opener.open(request,timeout=60) if self.opener else urllib.request.urlopen(request,context=self.context,timeout=60)
            with response:return json.load(response)
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

def stop_focus_phase(client,ident,timeout=10):
    state=client.request('/status')
    if state.get('id')!=ident or state.get('scene')!='warmup':raise RuntimeError('Autofocus warm-up ownership mismatch')
    client.request('/stop',{})
    deadline=time.monotonic()+timeout
    while time.monotonic()<deadline:
        state=client.request('/status')
        if state.get('id')!=ident:raise RuntimeError('Autofocus warm-up changed during cancellation')
        if state.get('status') in ('complete','failed'):return
        time.sleep(.05)
    raise TimeoutError('Autofocus warm-up did not finish cancelling')

def trigger():
    import argparse
    p=argparse.ArgumentParser();p.add_argument('--pairing',type=Path,required=True);p.add_argument('--phase',type=Path,required=True);p.add_argument('--ack',type=Path,required=True)
    p.add_argument('--replace-focus-phase-id',help='Replace only this caller-owned autofocus warm-up')
    a=p.parse_args();client=Client(a.pairing)
    if a.replace_focus_phase_id:
        stop_focus_phase(client,a.replace_focus_phase_id)
    start=time.monotonic_ns();data=client.request('/start',json.loads(a.phase.read_text()))
    a.ack.write_text(json.dumps({'request_start_ns':start,'ack_ns':time.monotonic_ns(),'source':data}))
if __name__=='__main__':trigger()
