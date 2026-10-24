"""Bounded continuous-load observation, separate from optical campaigns."""
import argparse, datetime, json, subprocess, sys, time, uuid
from pathlib import Path

HERE=Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parent/'automation')]
from bootstrap import ROOT, build_swift
from bench import product_adapter
from client import Client
from context import application, host, network, digest
from metrics import resources, network_delta, classify_route
from run import Sampler, profile, verify_display_profile, verify_source, write
from stages import snapshot

def resource_windows(rows, seconds=30):
    """Retain time-local resource changes rather than hiding them in one median."""
    if len(rows)<3:raise RuntimeError('Sustained observation has too few samples')
    start=rows[0]['at_ns']; buckets={}
    for row in rows:
        index=int((row['at_ns']-start)//int(seconds*1e9))
        buckets.setdefault(index,[]).append(row)
    return [{'start_offset_s':index*seconds,'sample_count':len(group),
             'resources':resources(group)} for index,group in sorted(buckets.items()) if len(group)>=3]

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--product',default='extend')
    parser.add_argument('--pairing',type=Path,default=ROOT/'.cache/campaign/source-pairing.json')
    parser.add_argument('--seconds',type=int,default=300)
    parser.add_argument('--screen',default='auto')
    parser.add_argument('--display-profile',required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    if not 60<=args.seconds<=600:parser.error('--seconds must be 60..600')
    adapter=product_adapter()
    if args.product!='extend' and not (adapter and args.product in adapter.products()):
        raise RuntimeError('Product adapter unavailable')
    client=Client(args.pairing)
    health=client.request('/health');source=client.request('/context/'+args.product)
    if health['workload_sha256']!=digest(HERE.parents[2]/'benchmarks/display/workloads/campaign-workload.swift'):
        raise RuntimeError('Workload versions differ between hosts')
    display=profile(source,args.screen);verify_display_profile(display,args.display_profile)
    receiver=application(args.product,adapter)
    if args.product!='extend' and source['application']['executable_sha256']!=receiver['executable_sha256']:
        raise RuntimeError('Third-party test builds differ between hosts')
    if args.product=='extend' and any(source['application'].get(k)!=receiver.get(k) for k in ['version','build']):
        raise RuntimeError('Extend versions differ between hosts')
    tools=build_swift(HERE.parents[2])
    args.output.mkdir(parents=True,exist_ok=False,mode=0o700)
    report={'schema_version':1,'kind':'sustained_load','status':'running',
        'created_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'product':args.product,'seconds':args.seconds,'workload_sha256':health['workload_sha256'],
        'context':{'source':source,'receiver':{'host':host(),'application':receiver}},
        'limits':['Continuous synthetic motion; no camera capture or physical input.',
                  'A live viewer and process telemetry do not prove physical-display cadence or image quality.',
                  'No network interruption, sleep/wake, or reconnection fault is injected.',
                  'This is connected-load observation, not an idle-baseline comparison.']}
    output=args.output/'report.json';write(output,report)
    job={'id':str(uuid.uuid4()),'product':args.product,'scene':'motion',
         'seconds':args.seconds,'screen':args.screen}
    # The source context's peer is this receiver, not the source endpoint.
    from urllib.parse import urlsplit
    peer=urlsplit(client.config['url']).hostname
    before=network(receiver,peer);started=time.monotonic_ns()
    try:
        viewers=[h['pid'] for h in receiver.get('display_helpers',[]) if h['role']=='viewer']
        if not (adapter and hasattr(adapter,'focus_viewer') and adapter.focus_viewer(receiver,tools/'campaign-workload')):
            subprocess.run([str(tools/'campaign-workload'),'--activate',str(viewers[0] if len(viewers)==1 else receiver['main_pid'])],check=True,capture_output=True)
        with Sampler(receiver) as sampler:
            client.request('/start',job)
            print(f'Continuous motion running for {args.seconds} seconds; leave both hosts idle.',flush=True)
            measured=client.wait(job['id'],args.seconds)
        sampler.summary();verify_source(measured,display,args.screen,source['application']['executable_sha256'])
        current=application(args.product,adapter)
        if current['main_pid']!=receiver['main_pid'] or current['executable_sha256']!=receiver['executable_sha256']:
            raise RuntimeError('Receiver application restarted or changed')
        ended=time.monotonic_ns()
        write(args.output/'source.json',measured);write(args.output/'receiver-samples.json',sampler.rows)
        deltas={'source':network_delta(measured['network_start'],measured['network_end']),
                'receiver':network_delta(before,network(receiver,peer))}
        route=classify_route(deltas)
        report.update({'status':'complete_with_declared_limits','route':route,
            'resource_windows':{'source':resource_windows(measured['rows']),
                                'receiver':resource_windows(sampler.rows)},
            'stages':{'source':measured['stages'],'receiver':snapshot(receiver,adapter,started,ended)},
            'source_cadence':measured['fixture']['source_callback_hz']})
        if route['classification']!='direct_lan':raise RuntimeError('Sustained local test did not retain a verified direct LAN route')
    except BaseException as error:
        report.update({'status':'failed','failure':str(error)})
        # Cancel only a fixture still owned by this command, never another job.
        try:
            state=client.request('/status')
            if state.get('id')==job['id'] and state.get('status')=='running':client.request('/stop',{})
        except Exception:pass
        raise
    finally:write(output,report)
    print('Sustained observation completed: '+str(output),flush=True)

if __name__=='__main__':main()
