#!/usr/bin/env python3
"""Verify pinned LAN video with generated patterns on a physical Android tablet.

Requires availability enabled and Devices visible. Preserves existing pairings;
only its freshly created Media Test Mac peer is removed. Build the documented
pattern generator and tablet_media_source example before running this script.
"""
import argparse, subprocess, time, json, re, os, sys
import xml.etree.ElementTree as E
from pathlib import Path
root=Path(__file__).resolve().parents[2]
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--serial',required=True)
p.add_argument('--image-python',default=sys.executable,help='Python with Pillow for screenshot validation')
p.add_argument('--tablet-peer',required=True,help='Physically verified tablet fingerprint')
p.add_argument('--resize',action='store_true',help='Verify live 1280x800 to 960x600 reconfiguration with both codecs')
p.add_argument('--cursor',action='store_true',help='Verify native separate cursor before video, moving while video is paused, and hidden')
p.add_argument('--native-usb',action='store_true',help='Verify native accessory bulk media; pairing/signaling still use LAN')
p.add_argument('--usb-device-serial',help='Exact physical USB serial, separate from wireless control serial')
p.add_argument('--usb-vendor',help='Physical device vendor in hexadecimal')
p.add_argument('--usb-helper',type=Path,default=root/'native/android/.local/aoa-host')
p.add_argument('--preserve-name',action='append',default=[],help='Existing device name required to remain after cleanup')
p.add_argument('--output',type=Path,default=root/'native/android/.local'/('media-ui-'+time.strftime('%Y%m%d-%H%M%S')))
args=p.parse_args();os.chdir(root)
if args.cursor and args.resize:p.error('--cursor and --resize currently require separate runs')
subprocess.run([args.image_python,'-c','from PIL import Image'],check=True)
assert re.fullmatch('[0-9a-f]{64}',args.tablet_peer), 'Invalid tablet pin'
if args.native_usb:
    assert args.usb_device_serial and args.usb_vendor and args.usb_helper.is_file(), 'Build native accessory helper and provide exact USB vendor/serial'
base=['adb','-s',args.serial]
def adb(*items):
    return subprocess.check_output(base + list(items), text=True, timeout=20)
def usb_mode():
    # Android svc prints this result to stderr, not stdout.
    return subprocess.check_output(base+['shell','svc','usb','getFunctions'],text=True,stderr=subprocess.STDOUT,timeout=20).strip()

def nodes():
    adb('shell', 'uiautomator', 'dump', '/sdcard/extend-incoming-ui.xml')
    return list(E.fromstring(adb('shell', 'cat', '/sdcard/extend-incoming-ui.xml')).iter('node'))

def wait_text(text):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        items = nodes()
        if any((n.get('text') == text for n in items)):
            return items
    raise RuntimeError('Expected UI text missing: ' + text)

def activate(label):
    for _ in range(12):
        items = nodes()
        if any((n.get('content-desc') == label and n.get('focused') == 'true' for n in items)):
            adb('shell', 'input', 'keyevent', '66')
            return
        adb('shell', 'input', 'keyevent', '61')
    raise RuntimeError('Could not focus ' + label)

out=args.output.resolve();out.mkdir(parents=True,exist_ok=False)
usb_functions=None
if args.native_usb:
    assert adb('shell','getprop','ro.serialno').strip()==args.usb_device_serial, 'Control device must match physical USB serial'
    usb_functions=usb_mode()
    assert usb_functions in ('mtp','ptp','midi','rndis','ncm',''), 'Accessory test must start in a restorable normal USB mode'
    (out/'usb-baseline.json').write_text(json.dumps({'functions':usb_functions,'sys_usb_config':adb('shell','getprop','sys.usb.config').strip()}))
items=wait_text('Show pairing code')
assert not any(n.get('text')=='Media Test Mac' for n in items), 'Temporary device name already exists; never removes an existing peer'
for name in args.preserve_name:assert any(n.get('text')==name for n in items), 'Existing peer missing: '+name
activate('Show pairing code');items=wait_text('Pairing code')
code=next(n.get('text') for n in items if re.fullmatch(r'[0-9a-f]{4}(?:-[0-9a-f]{4}){3}',n.get('text','')))
address=next(n.get('content-desc').removeprefix('Pairing address ') for n in items if n.get('content-desc','').startswith('Pairing address '))
fixture_data={'code':code,'address':address,'peer':args.tablet_peer}
if args.cursor:
    fixture_data['cursor_png']=json.loads(subprocess.check_output([args.image_python,'scripts/android/verify_media_cursor.py','--generate'],text=True))['png']
