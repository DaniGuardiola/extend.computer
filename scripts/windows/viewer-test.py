"""Drives ExtendComputerDisplay.exe viewer the way the desktop runtime does and checks its replies.

Usage: viewer-test.py <ExtendComputerDisplay.exe> [width height]

Runs in the signed-in user's desktop session (viewer-test.ps1); the viewer opens full screen on the
primary monitor. The H.264 stream comes from `self-test stream`, so no screen is captured. Checks:
ready, a full-screen viewport in physical pixels at scale 1, settings answered, render readiness and
presented counts while frames play at 60 fps, a keyframe request after an unreadable frame, a click and
typing in the window as kind 4 input (printable keys as text only, Ctrl+C as Command+C, Enter as a
physical key), closing the window, and exit on stop, on end of input and when its parent exits.
"""
import ctypes
import json
import queue
import struct
import subprocess
import sys
import threading
import time
from ctypes import wintypes

exe = sys.argv[1]
width, height = (int(sys.argv[2]), int(sys.argv[3])) if len(sys.argv) > 3 else (1920, 1080)
failures = []
user32 = ctypes.WinDLL('user32', use_last_error=True)
user32.SetProcessDPIAware()


def check(ok, name, detail=''):
    print(f"{'ok' if ok else 'FAIL'} {name}{': ' + detail if detail else ''}", flush=True)
    if not ok:
        failures.append(name)


def packet(kind, payload):
    return bytes([kind]) + struct.pack('>I', len(payload)) + payload


def control(message):
    return packet(2, json.dumps(message, separators=(',', ':')).encode())


def packets(data):
    i = 0
    while i + 5 <= len(data):
        kind, size = data[i], struct.unpack('>I', data[i + 1:i + 5])[0]
        yield kind, data[i + 5:i + 5 + size]
        i += 5 + size


