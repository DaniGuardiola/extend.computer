#!/usr/bin/env python3
"""Verify foreground incoming code pairing over LAN through the actual tablet UI.

Requires an empty paired list and a verified tablet fingerprint. Uses only a
fresh temporary desktop identity, then forgets that test peer through the UI.
"""
import argparse, atexit, datetime, json, os, re, socket, subprocess, time
import xml.etree.ElementTree as E
from pathlib import Path
ROOT = Path(__file__).resolve().parents[2]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--serial', required=True)
p.add_argument('--tablet-peer', required=True, help='Fingerprint from physical engine verification')
p.add_argument('--decision', choices=['allow', 'deny', 'cancel'], default='allow')
p.add_argument('--receiving', action='store_true', help='Also verify foreground-service reconnect while app is backgrounded')
p.add_argument('--output', type=Path, default=ROOT/'native/android/.local'/('incoming-ui-'+datetime.datetime.now().strftime('%Y%m%d-%H%M%S')))
args = p.parse_args()
if args.receiving and args.decision != 'allow': p.error('--receiving requires --decision allow')
args.output = args.output.resolve(); args.output.mkdir(parents=True, exist_ok=False)
os.chdir(ROOT)
base = ['adb', '-s', args.serial]
def adb(*items): return subprocess.check_output(base+list(items), text=True, timeout=20)
def nodes():
    adb('shell', 'uiautomator', 'dump', '/sdcard/extend-incoming-ui.xml')
    return list(E.fromstring(adb('shell', 'cat', '/sdcard/extend-incoming-ui.xml')).iter('node'))
def wait_text(text):
    deadline = time.monotonic()+30
    while time.monotonic()<deadline:
        items=nodes()
        if any(n.get('text')==text for n in items): return items
    raise RuntimeError('Expected UI text missing: '+text)
def activate(label):
    for _ in range(12):
        items=nodes()
        if any(n.get('content-desc')==label and n.get('focused')=='true' for n in items):
            adb('shell','input','keyevent','66'); return
        adb('shell','input','keyevent','61')
    raise RuntimeError('Could not focus '+label)
def closed(address):
    host,port=address.rsplit(':',1)
    try:
        with socket.create_connection((host,int(port)),timeout=2): pass
    except OSError: return
    raise RuntimeError('Listener remained open after stopping it')