fixture=out/'private-fixture.json';fixture.write_text(json.dumps(fixture_data));fixture.chmod(0o600)
receipt=out/'source.json';log=(out/'source.log').open('w')
source_args=['target/debug/examples/tablet_media_source',str(fixture),str(receipt),'native/android/.local/media-pattern.h264.frames','native/android/.local/media-pattern.hevc.frames']
if args.resize: source_args += ['native/android/.local/media-pattern-960.h264.frames','native/android/.local/media-pattern-960.hevc.frames']
source_env=os.environ.copy()
if args.native_usb:
    source_env.update(EXTEND_NATIVE_USB_HOST=str(args.usb_helper.resolve()),EXTEND_NATIVE_USB_VENDOR=args.usb_vendor,EXTEND_NATIVE_USB_SERIAL=args.usb_device_serial)
source=subprocess.Popen(source_args,stdin=subprocess.PIPE,stdout=log,stderr=log,text=True,env=source_env)
def status(expected):
    deadline=time.monotonic()+(40 if expected.startswith('usb-') else 20)
    while time.monotonic()<deadline:
        try:
            if json.loads(receipt.read_text())['status']==expected:return
        except (FileNotFoundError,json.JSONDecodeError):pass
        if source.poll() is not None:raise RuntimeError('Source ended: '+(out/'source.log').read_text())
        time.sleep(.1)
    raise RuntimeError('Source status missing: '+expected)
def command(value):
    source.stdin.write(value+'\n');source.stdin.flush()
def cursor_stage(codec,stage,sample=''):
    status(codec+'-cursor-'+stage)
    value=json.loads(receipt.read_text())
    artifact=codec+'-cursor-'+stage+('-'+sample if sample else '')
    stage_receipt=out/(artifact+'-source.json');stage_receipt.write_text(json.dumps(value,indent=2))
    screenshot=out/(artifact+'.png')
    deadline=time.monotonic()+12
    while time.monotonic()<deadline:
        adb('shell','screencap','-p','/sdcard/extend-media-cursor.png');adb('pull','/sdcard/extend-media-cursor.png',str(screenshot))
        result=subprocess.run([args.image_python,'scripts/android/verify_media_cursor.py',str(screenshot),'--source',str(stage_receipt)],capture_output=True,text=True)
        if result.returncode==0:break
        time.sleep(.2)
    else:raise RuntimeError('Native cursor screenshot failed: '+result.stderr)
    (out/(artifact+'-pixels.json')).write_text(result.stdout)
    logs=adb('logcat','-d','-v','time','-s','ExtendCursor:I','*:S')
    (out/(artifact+'.log')).write_text(logs)
    if value['cursor']['visible']:
        assert re.search(r'Cursor drawn: revision='+str(value['cursor']['revision'])+r'\b',logs), 'Native cursor draw record missing'
    assert json.loads(receipt.read_text())['sent']==value['sent'], 'Video must remain paused during cursor screenshot'
    return value
def forget_temporary_peer():
    # Target only the temporary named card; never the real Mac's Forget button.
    items=nodes();names=[n for n in items if n.get('text')=='Media Test Mac'];assert len(names)==1
    bounds=lambda n:list(map(int,re.findall(r'\d+',n.get('bounds',''))))
    y=bounds(names[0])[1]
    choices=sorted([n for n in items if n.get('content-desc')=='Forget' and bounds(n)[1]>y],key=lambda n:bounds(n)[1]);assert choices
    for name in args.preserve_name:
        existing=next(n for n in items if n.get('text')==name)
        assert bounds(existing)[1]<y or bounds(choices[0])[1]<bounds(existing)[1], 'Target must remain inside temporary peer row'
    target=choices[0].get('bounds')
    for _ in range(24):
        items=nodes()
        if any(n.get('content-desc')=='Forget' and n.get('bounds')==target and n.get('focused')=='true' for n in items):adb('shell','input','keyevent','66');break
        adb('shell','input','keyevent','61')
    else:raise RuntimeError('Temporary peer Forget focus missing')
    wait_text('Forget device?')
    for _ in range(8):
        items=nodes()
        if any(n.get('resource-id')=='android:id/button1' and n.get('focused')=='true' for n in items):adb('shell','input','keyevent','66');break
        adb('shell','input','keyevent','61')
    else:raise RuntimeError('Temporary peer confirmation focus missing')
    time.sleep(.5);items=nodes();assert not any(n.get('text')=='Media Test Mac' for n in items);assert all(any(n.get('text')==name for n in items) for name in args.preserve_name)

