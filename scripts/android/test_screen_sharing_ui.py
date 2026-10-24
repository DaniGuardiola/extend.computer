#!/usr/bin/env python3
"""Exercise real Android projection consent and pinned tablet-to-desktop media.

Requires an authorized physical tablet, a uniquely named paired desktop accepting
connections, the observer test APK, Devices visible, and that desktop's metrics log. Does not create
or remove pairings. Receiver pixels must additionally be inspected in its viewer;
this runner establishes fresh native capture/transport/renderer telemetry only.
"""
import argparse
import json
import re
import subprocess
import time
from pathlib import Path
from notification_ui import notification_step

ROOT = Path(__file__).resolve().parents[2]
APP = 'computer.extend.android.development'
OBSERVER = 'computer.extend.android.observer/computer.extend.android.tests.ObserverInstrumentation'
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--serial', required=True)
p.add_argument('--peer', required=True, help='Verified paired desktop fingerprint')
p.add_argument('--name', required=True, help='Unique existing paired desktop name')
p.add_argument('--desktop-log', type=Path, required=True, help='Live receiver metrics log on this host')
p.add_argument('--expect-route', choices=['lan','usb'], default='lan', help='Required native media transport')
p.add_argument('--expect-signaling-route', choices=['lan','usb'], default='lan', help='Required pinned display signaling route; USB uses the same captured carrier as media')
p.add_argument('--live-settings',action='store_true',help='Verify UI live quality, frame rate, resolution and rejected hardware settings')
p.add_argument('--rotation-check', action='store_true', help='Verify native portrait/landscape reconfiguration and restore rotation settings')
p.add_argument('--stop-control', choices=['app','notification'], default='app', help='Stop capture from the application or its foreground notification')
p.add_argument('--capture-scope', choices=['screen','app'], default='screen', help='Capture the entire screen or only the disposable engine-test activity')
p.add_argument('--codec', choices=['h264','hevc'], default='h264')
p.add_argument('--fps', type=int, choices=[30,60,120], default=60)
p.add_argument('--resolution', choices=['1280','1920','fit'], default='1280')
p.add_argument('--quality', choices=['responsive','balanced','detail'], default='balanced')
p.add_argument('--inspection-seconds', type=int, default=0, choices=range(31), help='Keep the verified viewer open for a bounded visual inspection')
p.add_argument('--output', type=Path, default=ROOT/'native/android/.local'/('screen-sharing-ui-'+time.strftime('%Y%m%d-%H%M%S')))
a = p.parse_args()
if a.expect_signaling_route != a.expect_route: p.error('Current adapters require matching signaling and media routes')
if a.rotation_check and a.capture_scope!='screen': p.error('--rotation-check currently requires full-screen capture')
if a.live_settings and (a.rotation_check or a.capture_scope!='screen' or a.resolution!='1280' or a.fps!=60 or a.quality!='balanced'): p.error('--live-settings requires baseline 1280/60/balanced full-screen capture without rotation')
original_rotation={}
assert re.fullmatch('[0-9a-f]{64}', a.peer), 'Invalid peer fingerprint'
out = a.output.resolve(); out.mkdir(parents=True, exist_ok=False)
base = ['adb', '-s', a.serial]

def adb(*args):
    return subprocess.check_output(base+list(args), text=True, timeout=25)

def nodes(package):
    raw = adb('shell', 'am', 'instrument', '-w', '-e', 'action', 'dump', '-e', 'package', package, OBSERVER)
    fields = dict(s.removeprefix('INSTRUMENTATION_RESULT: ').split('=', 1) for s in raw.splitlines() if s.startswith('INSTRUMENTATION_RESULT: ') and '=' in s)
    return json.loads(fields.get('nodes', '[]'))

def wait_text(package, text):
    deadline = time.monotonic()+30
    while time.monotonic()<deadline:
        items = nodes(package)
        if any(n['text']==text or n['description']==text for n in items): return items
    raise RuntimeError('Expected UI missing: '+text)

def click(package, label):
    subprocess.run(['python3', str(ROOT/'scripts/android/ui.py'), '--serial', a.serial, '--package', package, '--preserve-services', '--action', 'click', '--text', label], check=True, stdout=subprocess.DEVNULL)

