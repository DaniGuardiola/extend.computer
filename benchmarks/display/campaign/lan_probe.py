"""Bounded synthetic LAN round-trip diagnostic, outside product benchmark phases.

No product packets or user content are collected. Server echoes only 16-byte
probe messages from one configured peer and exits at its fixed deadline.
"""
import argparse
import ipaddress
import json
import selectors
import socket
import struct
import sys
import time
from pathlib import Path
from metrics import stats

SERVICES={'best-effort':0,'responsive-video':5,'interactive-video':3}


def service(sock, name):
    if name=='best-effort':return
    if sys.platform!='darwin':raise ValueError('Service diagnostic requires macOS')
    sock.setsockopt(socket.SOL_SOCKET,0x1116,SERVICES[name])
    if sock.getsockopt(socket.SOL_SOCKET,0x1116)!=SERVICES[name]:
        raise RuntimeError('Network service type not applied')


def server(bind, peer, seconds, selected_service):
    selector=selectors.DefaultSelector()
    udp=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);udp.bind((bind,0));udp.setblocking(False)
    service(udp,selected_service)
    port=udp.getsockname()[1]
    tcp=socket.socket(socket.AF_INET,socket.SOCK_STREAM);tcp.bind((bind,port));tcp.listen(1);tcp.setblocking(False)
    service(tcp,selected_service)
    selector.register(udp,selectors.EVENT_READ,'udp');selector.register(tcp,selectors.EVENT_READ,'accept')
    buffers={};count={'udp':0,'tcp':0}
    print(json.dumps({'port':port,'service':selected_service,'deadline_seconds':seconds}),flush=True)
    deadline=time.monotonic()+seconds
    try:
        while time.monotonic()<deadline:
            for key,_ in selector.select(min(.2,max(0,deadline-time.monotonic()))):
                sock=key.fileobj
                if key.data=='udp':
                    data,address=sock.recvfrom(2048)
                    if address[0]==peer and len(data)==16:
                        try:sock.sendto(data,address);count['udp']+=1
                        except BlockingIOError:pass
                elif key.data=='accept':
                    conn,address=sock.accept()
                    if address[0]!=peer or buffers:conn.close();continue
                    conn.setsockopt(socket.IPPROTO_TCP,socket.TCP_NODELAY,1);service(conn,selected_service)
                    conn.setblocking(False);selector.register(conn,selectors.EVENT_READ,'tcp');buffers[conn]=bytearray()
                else:
                    try:data=sock.recv(2048)
                    except ConnectionResetError:data=b''
                    if not data:selector.unregister(sock);buffers.pop(sock,None);sock.close();continue
                    buffer=buffers[sock];buffer.extend(data)
                    while len(buffer)>=16:
                        packet=bytes(buffer[:16]);del buffer[:16]
                        # Sixteen-byte echoes at <=120 Hz fit the bounded socket
                        # queue. An unexpectedly full queue ends this probe rather
                        # than silently making scheduling-dependent RTT claims.
                        sent=sock.send(packet)
                        if sent!=16:raise RuntimeError('Probe echo queue saturated')
                        count['tcp']+=1
    finally:
        for key in list(selector.get_map().values()):key.fileobj.close()
        selector.close()
    print(json.dumps({'status':'finished','echoes':count}),flush=True)


def client(peer, port, seconds, hz, selected_service, output):
    selector=selectors.DefaultSelector()
    udp=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);udp.connect((peer,port));service(udp,selected_service);udp.setblocking(False)
    tcp=socket.socket(socket.AF_INET,socket.SOCK_STREAM);tcp.settimeout(5);tcp.connect((peer,port))
    tcp.setsockopt(socket.IPPROTO_TCP,socket.TCP_NODELAY,1);service(tcp,selected_service);tcp.setblocking(False)
    selector.register(udp,selectors.EVENT_READ,'udp');selector.register(tcp,selectors.EVENT_READ,'tcp')
    pending={'udp':{},'tcp':{}};rows={'udp':[],'tcp':[]};buffers=bytearray();sent={'udp':0,'tcp':0}
    start=time.monotonic_ns();end=start+int(seconds*1e9);period=int(1e9/hz);next_send=start;sequence=0;send_gaps=[];previous_send=None
    try:
        while time.monotonic_ns()<end+int(2e9):
            now=time.monotonic_ns()
            if now>=next_send and now<end:
                if previous_send is not None:send_gaps.append((now-previous_send)/1e6)
                previous_send=now;sequence+=1;next_send=now+period
                for name,sock in [('udp',udp),('tcp',tcp)]:
                    at=time.monotonic_ns();packet=struct.pack('!QQ',sequence,at)
                    n=sock.send(packet)
                    if n!=16:raise RuntimeError('Probe send queue saturated')
                    pending[name][packet]=at;sent[name]+=1
            delay=min(.02,max(0,(next_send-time.monotonic_ns())/1e9)) if time.monotonic_ns()<end else .02
            for key,_ in selector.select(delay):
                data=key.fileobj.recv(2048);received=time.monotonic_ns()
                if not data:raise RuntimeError('Probe peer closed unexpectedly')
                if key.data=='udp':packets=[data] if len(data)==16 else []
                else:
                    buffers.extend(data);packets=[]
                    while len(buffers)>=16:packets.append(bytes(buffers[:16]));del buffers[:16]
                for packet in packets:
                    at=pending[key.data].pop(packet,None)
                    if at is not None:rows[key.data].append((received-at)/1e6)
            if time.monotonic_ns()>=end and not any(pending.values()):break
    finally:
        udp.close();tcp.close();selector.close()
    result={'schema_version':1,'service':selected_service,'requested_hz':hz,'duration_s':seconds,
            'send_gaps_ms':stats(send_gaps),'results':{name:{'sent':sent[name],
            'responses':len(values),'unanswered_after_two_seconds':len(pending[name]),
            'rtt_ms':stats(values)} for name,values in rows.items()},
            'limits':['Synthetic simultaneous 16-byte TCP and UDP echo, not product throughput or screen latency.',
                      'Round trips use only the client monotonic clock; no cross-host clock subtraction.',
                      'Missing responses remain reported; RTT distribution includes received responses only.',
                      'Active probes can change Wi-Fi power/queue behavior. Do not run during optical capture.']}
    output.parent.mkdir(parents=True,exist_ok=True);output.write_text(json.dumps(result,indent=2))
    print(json.dumps(result,indent=2))


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('mode',choices=['server','client']);p.add_argument('--bind',default='127.0.0.1')
    p.add_argument('--peer',required=True);p.add_argument('--port',type=int)
    p.add_argument('--seconds',type=int,default=30);p.add_argument('--hz',type=int,default=60)
    p.add_argument('--service',choices=SERVICES,default='best-effort');p.add_argument('--output',type=Path)
    a=p.parse_args()
    for address in [a.bind,a.peer]:
        value=ipaddress.ip_address(address)
        if value.version!=4 or not (value.is_private or value.is_loopback):p.error('Use IPv4 LAN addresses')
    if not 3<=a.seconds<=120 or not 1<=a.hz<=120:p.error('Use 3..120 seconds and 1..120 Hz')
    if a.mode=='client' and (not a.output or not a.port or not 1<=a.port<=65535):p.error('Client needs --port and --output')
    if a.mode=='server':server(a.bind,a.peer,a.seconds,a.service)
    else:client(a.peer,a.port,a.seconds,a.hz,a.service,a.output)

if __name__=='__main__':main()
