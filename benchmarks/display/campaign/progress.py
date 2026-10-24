"""Small terminal presentation; no timers or threads during measurements."""
import contextlib,os,sys,time

class Progress:
    def __init__(self,stream=None,indent=''):
        self.stream=stream or sys.stdout;self.indent=indent
        self.color=self.stream.isatty() and os.environ.get('TERM')!='dumb' and 'NO_COLOR' not in os.environ
    def paint(self,text,code):
        return f'\033[{code}m{text}\033[0m' if self.color else text
    def line(self,text):
        print(self.indent+text,file=self.stream,flush=True)
    def heading(self,product,mode):
        self.line(self.paint('display benchmark','1')+'  '+self.paint('· '+product+' · '+mode,'36'))
    def scene(self,index,total,name,repetition,repetitions):
        self.line('')
        self.line(self.paint(f'{index:02}/{total:02}','36')+'  '+self.paint(name,'1')+self.paint(f'  · repeat {repetition}/{repetitions}','90'))
    @contextlib.contextmanager
    def step(self,label):
        start=time.monotonic()
        prefix=self.indent+self.paint('●','36')+'  '+label
        print(prefix,end='' if self.color else '\n',file=self.stream,flush=True)
        try:yield
        except BaseException:
            if self.color:print('\r',end='',file=self.stream)
            self.line(self.paint('×','31')+'  '+label+'  '+self.paint('failed','31'))
            raise
        else:
            if self.color:print('\r',end='',file=self.stream)
            self.line(self.paint('✓','32')+'  '+label+'  '+self.paint(f'{time.monotonic()-start:.1f}s','90'))

def camera_progress():
    return Progress(indent='   ') if os.environ.get('DISPLAY_BENCH_PROGRESS')=='1' else None
