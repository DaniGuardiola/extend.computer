#!/usr/bin/env python3
import argparse, datetime, os, subprocess, time, json, xml.etree.ElementTree as E
from pathlib import Path
ROOT = Path(__file__).resolve().parents[2]
p = argparse.ArgumentParser(description="Verify pairing through the installed development app's actual UI. Requires an empty paired-device list; never removes existing devices.")
p.add_argument('--serial', required=True)
p.add_argument('--output', type=Path, default=ROOT / 'native/android/.local' / ('pairing-ui-' + datetime.datetime.now().strftime('%Y%m%d-%H%M%S')))
args = p.parse_args()
args.output = args.output.resolve()
args.output.mkdir(parents=True, exist_ok=False)
os.chdir(ROOT)
base=['adb','-s',args.serial]
def adb(*args): return subprocess.check_output(base+list(args),text=True)
def nodes():
 for attempt in range(3):
  try:
   adb('shell','uiautomator','dump','/sdcard/extend-ui-test.xml');break
  except subprocess.CalledProcessError:
   if attempt==2: raise
   time.sleep(.5)
 return list(E.fromstring(adb('shell','cat','/sdcard/extend-ui-test.xml')).iter('node'))
def focus(label):
 for _ in range(8):
  ns=nodes()
  if any(n.get('content-desc')==label and n.get('focused')=='true' for n in ns):
   print('Focused '+label,flush=True);return
  adb('shell','input','keyevent','61')
 raise RuntimeError('Cannot focus '+label)
def wait_text(text):
 end=time.monotonic()+30
 while time.monotonic()<end:
  if any(n.get('text')==text for n in nodes()): return
 raise RuntimeError('Expected UI text missing: '+text)
adb('shell','am','force-stop','computer.extend.android.development');adb('shell','am','start','-n','computer.extend.android.development/.MainActivity');wait_text('No paired devices');focus('Add device');adb('shell','input','keyevent','66');focus('Device name');adb('shell','input','text','UI-Test-Mac')
fixture=args.output / 'private-peer.json'
peer=subprocess.Popen(['target/debug/examples/tablet_pairing_peer',str(fixture)])
port=None
try:
 end=time.monotonic()+5
 while not fixture.exists() and time.monotonic()<end: time.sleep(.1)
 data=json.loads(fixture.read_text());port=data['address'].rsplit(':',1)[1]
 adb('reverse','--no-rebind','tcp:'+port,'tcp:'+port)
 focus('Device address');adb('shell','input','text',data['address'])
 focus('Pairing code');adb('shell','input','text',data['code'])
 focus('Pair device');adb('shell','input','keyevent','66')
 wait_text('UI-Test-Mac');print('UI paired peer present.',flush=True)
 focus('Check connection');adb('shell','input','keyevent','66')
 wait_text('Connected successfully');print('UI authenticated connection passed.',flush=True)
 adb('shell','am','force-stop','computer.extend.android.development')
 adb('shell','am','start','-n','computer.extend.android.development/.MainActivity')
 wait_text('UI-Test-Mac');print('UI paired peer present.',flush=True)
 focus('Check connection');adb('shell','input','keyevent','66')
 wait_text('Connected successfully');print('UI authenticated connection passed.',flush=True)
 peer.wait(timeout=15)
 assert peer.returncode==0
 adb('shell','screencap','-p','/sdcard/extend-paired-ui.png')
 adb('pull','/sdcard/extend-paired-ui.png',str(args.output / 'paired-ui.png'))
 focus('Forget');adb('shell','input','keyevent','66')
 wait_text('Forget device?')
 ns=nodes();target=next(n for n in ns if n.get('text','').lower()=='forget' and n.get('resource-id')=='android:id/button1')
 for _ in range(5):
  ns=nodes()
  if any(n.get('resource-id')=='android:id/button1' and n.get('focused')=='true' for n in ns): break
  adb('shell','input','keyevent','61')
 else: raise RuntimeError('Forget confirmation focus missing')
 adb('shell','input','keyevent','66')
 wait_text('No paired devices')
 (args.output / 'result.json').write_text(json.dumps({'status':'passed','checks':['form-pair','authenticated-check','saved-peer-after-restart','reconnect','confirmed-forget']},indent=2))
 print('PASS: actual mobile form, pairing, restart, reconnect and confirmed forgetting.')
finally:
 if port: adb('reverse','--remove','tcp:'+port)
 if peer.poll() is None: peer.terminate();peer.wait(timeout=5)
