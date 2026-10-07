"""Local HTTP integration only; no browser, app, screen or sensor access."""
import json
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.error
import urllib.request

from test_workload import phase


class ControlledWorkloadTests(unittest.TestCase):
    def test_readiness_start_completion_and_exclusive_file(self):
        with tempfile.TemporaryDirectory() as d:
            with socket.socket() as s:
                s.bind(('127.0.0.1',0));port=s.getsockname()[1]
            path=Path(d)/'phases.jsonl'
            command=[sys.executable,'-B','-m','benchmarks.display.serve_controlled','--port',str(port),'--output',str(path)]
            process=subprocess.Popen(command,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            def request(endpoint,data=None):
                body=None if data is None else json.dumps(data).encode()
                with urllib.request.urlopen(urllib.request.Request(f'http://127.0.0.1:{port}'+endpoint,data=body,headers={'Content-Type':'application/json'}),timeout=2) as response:
                    raw=response.read()
                    return json.loads(raw) if endpoint in {'/status','/command'} else raw
            try:
                deadline=time.monotonic()+5
                while True:
                    try:state=request('/status');break
                    except urllib.error.URLError:
                        if process.poll() is not None or time.monotonic()>deadline:raise RuntimeError('Server failed startup')
                        time.sleep(.02)
                self.assertFalse(state['ready'])
                config={'run_id':'01234567-89ab-4cde-8f01-23456789abcd','duration_s':1}
                try:
                    request('/start',config)
                    self.fail('Expected conflict')
                except urllib.error.HTTPError as error:
                    self.assertEqual(error.code,409);error.close()
                request('/command');request('/start',config)
                self.assertEqual(request('/command')['run_id'],config['run_id'])
                start=phase();request('/metrics',start)
                self.assertEqual(request('/status')['status'],'running')
                for scene in ['static','scroll','panel','motion']:
                    end=phase();end['kind']='phase_end';end['data'].update({'scene':scene,'measured':True,'elapsed_seconds':1,'source_callbacks':60,'source_callback_hz':60,'source_interval_p50_ms':16.6,'source_interval_p95_ms':16.7,'source_interval_max_ms':17,'source_intervals_over_33_33_ms':0,'hidden_during_phase':False,'resized_during_phase':False,'cancelled':False})
                    request('/metrics',end)
                self.assertEqual(request('/status')['status'],'complete')
                self.assertEqual(len(path.read_text().splitlines()),5)
                self.assertIn(b'display-baseline-v2',request('/workload.html'))
                try:
                    request('/start',config)
                    self.fail('Expected conflict')
                except urllib.error.HTTPError as error:
                    self.assertEqual(error.code,409);error.close()
                config['run_id']='11234567-89ab-4cde-8f01-23456789abcd';request('/start',config);request('/stop',{})
                self.assertEqual(request('/status')['status'],'cancelled')
            finally:
                process.terminate();process.wait(timeout=5)
            repeat=subprocess.run(command,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=5)
            self.assertNotEqual(repeat.returncode,0)

if __name__=='__main__':unittest.main()