def stop_from_notification():
    """Expanded notification RemoteViews belong to APP, not SystemUI."""
    operations = []
    deadline = time.monotonic()+30
    while time.monotonic()<deadline:
        own = nodes(APP)
        system = nodes('com.android.systemui')
        step = notification_step(own, system, 'Sharing your screen', 'Stop sharing')
        if step is None:
            continue
        if step['kind']=='action':
            (out/'notification-nodes.json').write_text(json.dumps({'own':own,'summary':[n for n in system if n['text']=='Sharing your screen'],'operations':operations},indent=2))
            click(APP,'Stop sharing')
            return
        if len(operations)>=4:
            raise RuntimeError('Capture notification action did not appear after own expansion')
        left, top, right, bottom = map(int, step['node']['bounds'].split())
        subprocess.run(['python3',str(ROOT/'scripts/android/ui.py'),'--serial',a.serial,'--package','com.android.systemui','--preserve-services','--action','gesture','--gesture','tap','--start',str((left+right)/2),str((top+bottom)/2)],check=True,stdout=subprocess.DEVNULL)
        operations.append({'kind':'expand','resource':step['node']['resource'],'bounds':step['node']['bounds']})
    raise RuntimeError('Expected capture notification action missing')

assert a.desktop_log.is_file(), 'Receiver log unavailable'
offset = a.desktop_log.stat().st_size
previous_capture_lines = set(adb('logcat', '-d', '-v', 'time', '-s', 'ExtendCapture:I', '*:S').splitlines())
adb('shell', 'am', 'start', '-n', APP+'/.MainActivity')
items = wait_text(APP, 'Devices')
assert len([n for n in items if n['text']==a.name])==1, 'Paired device name must be unique'
click(APP, 'Mirror display to '+a.name)
items=wait_text(APP,'Start sharing')
original_choices=[n['description'] for n in items if n['checked'] and any(n['description'].startswith(prefix+' ') for prefix in ['Resolution','Frame rate','Video quality','Video codec'])]
assert len(original_choices)==4, 'Original sharing preferences missing'
(out/'original-choices.json').write_text(json.dumps(original_choices))
def run_capture():
    expected_choices=['Resolution '+{'1280':'1280 px','1920':'1920 px','fit':'Fit screen'}[a.resolution],f'Frame rate {a.fps} FPS','Video quality '+{'responsive':'Lower bandwidth · 8 Mbps','balanced':'Balanced · 20 Mbps','detail':'Detail · 40 Mbps'}[a.quality],'Video codec '+{'h264':'H.264','hevc':'HEVC'}[a.codec]]
    for label in expected_choices: click(APP,label)
    items=nodes(APP);assert all(any(n['description']==label and n['checked'] for n in items) for label in expected_choices)
    click(APP, 'Start sharing')
    wait_text('com.android.systemui', 'Start recording or casting with extend.computer?')
    click('com.android.systemui', 'Cancel')
    wait_text(APP, 'Screen sharing is off')
    assert APP not in adb('shell', 'dumpsys', 'media_projection'), 'Canceled consent started capture'
    click(APP, 'Start sharing')
    wait_text('com.android.systemui', 'Start recording or casting with extend.computer?')
    if a.capture_scope=='screen':
        click('com.android.systemui', 'Share one app')
        wait_text('com.android.systemui', 'Share entire screen')
        click('com.android.systemui', 'Share entire screen')
    wait_text('com.android.systemui', 'Share screen' if a.capture_scope=='screen' else 'Next')
    started = False
    try:
        click('com.android.systemui', 'Share screen' if a.capture_scope=='screen' else 'Next'); started = True
        if a.capture_scope=='app':
            picker=wait_text('com.android.systemui','Choose app to share')
            (out/'picker-nodes.json').write_text(json.dumps(picker,indent=2))
            # Stop must cancel the pending selector, not only a running foreground service.
            adb('shell','am','start','-n',APP+'/.MainActivity')
            deadline=time.monotonic()+15
            while time.monotonic()<deadline:
                controls=nodes(APP)
                if any(n['text']=='Screen sharing is off' for n in controls): break
                if any(n['description']=='Stop sharing' for n in controls):
                    click(APP,'Stop sharing');wait_text(APP,'Screen sharing is off');break
            else: raise RuntimeError('Pending app selection did not cancel')
            assert APP not in adb('shell','dumpsys','media_projection')
            click(APP,'Start sharing')
            wait_text('com.android.systemui','Start recording or casting with extend.computer?')
            click('com.android.systemui','Next')
            picker=wait_text('com.android.systemui','Choose app to share')
            (out/'fresh-picker-nodes.json').write_text(json.dumps(picker,indent=2))
            wait_text('com.android.systemui','Extend engine tests')
            click('com.android.systemui','Extend engine tests')
            deadline=time.monotonic()+15
            while APP not in adb('shell','dumpsys','media_projection') and time.monotonic()<deadline: time.sleep(.2)
        else: wait_text(APP, 'Sharing your screen')
        projection = adb('shell', 'dumpsys', 'media_projection')
        assert APP in projection
        (out/'active-projection.txt').write_text(projection)
        # Leave the application while sharing its real screen, using only our disposable fixture.
        adb('shell', 'am', 'start', '-n', 'computer.extend.android.enginetest/computer.extend.android.tests.ReceiverTargetActivity')
        deadline = time.monotonic()+20
        stats = None
        while time.monotonic()<deadline:
            capture = adb('logcat', '-d', '-v', 'time', '-s', 'ExtendCapture:I', '*:S')
            candidates = []
            for line in capture.splitlines():
                if 'Capture stats: ' in line and line not in previous_capture_lines:
                    value = json.loads(line.split('Capture stats: ',1)[1])
                    if value.get('peer')==a.peer and value.get('state')=='streaming': candidates.append(value)
            with a.desktop_log.open('rb') as f: f.seek(offset); receiver=f.read().decode(errors='replace')
            if candidates and candidates[-1].get('encoded',0)>0 and candidates[-1]['transport'].get('sent',0)>0 and re.search(r'assembled=[1-9]\d*', receiver):
                stats=candidates[-1]; break
            time.sleep(.3)
        assert stats is not None, 'Fresh source and receiver frame evidence missing'
        assert stats['transport']['peer']==a.peer
        assert stats['transport'].get('route')==a.expect_route, 'Native capture used unexpected media route'
        # The native sender sets this route after opening its pinned signaling
        # socket and allocating media from that same captured carrier. Streaming
        # requires successful exact-peer authentication and the remote answer;
        # raw accessory readiness alone cannot satisfy this gate.
        assert stats['transport'].get('route')==a.expect_signaling_route, 'Native capture used unexpected signaling route'
        signaling_proof={'route':a.expect_signaling_route,'peer':a.peer,'session_id':stats['transport']['id'],'state':stats['transport']['state'],'evidence':'Fresh pinned native sender session; signaling and media use the same captured route'}
        (out/'signaling-proof.json').write_text(json.dumps(signaling_proof,indent=2))
        if a.expect_route=='usb': assert 'Display media transport: usb' in receiver, 'Desktop did not select USB media'
        config=stats['transport']['config'];assert config['codec']==a.codec and config['fps']==a.fps
        assert config['bitrate']<= {'responsive':8000000,'balanced':20000000,'detail':40000000}[a.quality]
        assert 'Display video decoded and presented by Metal' in receiver
        assert APP in adb('shell', 'dumpsys', 'media_projection'), 'Background capture stopped'
        (out/'capture.log').write_text(capture); (out/'receiver.log').write_text(receiver); (out/'stats.json').write_text(json.dumps(stats,indent=2))
        if a.live_settings:
            adb('shell','am','start','-n',APP+'/.MainActivity')
            wait_text(APP,'Sharing your screen')
            before=stats
            updates=[('Video quality Lower bandwidth · 8 Mbps',{'bitrate':8000000}),('Frame rate 30 FPS',{'fps':30}),('Resolution 1920 px',{'width':1920,'height':1200})]
            for index,(label,expected) in enumerate(updates):
                click(APP,label)
                deadline=time.monotonic()+20;updated=None
                while time.monotonic()<deadline:
                    lines=adb('logcat','-d','-v','time','-s','ExtendCapture:I','*:S')
                    for line in lines.splitlines():
                        if 'Capture stats: ' not in line or line in previous_capture_lines: continue
                        value=json.loads(line.split('Capture stats: ',1)[1]);t=value.get('transport',{})
                        if value.get('peer')==a.peer and value.get('state')=='streaming' and t.get('id')==before['transport']['id'] and t.get('revision',0)>before['transport'].get('revision',0) and t.get('sent_revision',0)>0 and all(t.get('config',{}).get(k)==v for k,v in expected.items()): updated=value
                    if updated: break
                    time.sleep(.2)
                assert updated is not None,'Live settings did not negotiate and send fresh frames: '+label
                controls=nodes(APP);assert any(n['description']==label and n['checked'] for n in controls)
                (out/f'live-settings-{index}.json').write_text(json.dumps(updated,indent=2));before=updated
            click(APP,'Frame rate 120 FPS')
            deadline=time.monotonic()+15
            while time.monotonic()<deadline:
                controls=nodes(APP)
                if any('Hardware encoding doesn\'t support' in n['text'] for n in controls): break
            else: raise RuntimeError('Unsupported live profile did not report its hardware rejection')
            assert any(n['description']=='Frame rate 30 FPS' and n['checked'] for n in controls)
            assert not any(n['description']=='Frame rate 120 FPS' and n['checked'] for n in controls)
            assert APP in adb('shell','dumpsys','media_projection'),'Rejected profile ended capture'
            (out/'live-settings-rejected-nodes.json').write_text(json.dumps(controls,indent=2))
            expected_choices=['Resolution 1920 px','Frame rate 30 FPS','Video quality Lower bandwidth · 8 Mbps','Video codec '+{'h264':'H.264','hevc':'HEVC'}[a.codec]]
            assert all(any(n['description']==label and n['checked'] for n in controls) for label in expected_choices)
            adb('shell','am','start','-n','computer.extend.android.enginetest/computer.extend.android.tests.ReceiverTargetActivity')
        if a.rotation_check:
            for setting in ['accelerometer_rotation','user_rotation']:
                original_rotation[setting]=adb('shell','settings','get','system',setting).strip()
            (out/'original-rotation.json').write_text(json.dumps(original_rotation))
            adb('shell','settings','put','system','accelerometer_rotation','0')
            before=stats
            for rotation in [0,1]:
                adb('shell','settings','put','system','user_rotation',str(rotation))
                deadline=time.monotonic()+20
                rotated=None
                while time.monotonic()<deadline:
                    capture=adb('logcat','-d','-v','time','-s','ExtendCapture:I','*:S')
                    candidates=[]
                    for line in capture.splitlines():
                        if 'Capture stats: ' not in line or line in previous_capture_lines: continue
                        value=json.loads(line.split('Capture stats: ',1)[1]);t=value.get('transport',{});c=t.get('config',{})
                        correct_shape=(c.get('height',0)>c.get('width',0)) if rotation==0 else (c.get('width',0)>c.get('height',0))
                        if value.get('peer')==a.peer and value.get('state')=='streaming' and t.get('id')==before['transport']['id'] and correct_shape and t.get('revision',0)>before['transport'].get('revision',0) and t.get('sent_revision',0)>0 and t.get('sent',0)>before['transport']['sent']: candidates.append(value)
                    if candidates: rotated=candidates[-1];break
                    time.sleep(.2)
                assert rotated is not None,'Rotation did not renegotiate and send fresh frames'
                (out/f'rotation-{rotation}-stats.json').write_text(json.dumps(rotated,indent=2))
                (out/f'rotation-{rotation}-capture.log').write_text(capture)
                with a.desktop_log.open('rb') as f: f.seek(offset); (out/f'rotation-{rotation}-receiver.log').write_text(f.read().decode(errors='replace'))
                before=rotated
                print('ROTATION READY:',rotation,out,flush=True)
                if a.inspection_seconds: time.sleep(a.inspection_seconds)
        elif a.inspection_seconds:
            print('CAPTURE READY:',a.capture_scope,out,flush=True)
            time.sleep(a.inspection_seconds)
        if a.stop_control=='notification':
            adb('shell','cmd','statusbar','expand-notifications')
            try:
                stop_from_notification()
            finally: adb('shell','cmd','statusbar','collapse')
            adb('shell','am','start','-n',APP+'/.MainActivity')
        else:
            adb('shell', 'am', 'start', '-n', APP+'/.MainActivity'); click(APP, 'Stop sharing')
        wait_text(APP, 'Screen sharing is off'); started=False
        projection=adb('shell','dumpsys','media_projection'); assert APP not in projection
        (out/'stopped-projection.txt').write_text(projection)
        services=adb('shell','dumpsys','activity','services',APP)
        assert not re.search(r'ServiceRecord\{[^\n]*ScreenSharingService',services), 'Capture service still running'
        adb('shell','input','keyevent','4'); items=wait_text(APP,'Devices');assert any(n['text']==a.name for n in items)
        click(APP,'Mirror display to '+a.name);items=wait_text(APP,'Start sharing')
        assert all(any(n['description']==label and n['checked'] for n in items) for label in expected_choices), 'Sharing preferences did not survive navigation'
        for label in original_choices: click(APP,label)
        items=nodes(APP);assert all(any(n['description']==label and n['checked'] for n in items) for label in original_choices), 'Original sharing preferences were not restored'
        adb('shell','input','keyevent','4');wait_text(APP,'Devices')
        (out/'result.json').write_text(json.dumps({'status':'passed','checks':['system-consent-cancel','new-consent-after-cancel','native-hardware-encoding','pinned-native-transport','fresh-desktop-decoded-frame-telemetry','background-projection',a.stop_control+'-stop-projection-cleanup','capture-service-stopped','existing-pairing-preserved','codec-and-fps-negotiated','per-device-preferences-persisted','original-preferences-restored'],'rotation_checked':a.rotation_check,'live_settings_checked':a.live_settings,'stop_control':a.stop_control,'pending_selector_return_cleanup_verified':a.capture_scope=='app','capture_scope':a.capture_scope,'media_route':a.expect_route,'signaling_route':a.expect_signaling_route,'signaling_proof':signaling_proof,'configuration':{'codec':a.codec,'fps':a.fps,'resolution':a.resolution,'quality':a.quality},'visual_pixel_gate':'Requires separate receiver screenshot inspection'},indent=2))
        print('PASS: Android screen-sharing lifecycle and native media.',out)
    finally:
        if started:
            adb('shell','am','start','-n',APP+'/.MainActivity')
            if any(n['text']=='Stop sharing' for n in nodes(APP)): click(APP,'Stop sharing')


