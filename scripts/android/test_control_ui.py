#!/usr/bin/env python3
"""Exercise tablet input against the focused disposable input-target.swift window.

The real desktop must be paired, accepting connections, and have input permission.
Start input-target.swift on the Mac and focus its window before running. No other
Mac application is monitored. Existing tablet pairings are preserved.
"""
import argparse, json, re, subprocess, time
from pathlib import Path
import xml.etree.ElementTree as E

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--serial', required=True)
p.add_argument('--device-name', required=True)
p.add_argument('--window-event-log', type=Path, required=True)
p.add_argument('--output', type=Path, required=True)
p.add_argument('--already-connected', action='store_true', help='Verify an explicitly opened control screen')
args = p.parse_args()
assert args.window_event_log.is_file(), 'Start the disposable native input target first'
args.output.mkdir(parents=True, exist_ok=False)
base = ['adb', '-s', args.serial]
def adb(*items):
    return subprocess.check_output(base + list(items), text=True, timeout=20)
def nodes():
    adb('shell', 'uiautomator', 'dump', '/sdcard/extend-control-ui.xml')
    return list(E.fromstring(adb('shell','cat','/sdcard/extend-control-ui.xml')).iter('node'))
def wait_text(text):
    deadline = time.monotonic()+30
    while time.monotonic()<deadline:
        items=nodes()
        if any(n.get('text')==text for n in items): return items
    raise RuntimeError('Expected tablet UI missing: '+text)
def rect(node):
    return list(map(int,re.findall(r'\d+',node.get('bounds',''))))
def tap(label, items=None):
    matches=[n for n in (items if items is not None else nodes()) if n.get('content-desc')==label]
    assert len(matches)==1, 'Ambiguous UI action: '+label
    a,b,c,d=rect(matches[0]); time.sleep(.5); adb('shell','input','touchscreen','-d','0','tap',str((a+c)//2),str((b+d)//2))
def activate(label):
    # Keyboard activation avoids device-specific shell touch routing at startup.
    for _ in range(16):
        items=nodes()
        matches=[n for n in items if n.get('content-desc')==label]
        assert len(matches)==1, 'Ambiguous UI action: '+label
        if matches[0].get('focused')=='true':
            adb('shell','input','keyevent','66'); return
        adb('shell','input','keyevent','61')
    raise RuntimeError('Could not focus '+label)
def events():
    return [json.loads(line) for line in args.window_event_log.read_text().splitlines() if line]
initial=len(events())
passed=False
try:
    if not args.already_connected:
        items=wait_text('Show pairing code'); assert any(n.get('text')==args.device_name for n in items)
        activate('Control '+args.device_name)
    items=wait_text('Controlling the main screen')
    trackpad=next(n for n in items if n.get('content-desc')=='Remote trackpad')
    a,b,c,d=rect(trackpad); x=(a+c)//2; y=(b+d)//2
    adb('shell','input','touchscreen','-d','0','swipe',str(x),str(y),str(x+80),str(y+40),'200')
    adb('shell','input','touchscreen','-d','0','tap',str(x),str(y))
    adb('shell','input','text','extend')
    tap('Remote right click'); tap('Remote tab')
    # A held drag must release when the app leaves the foreground.
    adb('shell','input','touchscreen','-d','0','motionevent','DOWN',str(x),str(y)); time.sleep(.7)
    before_home=events()[initial:]
    assert before_home and before_home[-1]['kind']=='left_down', 'Long press did not hold remote mouse; check native gesture and receiver logs'
    adb('shell','input','keyevent','3'); time.sleep(.8)
    samples=events()[initial:]
    kinds=[s['kind'] for s in samples]
    assert all(kind in kinds for kind in ['move','left_down','left_up','right_down','right_up','key_down','key_up'])
    assert kinds[-1]=='left_up', 'Background disconnect must release held mouse'
    typed=''.join(s['text'] for s in samples if s['kind']=='key_down')
    assert 'extend' in typed and '\t' in typed, 'Native typing and toolbar Tab required'
    (args.output/'events.json').write_text(json.dumps(samples,indent=2))
    adb('shell','am','start','-n','computer.extend.android.development/.MainActivity')
    wait_text('Control stopped'); adb('shell','input','keyevent','4'); items=wait_text('Show pairing code')
    assert any(n.get('text')==args.device_name for n in items)
    (args.output/'result.json').write_text(json.dumps({'status':'passed','checks':['actual-mac-native-injection','tablet-trackpad-movement','left-and-right-click','native-keyboard-typing','toolbar-tab','long-press-drag','held-mouse-released-on-background','existing-pairing-preserved'],'events':len(samples)},indent=2))
    passed=True
    print('PASS: physical tablet controls real Mac input target.',args.output)
finally:
    if not passed: adb('shell','input','keyevent','3')
    adb('shell','input','touchscreen','-d','0','motionevent','UP','0','0')
