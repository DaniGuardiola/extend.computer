"""Bounded synthetic frame trains; run outside optical/product measurement phases."""
import argparse, ctypes, ipaddress, json, selectors, socket, struct, sys, time
from pathlib import Path
from metrics import stats
from lan_probe import service, SERVICES
HEADER=struct.Struct('!4sIHH')
SIZE=1200
MAGIC=b'EXSP'
HELLO=b'extend-stream-probe-v1\n'

class Timeval(ctypes.Structure):
    _fields_=[('seconds',ctypes.c_long),('microseconds',ctypes.c_int)]

def packet(frame,part,chunks):
    return HEADER.pack(MAGIC,frame,part,chunks)+bytes(SIZE-HEADER.size)

def decode(data,chunks):
    if len(data)!=SIZE:raise ValueError('Wrong probe packet size')
    magic,frame,part,total=HEADER.unpack(data[:HEADER.size])
    if magic!=MAGIC or total!=chunks or part>=chunks or any(data[HEADER.size:]):raise ValueError('Wrong synthetic payload')
    return frame,part

def sender(a):
    tcp=socket.socket();tcp.bind((a.bind,0));tcp.listen(1);tcp.settimeout(30)
    port=tcp.getsockname()[1];print(json.dumps({'port':port,'protocol':a.protocol,'chunks':a.chunks,'hz':a.hz,'seconds':a.seconds}),flush=True)
    conn=None;udp=None
    try:
        conn,address=tcp.accept()
        if address[0]!=a.peer:raise ValueError('Unexpected peer')
        conn.settimeout(2);conn.setsockopt(socket.IPPROTO_TCP,socket.TCP_NODELAY,1);service(conn,a.service)
        hello=bytearray()
        while b'\n' not in hello and len(hello)<128:
            part=conn.recv(128-len(hello))
            if not part:raise ValueError('Incomplete probe greeting')
            hello.extend(part)
        port=None
        if a.protocol=='tcp':
            if bytes(hello)!=HELLO:raise ValueError('Wrong probe greeting')
        else:
            prefix=HELLO[:-1]+b' '
            if not hello.startswith(prefix) or not hello.endswith(b'\n'):raise ValueError('Wrong probe greeting')
            port=int(hello[len(prefix):-1])
            if not 1<=port<=65535:raise ValueError('Invalid receiver port')
        if a.protocol=='udp':
            udp=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);udp.bind((a.bind,0));service(udp,a.service)
            conn.sendall(struct.pack('!H',udp.getsockname()[1]))
        start=time.monotonic_ns();times=[];frame=0
        while time.monotonic_ns()<start+a.seconds*1_000_000_000:
            due=start+int(frame*1e9/a.hz);delay=(due-time.monotonic_ns())/1e9
            if delay>0:time.sleep(delay)
            times.append(time.monotonic_ns())
            for part in range(a.chunks):
                data=packet(frame,part,a.chunks)
                if udp:udp.sendto(data,(a.peer,port))
                else:conn.sendall(data)
            frame+=1
        print(json.dumps({'sent_frames':frame,'send_gaps_ms':stats([(b-a)/1e6 for a,b in zip(times,times[1:])])}),flush=True)
    finally:
        if conn:conn.close()
        if udp:udp.close()
        tcp.close()

