"""Owned foreground fixture preparation; never signals an unrelated PID."""
import json, subprocess, time
from pathlib import Path

class Preparation:
    def __init__(self, command_path, output):
        raw=Path(command_path).read_bytes()
        self.argv=json.loads(raw)
        if not isinstance(self.argv,list) or not self.argv or len(self.argv)>64 or any(not isinstance(v,str) or not v or '\0' in v for v in self.argv):
            raise ValueError('Preparation command must be a nonempty argv JSON array')
        self.output=Path(output);self.process=None;self.log=None
        self.receipt={'argv':self.argv,'status':'not_started','scope':'Static focus preparation, excluded from measured scene'}
    def save(self):
        (self.output/'preparation.json').write_text(json.dumps(self.receipt,indent=2))
    def start(self, timeout=10):
        self.log=(self.output/'preparation.log').open('w')
        self.process=subprocess.Popen(self.argv,stdout=self.log,stderr=subprocess.STDOUT)
        self.receipt.update(status='starting',pid=self.process.pid,started_ns=time.monotonic_ns());self.save()
        deadline=time.monotonic()+timeout
        while time.monotonic()<deadline:
            if self.process.poll() is not None:raise RuntimeError('Preparation fixture exited before camera acquisition')
            if 'ready' in (self.output/'preparation.log').read_text().splitlines():
                self.receipt.update(status='ready',ready_ns=time.monotonic_ns());self.save();return
            time.sleep(.02)
        raise TimeoutError('Preparation fixture readiness timeout')
    def stop(self, reason='cleanup', require_live=False):
        if self.process is None:
            if self.log:self.log.close();self.log=None
            return
        live=self.process.poll() is None
        if live:
            self.process.terminate()
            try:self.process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self.process.kill();self.process.wait(timeout=2)
                self.receipt['forced_kill']=True
        self.receipt.update(status='intentionally_terminated' if live else 'already_exited',stop_reason=reason,stopped_ns=time.monotonic_ns(),returncode=self.process.returncode)
        self.save()
        if self.log:self.log.close();self.log=None
        self.process=None
        if require_live and not live:raise RuntimeError('Preparation fixture ended before camera readiness')
