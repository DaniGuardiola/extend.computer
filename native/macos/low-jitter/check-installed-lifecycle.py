import json
import subprocess
import time

BINARY = '/Library/PrivilegedHelperTools/computer.extend.lowjitter-development/ExtendComputerLowJitter'

def up():
    output = subprocess.check_output(['/sbin/ifconfig', 'awdl0'], text=True)
    return 'UP' in output.splitlines()[0].split('<', 1)[1].split('>', 1)[0].split(',')

def wait_state(wanted, timeout):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        if up() == wanted:
            return True
        time.sleep(.1)
    return False

def status():
    return subprocess.check_output([BINARY,'status'], text=True).strip()

if not up():
    raise SystemExit('AWDL already down; leave unchanged and skip test')
print('initial',status(),flush=True)
for mode in ['disconnect', 'expiry']:
    child = subprocess.Popen([BINARY, 'test-30s'], stdout=subprocess.PIPE, stderr=subprocess.PIPE,text=True)
    try:
        if not wait_state(False,5):
            raise RuntimeError('lease did not lower AWDL')
        print(mode,'down confirmed',status(),flush=True)
        start = time.monotonic()
        if mode == 'disconnect':
            child.kill()
        else:
            import signal
            child.send_signal(signal.SIGSTOP)
        if not wait_state(True,15):
            raise RuntimeError('AWDL not restored by deadline')
        print(mode,'restored_after_s',round(time.monotonic()-start,3),status(),flush=True)
    finally:
        child.kill()
        child.communicate(timeout=5)
print('final',status(),'awdl_up',up(),flush=True)