def receiver(a):
    udp=None;tcp=socket.socket();tcp.settimeout(5);tcp.connect((a.peer,a.port));tcp.setsockopt(socket.IPPROTO_TCP,socket.TCP_NODELAY,1);service(tcp,a.service)
    if a.protocol=='udp':
        udp=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);udp.bind((a.bind,0));service(udp,a.service)
        if sys.platform=='darwin':udp.setsockopt(socket.SOL_SOCKET,0x0400,1)
        tcp.sendall(HELLO[:-1]+b' '+str(udp.getsockname()[1]).encode()+b'\n')
        port_bytes=bytearray()
        while len(port_bytes)<2:
            data=tcp.recv(2-len(port_bytes))
            if not data:raise ValueError('Missing source UDP port')
            port_bytes.extend(data)
        reverse_port=struct.unpack('!H',port_bytes)[0]
    else:tcp.sendall(HELLO)
    sock=udp or tcp;sock.setblocking(False);selector=selectors.DefaultSelector();selector.register(sock,selectors.EVENT_READ)
    started=time.monotonic();deadline=started+a.seconds+3;buffer=bytearray();frames={};samples=[];first={};kernel_first={};queue=[];reverse_count=0;next_reverse=started
    try:
        while time.monotonic()<deadline:
            now=time.monotonic()
            if udp and a.reverse_hz and now<started+a.seconds and now>=next_reverse:
                udp.sendto(b'EXSA'+struct.pack('!I',reverse_count),(a.peer,reverse_port))
                reverse_count+=1;next_reverse=now+1/a.reverse_hz
            delay=min(.05,max(0,next_reverse-time.monotonic())) if udp and a.reverse_hz and now<started+a.seconds else .05
            if not selector.select(delay):continue
            kernel=None
            if udp:
                data,anc,flags,address=udp.recvmsg(SIZE+1,128)
                if address[0]!=a.peer:continue
                for level,kind,value in anc:
                    if level==socket.SOL_SOCKET and kind==2 and len(value)>=ctypes.sizeof(Timeval):
                        stamp=Timeval.from_buffer_copy(value);kernel=stamp.seconds*1_000_000_000+stamp.microseconds*1000
                packets=[data]
            else:
                data=tcp.recv(65536)
                if not data:break
                buffer.extend(data);packets=[]
                while len(buffer)>=SIZE:packets.append(bytes(buffer[:SIZE]));del buffer[:SIZE]
            now=time.monotonic_ns();wall=time.time_ns()
            for data in packets:
                frame,part=decode(data,a.chunks)
                if frame>=a.seconds*a.hz+2:raise ValueError('Frame outside bounded probe')
                frames.setdefault(frame,set()).add(part);first.setdefault(frame,now)
                if kernel is not None:
                    kernel_first.setdefault(frame,kernel);queue.append((wall-kernel)/1e6)
                samples.append({'frame':frame,'part':part,'received_ns':now,'kernel_wall_ns':kernel})
    finally:
        selector.close();tcp.close()
        if udp:udp.close()
    observed=sorted(first);gaps=lambda values:stats([(values[b]-values[a])/1e6 for a,b in zip(observed,observed[1:]) if a in values and b in values])
    result={'schema_version':1,'protocol':a.protocol,'service':a.service,'requested_hz':a.hz,'chunks':a.chunks,'packet_bytes':SIZE,'seconds':a.seconds,
        'requested_reverse_hz':a.reverse_hz,'reverse_packets_sent':reverse_count,
        'observed_frames':len(frames),'complete_frames':sum(len(v)==a.chunks for v in frames.values()),'first_packet_gaps_ms':gaps(first),
        'kernel_first_packet_gaps_ms':gaps(kernel_first),'kernel_to_userspace_ms':stats(queue),'samples':samples,
        'limits':['Synthetic frame train, not product video or screen latency.','Only receiver-local gaps are measured; no cross-host clock subtraction.',
                  'Kernel timestamps are OS receive timestamps, not wireless transmission or hardware photon timestamps.','TCP receive batching can place several records at one userspace timestamp.',
                  'Active probes change network behavior. Never run during optical phases.']}
    a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(result,indent=2))
    print(json.dumps({k:v for k,v in result.items() if k!='samples'},indent=2))

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('mode',choices=['sender','receiver']);p.add_argument('--bind',required=True);p.add_argument('--peer',required=True)
    p.add_argument('--protocol',choices=['udp','tcp'],required=True);p.add_argument('--seconds',type=int,default=10);p.add_argument('--hz',type=int,default=60);p.add_argument('--chunks',type=int,default=8)
    p.add_argument('--service',choices=SERVICES,default='responsive-video');p.add_argument('--port',type=int);p.add_argument('--output',type=Path)
    p.add_argument('--reverse-hz',type=int,default=0,help='UDP-only synthetic reverse activity diagnostic, default off')
    a=p.parse_args()
    for address in [a.bind,a.peer]:
        ip=ipaddress.ip_address(address)
        if ip.version!=4 or not (ip.is_private or ip.is_loopback):p.error('Use private IPv4 addresses')
    if not 3<=a.seconds<=30 or not 1<=a.hz<=120 or not 1<=a.chunks<=16:p.error('Use 3..30 seconds, 1..120 Hz, 1..16 chunks')
    if not 0<=a.reverse_hz<=120 or (a.reverse_hz and a.protocol!='udp'):p.error('Reverse activity requires UDP and 0..120 Hz')
    if a.mode=='receiver' and (not a.output or not a.port or not 1<=a.port<=65535):p.error('Receiver requires port and output')
    if a.mode=='sender':sender(a)
    else:receiver(a)
if __name__=='__main__':main()