class Viewer:
    """A viewer process with framed pipes. `wrap` starts it through cmd, which the test can kill."""

    def __init__(self, wrap=False):
        args = [exe, 'viewer', str(width), str(height), '60', '12000000', 'standard', '0', 'h264', 'embedded']
        self.process = subprocess.Popen((['cmd', '/c'] if wrap else []) + args, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.messages = queue.Queue()
        self.closed = False
        self.stderr = []
        threading.Thread(target=self._read, daemon=True).start()
        threading.Thread(target=lambda: self.stderr.extend(
            line.decode(errors='replace').rstrip() for line in self.process.stderr), daemon=True).start()

    def _read(self):
        stream = self.process.stdout
        while True:
            header = stream.read(5)
            if len(header) < 5:
                self.messages.put((time.monotonic(), None, None))
                return
            payload = stream.read(struct.unpack('>I', header[1:])[0])
            self.messages.put((time.monotonic(), header[0], payload))

    def send(self, data):
        self.process.stdin.write(data)
        self.process.stdin.flush()

    def wait(self, accept, timeout):
        """The first message `accept` takes within `timeout`; kind 0, 2 and 4 payloads arrive as JSON."""
        deadline = time.monotonic() + timeout
        while (left := deadline - time.monotonic()) > 0:
            try:
                _, kind, payload = self.messages.get(timeout=left)
            except queue.Empty:
                break
            if kind is None:
                self.closed = True
                break
            message = json.loads(payload) if kind in (0, 2, 4) else payload
            if accept(kind, message):
                return message
        return None

    def drain(self):
        drained = []
        while not self.messages.empty():
            _, kind, payload = self.messages.get()
            if kind is None:
                self.closed = True
            elif kind in (0, 2, 4):
                drained.append((kind, json.loads(payload)))
        return drained

    def exited(self, timeout):
        """Seconds until the viewer closed its stdout, or None."""
        started = time.monotonic()
        while not self.closed and time.monotonic() - started < timeout:
            self.wait(lambda kind, _: False, timeout - (time.monotonic() - started))
        return time.monotonic() - started if self.closed else None

    def child(self):
        """The viewer's process ID when cmd started it."""
        found = subprocess.run(['powershell', '-NoProfile', '-Command',
                                f'(Get-CimInstance Win32_Process -Filter "ParentProcessId={self.process.pid}").ProcessId'],
                               capture_output=True, text=True).stdout.split()
        return int(found[0]) if found else None

    def kill(self, pid=None):
        subprocess.run(['taskkill', '/F', '/T', '/PID', str(pid or self.process.pid)], capture_output=True)


def window():
    return user32.FindWindowW('ExtendComputerViewer', None)


# --- SendInput, for the input checks.
class MOUSEINPUT(ctypes.Structure):
    _fields_ = [('dx', wintypes.LONG), ('dy', wintypes.LONG), ('mouseData', wintypes.DWORD),
                ('dwFlags', wintypes.DWORD), ('time', wintypes.DWORD), ('dwExtraInfo', ctypes.c_size_t)]


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [('wVk', wintypes.WORD), ('wScan', wintypes.WORD), ('dwFlags', wintypes.DWORD),
                ('time', wintypes.DWORD), ('dwExtraInfo', ctypes.c_size_t)]


class INPUT(ctypes.Structure):
    class _U(ctypes.Union):
        _fields_ = [('mi', MOUSEINPUT), ('ki', KEYBDINPUT), ('pad', ctypes.c_byte * 32)]
    _anonymous_ = ('u',)
    _fields_ = [('type', wintypes.DWORD), ('u', _U)]


def send_inputs(*inputs):
    array = (INPUT * len(inputs))(*inputs)
    user32.SendInput(len(inputs), array, ctypes.sizeof(INPUT))
    time.sleep(0.05)


def mouse(flags):
    event = INPUT(type=0)
    event.mi = MOUSEINPUT(0, 0, 0, flags, 0, 0)
    return event


def key(vk, up=False):
    event = INPUT(type=1)
    event.ki = KEYBDINPUT(vk, user32.MapVirtualKeyW(vk, 0), 2 if up else 0, 0, 0)
    return event


def tap(*vks):
    send_inputs(*[key(vk) for vk in vks])
    send_inputs(*[key(vk, up=True) for vk in reversed(vks)])


def events(viewer, settle=0.5):
    time.sleep(settle)
    return [message for kind, message in viewer.drain() if kind == 4]


# --- The stream: a moving test pattern from the helper's own encoder.
encoded = subprocess.run([exe, 'self-test', 'stream', str(width), str(height), '300'],
                         capture_output=True, check=True).stdout
frames = [packet(kind, payload) for kind, payload in packets(encoded) if kind == 1]
check(len(frames) >= 280, 'synthetic stream', f'{len(frames)} frames, {len(encoded)} bytes')

viewer = Viewer()
try:
    started = time.monotonic()
    ready = viewer.wait(lambda kind, m: kind == 0, 10)
    check(ready == {'ready': True}, 'ready', f'{ready} after {time.monotonic() - started:.2f}s')

    # The runtime asks every 50 ms for up to 4 s until the window is full screen.
    started, viewport, request = time.monotonic(), None, 0
    while time.monotonic() - started < 4:
        request += 1
        viewer.send(control({'type': 'viewport', 'id': request}))
        reply = viewer.wait(lambda kind, m: kind == 2 and m.get('type') == 'viewport', 1)
        if reply and reply.get('fullscreen'):
            viewport = reply
            break
        time.sleep(0.05)
    screen = (user32.GetSystemMetrics(0), user32.GetSystemMetrics(1))
    check(viewport is not None and (viewport['width'], viewport['height']) == screen and viewport['scale'] == 1.0,
          'full-screen viewport', f'{viewport} after {time.monotonic() - started:.2f}s, primary monitor {screen}')

    viewer.send(control({'type': 'configuration', 'id': 100, 'width': width, 'height': height, 'retina': False,
                         'fps': 60, 'bitrate': 12000000}))
    reply = viewer.wait(lambda kind, m: kind == 2 and m.get('type') == 'configured', 2)
    check(reply == {'type': 'configured', 'id': 100, 'ok': True}, 'settings applied', str(reply))
    viewer.send(control({'type': 'configuration', 'id': 101, 'width': 100, 'height': height, 'retina': False,
                         'fps': 60, 'bitrate': 12000000}))
    reply = viewer.wait(lambda kind, m: kind == 2 and m.get('type') == 'configured', 2)
    check(reply == {'type': 'configured', 'id': 101, 'ok': False}, 'invalid settings refused', str(reply))

    # Pointer packets are ignored by a viewer that draws the pointer into the video.
    viewer.send(packet(3, b'{"revision":1,"sequence":1,"x":0.5,"y":0.5,"visible":true}'))
    started, render_ready, presented = time.monotonic(), None, 0
    for index, frame in enumerate(frames):
        viewer.send(frame)
        time.sleep(max(0, started + (index + 1) / 60 - time.monotonic()))
        for kind, message in viewer.drain():
            if kind == 2 and message.get('type') == 'render_ready' and render_ready is None:
                render_ready = time.monotonic() - started
            if kind == 2 and message.get('type') == 'presented':
                presented = max(presented, message['frames'])
    last = viewer.wait(lambda kind, m: kind == 2 and m.get('type') == 'presented', 1)
    presented = max(presented, last['frames'] if last else 0)
    check(render_ready is not None, 'render ready', f'after {render_ready}s' if render_ready else 'never')
    check(presented >= 240, 'presented frames', f'{presented} of {len(frames)} reported')

    viewer.send(packet(1, b'\0' * 5))
    reply = viewer.wait(lambda kind, m: kind == 2 and m.get('type') == 'keyframe', 1)
    check(reply == {'type': 'keyframe'}, 'keyframe after an unreadable frame', str(reply))
    viewer.send(packet(1, struct.pack('>Q', 1) + b'\0' + b'\0\0\0\x01\x41' + bytes(range(200))))
    reply = viewer.wait(lambda kind, m: kind == 2 and m.get('type') == 'keyframe', 1)
    print(f"note garbage picture data: {'keyframe requested' if reply else 'concealed by the decoder'}")

    # Input: available but not granted, a click asks for the lease and is sent; once granted,
    # keys follow. A real click also brings the window to the front.
    viewer.send(control({'type': 'input_state', 'available': True, 'enabled': False, 'focused': False,
                         'text_supported': True, 'shortcut_policy': 'command',
                         'content': {'x': 0.0, 'y': 0.0, 'width': 1.0, 'height': 1.0}, 'revision': 1}))
    time.sleep(0.2)
    viewer.drain()
    user32.SetForegroundWindow(window())
    user32.SetCursorPos(screen[0] // 2, screen[1] // 2)
    send_inputs(mouse(0x2), mouse(0x4))
    clicked = events(viewer)
    kinds = [m['type'] if m['type'] == 'focus' else m['command']['event']['kind'] for m in clicked
             if m['type'] == 'focus' or m['command']['kind'] == 'input']
    check(kinds[:3] == ['focus', 'button', 'button'] and clicked[0].get('active') is True,
          'click asks for the lease', json.dumps(clicked))
    viewer.send(control({'type': 'input_state', 'available': True, 'enabled': True, 'focused': True,
                         'text_supported': True, 'shortcut_policy': 'command',
                         'content': {'x': 0.0, 'y': 0.0, 'width': 1.0, 'height': 1.0}, 'revision': 1}))
    time.sleep(0.2)
    viewer.drain()
    tap(0x41)  # A
    typed = [m['command']['event'] for m in events(viewer) if m['type'] == 'input']
    check(typed == [{'kind': 'text', 'value': 'a'}], 'a printable key is text only', json.dumps(typed))
    tap(0x11, 0x43)  # Ctrl+C
    chord = [m['command']['event'] for m in events(viewer) if m['type'] == 'input']
    check({'kind': 'mac_key', 'code': 8, 'down': True, 'repeat': False} in chord
          and {'kind': 'modifiers', 'mask': 8} in chord and all(e['kind'] != 'text' for e in chord),
          'Ctrl+C is Command+C for a Command source', json.dumps(chord))
    tap(0x0D)  # Enter
    enter = [m['command']['event'] for m in events(viewer) if m['type'] == 'input']
    check([e.get('code') for e in enter] == [36, 36] and all(e['kind'] == 'mac_key' for e in enter),
          'Enter is a physical key', json.dumps(enter))

    # Closing the window gives up the lease and ends the session.
    user32.PostMessageW(window(), 0x0010, 0, 0)  # WM_CLOSE
    closed = viewer.wait(lambda kind, m: kind == 2 and m.get('type') == 'closed', 2)
    check(closed == {'type': 'closed'}, 'closing the window reports closed')
    seconds = viewer.exited(2)
    check(seconds is not None and viewer.process.wait(2) == 0, 'exit after closing', f'{seconds}s')
finally:
    viewer.kill()
    print('\n'.join(f'  viewer: {line}' for line in viewer.stderr[-15:]))

def exit_keys(viewer, *vks):
    user32.SetForegroundWindow(window())
    time.sleep(0.2)
    tap(*vks)


for name, end in [('stop', lambda v: v.send(control({'type': 'stop'}))),
                  ('end of input', lambda v: v.process.stdin.close()),
                  ('Ctrl+Alt+Esc', lambda v: exit_keys(v, 0x11, 0x12, 0x1B)),
                  ('parent exit', None)]:
    viewer = Viewer(wrap=end is None)
    child = None
    try:
        viewer.wait(lambda kind, m: kind == 0, 10)
        viewer.send(frames[0])
        if end:
            end(viewer)
        else:
            # Kill only cmd; the viewer's stdin stays open, so only its parent watch can end it.
            child = viewer.child()
            subprocess.run(['taskkill', '/F', '/PID', str(viewer.process.pid)], capture_output=True)
        seconds = viewer.exited(4)
        check(seconds is not None and seconds <= (3 if end is None else 2), f'exit on {name}', f'{seconds}s')
    finally:
        viewer.kill(child)

print('PASS' if not failures else f'FAILED: {", ".join(failures)}')
sys.exit(1 if failures else 0)