temporary_peer_owned=False
try:
    items=wait_text('Allow pairing?');peer=json.loads(receipt.read_text())['peer'];assert any(n.get('text')=='Device ID: '+peer for n in items)
    activate('Allow pairing');temporary_peer_owned=True;wait_text('Device paired');adb('shell','input','keyevent','4');wait_text('Show pairing code');status('paired')
    if args.native_usb:
        source.stdin.write('usb\n');source.stdin.flush()
        status('usb-connected')
        deadline=time.monotonic()+35
        while time.monotonic()<deadline:
            logs=adb('logcat','-d','-v','time','-s','ExtendUsb:I','*:S')
            ready=[]
            for line in logs.splitlines():
                if 'Accessory ready: ' in line:
                    value=json.loads(line.split('Accessory ready: ',1)[1])
                    if value.get('peer')==peer and value.get('state')=='connected':ready.append(value)
            if ready:
                (out/'usb-attached.json').write_text(json.dumps(ready[-1],indent=2));break
            items=nodes();text=' '.join(n.get('text','')+' '+n.get('content-desc','') for n in items).lower()
            if 'extend.computer' in text and ('usb accessory' in text or 'usb access' in text or 'connected usb' in text or 'encrypted display transport' in text):
                buttons=[n for n in items if n.get('resource-id')=='android:id/button1' and n.get('text','').lower() in ('ok','allow','open')]
                if buttons:
                    def click_node(node):
                        subprocess.run([sys.executable,'scripts/android/ui.py','--serial',args.serial,'--package',node.get('package'),'--preserve-services','--action','click','--text',node.get('text')],check=True)
                    defaults=[n for n in items if n.get('resource-id')=='android:id/alwaysUse' and 'extend.computer' in n.get('text','') and n.get('checked')=='false']
                    (out/'usb-permission-nodes.json').write_text(json.dumps([dict(n.attrib) for n in items],indent=2))
                    if defaults: click_node(defaults[0])
                    click_node(buttons[0])
            if source.poll() is not None:raise RuntimeError('Native USB source ended: '+(out/'source.log').read_text())
            time.sleep(.2)
        else:raise RuntimeError('Own USB accessory permission/worker not ready')
        source.stdin.write('usb-ready\n');source.stdin.flush();wait_text('Show pairing code')
    for codec,expected in [('h264','h264-sent'),('hevc','passed')]:
        cursor_log_start=None
        if args.cursor:
            # Snapshot log boundary without clearing unrelated device evidence.
            cursor_log_start=adb('logcat','-d','-v','time','-s','ExtendCursor:I','*:S')
        command(codec)
        if args.cursor:
            initial=cursor_stage(codec,'initial');assert initial['sent']==0 and initial['video_paused']
            command('cursor-video');status(codec+'-cursor-video')
        time.sleep(10.5 if args.resize else 4.5)
        adb('shell','screencap','-p','/sdcard/extend-media-pattern.png');adb('pull','/sdcard/extend-media-pattern.png',str(out/(codec+'.png')))
        pixels=subprocess.check_output([args.image_python,'scripts/android/verify_media_pattern.py',str(out/(codec+'.png'))],text=True)
        (out/(codec+'-pixels.json')).write_text(pixels)
        logs=adb('logcat','-d','-v','time','-s','ExtendMedia:I','ExtendDisplay:E','*:S');(out/(codec+'.log')).write_text(logs)
        stats=[]
        for line in logs.splitlines():
            if 'Stream stats: ' in line:
                value=json.loads(line.split('Stream stats: ',1)[1])
                if value['peer']==peer and value['config']['codec']==codec:stats.append(value)
        assert stats and max(s['rendered'] for s in stats)>=120, 'At least 120 actual decoded/rendered frames required: '+codec
        if args.native_usb:
            stats=[s for s in stats if s.get('route')=='usb'];assert stats and max(s['rendered'] for s in stats)>=120,'Actual native USB media route required'
        if args.resize:
            stats=[s for s in stats if s.get('revision')==1 and s.get('rendered_revision',0)>=120 and s['config']['width']==960 and s['config']['height']==600 and s['config']['bitrate']==4000000]
            assert stats, 'Live configuration did not produce 120 rendered frames under new geometry'
        latest=max(stats,key=lambda s:s['rendered'])
        assert latest['rendered_timestamp_ns']>0 and latest['render_device_ns']>0
        assert latest['render_clock_valid'], 'Codec render clock differs from tablet clock: '+codec
        assert latest['render_device_ns']<=latest['callback_device_ns']
        assert latest['received_timestamp_ns']>=latest['rendered_timestamp_ns']
        (out/(codec+'-timing.json')).write_text(json.dumps(latest,indent=2))
        if args.cursor:
            assert latest['config']['separate_cursor'] and latest.get('cursor_received',0)>0 and latest.get('cursor_delivered',0)>0, 'Native separate cursor must be negotiated and consumed'
            command('cursor-move');paused=cursor_stage(codec,'paused');assert paused['video_paused'] and paused['sent']>=120
            time.sleep(1)
            paused_again=cursor_stage(codec,'paused','again');assert paused_again['sent']==paused['sent'], 'Paused cursor movement cannot depend on fresh video'
            command('cursor-hide');hidden=cursor_stage(codec,'hidden');assert not hidden['cursor']['visible'] and hidden['sent']==paused['sent']
            new_cursor_logs=adb('logcat','-d','-v','time','-s','ExtendCursor:I','*:S')
            old_lines=set(cursor_log_start.splitlines())
            fresh='\n'.join(line for line in new_cursor_logs.splitlines() if line not in old_lines)
            assert all(re.search(r'Cursor drawn: revision='+str(revision)+r'\b',fresh) for revision in (1,2)), 'Fresh initial and paused-video native cursor draws required'
            (out/(codec+'-cursor-fresh.log')).write_text(fresh)
            command('cursor-finish')
        status(expected)
        adb('shell','input','keyevent','4');wait_text('Show pairing code')
    source.wait(timeout=10);assert source.returncode==0
    forget_temporary_peer();temporary_peer_owned=False
    source_result=json.loads(receipt.read_text())
    if args.native_usb:assert source_result['usb']['sent_packets']>100 and source_result['usb']['received_packets']>0 and source_result['usb']['error']==''
    checks=['pinned-media-signal','hardware-avc-decode','hardware-hevc-decode','120-rendered-frames-per-codec','verified-pattern-pixels','native-render-clock-sanity','tablet-viewer-auto-open','back-stops-session','temporary-peer-only-forgotten','existing-pairings-preserved']
    if args.cursor:checks+=['separate-cursor-before-video','native-cursor-pixels-and-hotspot','native-cursor-moves-with-video-paused','hidden-cursor-absent','native-cursor-draw-log']
    (out/'result.json').write_text(json.dumps({'status':'passed','native_usb_media_checked':args.native_usb,'signaling_route':'lan','live_configuration_checked':args.resize,'separate_cursor_checked':args.cursor,'checks':checks,'source':source_result},indent=2))
    print('PASS: native encrypted media, AVC/HEVC decoding and viewer cleanup.',out)
finally:
    if source.poll() is None:source.terminate();source.wait(timeout=5)
    fixture.unlink(missing_ok=True)
    log.close()
    if usb_functions is not None:
        adb('shell','svc','usb','setFunctions',usb_functions)
        time.sleep(2)
        restored=usb_mode()
        assert restored==usb_functions, 'Original USB function not restored'
        (out/'usb-restored.json').write_text(json.dumps({'functions':restored,'sys_usb_config':adb('shell','getprop','sys.usb.config').strip()}))
    if temporary_peer_owned:
        try:
            adb('shell','am','start','-n','computer.extend.android.development/.MainActivity')
            wait_text('Show pairing code')
            forget_temporary_peer()
            (out/'failure-cleanup.json').write_text(json.dumps({'temporary_peer':peer,'removed':True,'preserved_names':args.preserve_name}))
        except Exception as error:
            (out/'failure-cleanup.json').write_text(json.dumps({'temporary_peer':peer,'removed':False,'error':str(error)}))