def receiving_endpoint():
    query=subprocess.Popen(['/usr/bin/dns-sd','-L','extend-presence-'+args.tablet_peer[:12],'_extendpresence._tcp','local.'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
    try: resolved=query.communicate(timeout=6)[0]
    except subprocess.TimeoutExpired:
        query.terminate();resolved=query.communicate(timeout=3)[0]
    assert 'capability=presence' in resolved and 'version=1' in resolved, 'Expected availability advertisement missing'
    endpoints=re.findall(r'can be reached at ([^\s:]+):(\d+)', resolved)
    assert endpoints, 'Availability endpoint missing'
    host,port=endpoints[-1]
    return socket.gethostbyname(host)+':'+port
availability_changed=False
def cleanup_availability():
    global availability_changed
    if not availability_changed: return
    adb('shell','am','force-stop','computer.extend.android.development')
    adb('shell','am','start','-n','computer.extend.android.development/.MainActivity')
    wait_text('Show pairing code');activate('Settings');items=wait_text('Stay available')
    if any(n.get('content-desc')=='Stay available' and n.get('checked')=='true' for n in items):
        activate('Stay available');wait_text('Off')
    availability_changed=False
    adb('shell','input','keyevent','4')
atexit.register(cleanup_availability)
adb('shell','am','force-stop','computer.extend.android.development')
adb('shell','am','start','-n','computer.extend.android.development/.MainActivity')
wait_text('No paired devices')
receiving_address=None
if args.receiving:
    activate('Settings'); items=wait_text('Stay available')
    control=next(n for n in items if n.get('content-desc')=='Stay available')
    assert control.get('checked')=='false', 'Test requires availability initially off; never changes an existing enabled service'
    availability_changed=True
    activate('Stay available')
    # Accept only this app's normal notification permission prompt if presented.
    deadline=time.monotonic()+30
    while time.monotonic()<deadline:
        items=nodes()
        if any(n.get('text')=='Available' for n in items): break
        if any(n.get('resource-id','').endswith(':id/permission_allow_button') for n in items):
            for _ in range(6):
                items=nodes()
                if any(n.get('resource-id','').endswith(':id/permission_allow_button') and n.get('focused')=='true' for n in items):
                    adb('shell','input','keyevent','66');break
                adb('shell','input','keyevent','61')
    else: raise RuntimeError('Availability did not start')
    receiving_address=receiving_endpoint()
    adb('shell','input','keyevent','4');wait_text('No paired devices')
activate('Show pairing code')
items=wait_text('Pairing code')
code=next(n.get('text') for n in items if re.fullmatch(r'[0-9a-f]{4}(?:-[0-9a-f]{4}){3}', n.get('text','')))
address=next(n.get('content-desc').removeprefix('Pairing address ') for n in items if n.get('content-desc','').startswith('Pairing address '))
fixture=args.output/'private-tablet.json'
fixture.write_text(json.dumps({'address':address,'code':code,'peer':args.tablet_peer, 'receivingAddress':receiving_address})); fixture.chmod(0o600)
receipt=args.output/'client.json'
client=subprocess.Popen(['target/debug/examples/tablet_pairing_client',str(fixture),str(receipt)],stdin=subprocess.PIPE,text=True)
try:
    items=wait_text('Allow pairing?')
    expected=json.loads(receipt.read_text())['clientPeer']
    assert any(n.get('text')=='Device ID: '+expected for n in items), 'Consent must identify the actual test peer'
    if args.decision != 'allow':
        if args.decision == 'deny':
            activate('Deny pairing'); wait_text("Pairing wasn't approved")
        adb('shell','input','keyevent','4')
        wait_text('No paired devices')
        client.wait(timeout=15); assert client.returncode != 0
        host,port=address.rsplit(':',1)
        try:
            with socket.create_connection((host,int(port)),timeout=2): pass
        except OSError: pass
        else: raise RuntimeError('Canceled listener remained open')
        (args.output/'result.json').write_text(json.dumps({'status':'passed','checks':['real-lan-incoming','correct-peer-id',args.decision+'-leaves-no-trust','close-stops-listener']},indent=2))
        print('PASS: incoming '+args.decision+' leaves no trust and closes the listener.')
        raise SystemExit(0)
    activate('Allow pairing')
    wait_text('Device paired')
    if args.receiving:
        adb('shell','input','keyevent','4');wait_text('Incoming Test Mac')
        closed(address)
        adb('shell','input','keyevent','3')
        time.sleep(2)
        client.stdin.write('probe\n');client.stdin.flush()
        deadline=time.monotonic()+15
        while time.monotonic()<deadline:
            if json.loads(receipt.read_text())['status']=='background-passed': break
            if client.poll() is not None: raise RuntimeError('Background authenticated reconnect failed')
            time.sleep(.1)
        else: raise RuntimeError('Background reconnect receipt missing')
        adb('shell','am','start','-n','computer.extend.android.development/.MainActivity');wait_text('Incoming Test Mac')
        adb('shell','am','force-stop','computer.extend.android.development')
        adb('shell','am','start','-n','computer.extend.android.development/.MainActivity');wait_text('Incoming Test Mac')
        receiving_address=receiving_endpoint()
        client.stdin.write(receiving_address+'\n');client.stdin.flush()
        deadline=time.monotonic()+20
        while time.monotonic()<deadline:
            if json.loads(receipt.read_text())['status']=='return-route-ready': break
            if client.poll() is not None: raise RuntimeError('Return-route fixture failed')
            time.sleep(.1)
        else: raise RuntimeError('Return-route fixture missing')
        # Code listener is already closed. Incoming peer has no manually entered address.
        activate('Check connection');wait_text('Connected successfully')
        # Native repaired route persists across a second fresh application process.
        adb('shell','am','force-stop','computer.extend.android.development')
        adb('shell','am','start','-n','computer.extend.android.development/.MainActivity');wait_text('Incoming Test Mac')
        activate('Check connection');wait_text('Connected successfully')
        client.stdin.write('route-checked\n');client.stdin.flush()
    client.wait(timeout=15); assert client.returncode==0
    assert json.loads(receipt.read_text())['status']=='passed'
    if args.receiving: adb('shell','am','start','-n','computer.extend.android.development/.MainActivity')
    else: adb('shell','input','keyevent','4')
    expected_name='Incoming Test Mac'
    wait_text(expected_name)
    # Closing the pairing screen must close the listener, even after a success.
    if not args.receiving:
        closed(address)
        adb('shell','am','force-stop','computer.extend.android.development')
        adb('shell','am','start','-n','computer.extend.android.development/.MainActivity');wait_text(expected_name)
    adb('shell','screencap','-p','/sdcard/extend-incoming-paired.png')
    adb('pull','/sdcard/extend-incoming-paired.png',str(args.output/'paired.png'))
    activate('Forget')
    items=wait_text('Forget device?')
    for _ in range(6):
        if any(n.get('resource-id')=='android:id/button1' and n.get('focused')=='true' for n in nodes()): break
        adb('shell','input','keyevent','61')
    else: raise RuntimeError('Forget confirmation focus missing')
    adb('shell','input','keyevent','66'); wait_text('No paired devices')
    if args.receiving:
        activate('Settings');wait_text('Available');activate('Stay available');wait_text('Off')
        closed(receiving_address);availability_changed=False
        adb('shell','input','keyevent','4');wait_text('No paired devices')
    (args.output/'result.json').write_text(json.dumps({'status':'passed','checks':['real-lan-incoming','explicit-consent-ui','correct-peer-id','pinned-reconnect','authenticated-peer-name','name-survives-restart','close-stops-listener','confirmed-forget']+(['background-reconnect','availability-restarts-with-app','availability-toggle-off','android-ui-return-route-repair','return-route-survives-restart'] if args.receiving else [])},indent=2))
    print('PASS: real incoming tablet UI, explicit consent, LAN reconnect, listener cleanup and local forgetting.')
finally:
    if client.poll() is None: client.terminate(); client.wait(timeout=5)
    fixture.unlink(missing_ok=True)