def restore_after_failure():
    system=nodes('com.android.systemui')
    if any(n['text']=='Start recording or casting with extend.computer?' for n in system):
        click('com.android.systemui','Cancel')
    adb('shell','am','start','-n',APP+'/.MainActivity')
    items=nodes(APP)
    if any(n['description']=='Stop sharing' for n in items):
        click(APP,'Stop sharing');items=wait_text(APP,'Screen sharing is off')
    if any(n['text']=='Devices' for n in items):
        click(APP,'Mirror display to '+a.name);items=wait_text(APP,'Start sharing')
    assert any(n['text']=='Start sharing' for n in items), 'Return to sharing controls to restore preferences'
    for label in original_choices: click(APP,label)
    items=nodes(APP);assert all(any(n['description']==label and n['checked'] for n in items) for label in original_choices)
    assert APP not in adb('shell','dumpsys','media_projection')
    adb('shell','input','keyevent','4');wait_text(APP,'Devices')
    (out/'failure-cleanup.json').write_text(json.dumps({'preferences_restored':True,'projection_stopped':True}))

try:
    run_capture()
except Exception as failure:
    (out/'failure.json').write_text(json.dumps({'status':'failed','error':str(failure)}))
    (out/'failure-app-nodes.json').write_text(json.dumps(nodes(APP),indent=2))
    try: restore_after_failure()
    except Exception as cleanup: (out/'cleanup-error.txt').write_text(str(cleanup))
    raise

finally:
    if original_rotation:
        for setting,value in original_rotation.items():
            if value=='null': adb('shell','settings','delete','system',setting)
            else: adb('shell','settings','put','system',setting,value)
        restored={setting:adb('shell','settings','get','system',setting).strip() for setting in original_rotation}
        assert restored==original_rotation,'Original rotation settings were not restored'
        (out/'restored-rotation.json').write_text(json.dumps(restored))
