"""Verify lease EOF and parent-crash cleanup. Temporarily pauses local AWDL."""
import subprocess,time,sys,json
HELPER='/Library/PrivilegedHelperTools/computer.extend.lowjitter-development/ExtendComputerLowJitter'
def up():
 text=subprocess.check_output(['/sbin/ifconfig','awdl0'],text=True)
 return 'UP' in text.splitlines()[0].split('<',1)[1].split('>',1)[0].split(',')
def status():return subprocess.check_output([HELPER,'status'],text=True).strip()
def restored():
 start=time.monotonic()
 while time.monotonic()-start<12:
  if up() and status()=='leases=0 restoration_pending=false':return round(time.monotonic()-start,3)
  time.sleep(.05)
 raise RuntimeError('Restoration deadline exceeded')
assert up() and status()=='leases=0 restoration_pending=false'
child=subprocess.Popen([HELPER,'lease'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
try:
 assert child.stdout.readline()=='READY\n'
 assert not up()
 child.stdin.close()
 print('pipe_eof_restore_s',restored(),flush=True)
 child.wait(timeout=5)
 assert child.returncode==0
finally:
 if child.poll() is None:child.kill();child.wait()
code='''import subprocess,time,sys
child=subprocess.Popen([sys.argv[1],"lease"],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
assert child.stdout.readline()=="READY\\n"
print("READY",flush=True)
time.sleep(25)
'''
parent=subprocess.Popen([sys.executable,'-c',code,HELPER],stdout=subprocess.PIPE,text=True)
try:
 assert parent.stdout.readline()=='READY\n'
 assert not up()
 parent.kill();parent.wait(timeout=5)
 print('parent_sigkill_restore_s',restored(),flush=True)
finally:
 if parent.poll() is None:parent.kill();parent.wait()
print('final',status(),'awdl_up',up(),flush=True)
