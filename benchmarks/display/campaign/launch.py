"""Launch the selected Extend app with a fresh numeric timing session."""
import argparse,json,os,plistlib,signal,subprocess,time,uuid
from pathlib import Path
from context import processes
from bootstrap import ROOT

def reordering_deadline(value):
    try: milliseconds=int(value)
    except (ValueError,TypeError): raise argparse.ArgumentTypeError('Reordering deadline must be 20..1000 ms')
    if isinstance(value,bool) or str(milliseconds)!=str(value) or not 20<=milliseconds<=1000:
        raise argparse.ArgumentTypeError('Reordering deadline must be 20..1000 ms')
    return milliseconds

def launch_app(app,exe,arguments,trace,log,observe_marker=False,observe_transport=False,settings_file=None,paced_render=False,explicit_render=False,paced_transport=False,notification_render=False,video_qos=False,responsive_service=False,low_latency_encoder=False,capture_bgra=False,nv12_srgb_input=True,reordering_ms=500,encoder_slots=2,context_only=False,buffered_render=False,fixed_bitrate=False,kernel_arrival=False,auto_low_latency=False,auto_kernel_arrival=False):
    # Launch Services creates an independent application rather than a child
    # inheriting the invoking terminal or automation host's execution context.
    reordering_ms=reordering_deadline(reordering_ms)
    if context_only and (observe_marker or observe_transport):
        raise ValueError('Context-only launches cannot enable marker or transport timing')
    if buffered_render and not paced_render:
        raise ValueError('Buffered presentation requires paced rendering')
    if auto_low_latency and low_latency_encoder:
        raise ValueError('Automatic and explicit low-latency encoder policies are mutually exclusive')
    if auto_kernel_arrival and kernel_arrival:
        raise ValueError('Automatic and explicit kernel-arrival policies are mutually exclusive')
    environment=['--env','EXTEND_DISPLAY_REORDERING_MS='+str(reordering_ms)]
    environment.extend(['--env','EXTEND_DISPLAY_BENCHMARK_CONTEXT=1'] if context_only else ['--env','EXTEND_DISPLAY_METRICS='+str(trace)])
    environment.extend(['--env','EXTEND_DISPLAY_ENCODER_SLOTS='+str(encoder_slots)])
    if observe_marker:environment.extend(['--env','EXTEND_DISPLAY_BENCHMARK_MARKER=1'])
    if observe_transport:environment.extend(['--env','EXTEND_DISPLAY_TRANSPORT_TIMING=1'])
    if settings_file:environment.extend(['--env','EXTEND_DISPLAY_BENCHMARK_SETTINGS='+str(settings_file)])
    if paced_render:environment.extend(['--env','EXTEND_DISPLAY_PACED_RENDER=1'])
    if buffered_render:environment.extend(['--env','EXTEND_DISPLAY_BUFFERED_RENDER=1'])
    if explicit_render:environment.extend(['--env','EXTEND_DISPLAY_EXPLICIT_RENDER=1'])
    if notification_render:environment.extend(['--env','EXTEND_DISPLAY_EXPLICIT_RENDER=0'])
    if paced_transport:environment.extend(['--env','EXTEND_DISPLAY_PACED_TRANSPORT=1'])
    if video_qos:environment.extend(['--env','EXTEND_DISPLAY_DSCP=34'])
    if responsive_service:environment.extend(['--env','EXTEND_DISPLAY_MACOS_RESPONSIVE=1'])
    # Explicit zero preserves controlled normal-mode runs when the app's
    # ordinary USB policy selects low-latency encoding automatically.
    if not auto_low_latency:environment.extend(['--env','EXTEND_DISPLAY_LOW_LATENCY_ENCODER='+('1' if low_latency_encoder else '0')])
    if fixed_bitrate:environment.extend(['--env','EXTEND_DISPLAY_FIXED_BITRATE=1'])
    if not auto_kernel_arrival:environment.extend(['--env','EXTEND_DISPLAY_KERNEL_ARRIVAL='+('1' if kernel_arrival else '0')])
    if capture_bgra:environment.extend(['--env','EXTEND_DISPLAY_CAPTURE_BGRA=1'])
    environment.extend(['--env','EXTEND_DISPLAY_NV12_SRGB_INPUT='+('1' if nv12_srgb_input else '0')])
    subprocess.run(['/usr/bin/open','-n','-a',str(app),*environment,
                    '--stdout',str(log),'--stderr',str(log),
                    '--args',*arguments],check=True,timeout=15)
    deadline=time.monotonic()+15
    while time.monotonic()<deadline:
        selected=[r for r in processes() if r['path']==str(exe)]
        if len(selected)==1:return selected[0]['pid']
        if len(selected)>1:raise RuntimeError('Multiple prepared app instances launched')
        time.sleep(.1)
    raise RuntimeError('Prepared app did not launch; inspect '+str(log))

def connection_arguments(peer,profile,seconds,fps=60,quality="balanced",fit_display=False,codec="hevc"):
    import re
    match=re.fullmatch(r'([1-9]\d*)x([1-9]\d*)@([12])',profile)
    if not match:raise ValueError("Display profile must be WIDTHxHEIGHT@SCALE")
    width,height,scale=map(int,match.groups())
    if fit_display and scale!=2:raise ValueError('Fit-display launch requires a Retina profile; the profile is only an initial size hint')
    if width<640 or height<480 or width*scale>4096 or height*scale>4096:
        raise ValueError("Display backing dimensions must fit 4096 pixels")
    if not 30<=seconds<=3600:raise ValueError("Connection duration must be 30..3600 seconds")
    if fps not in (30,60,120):raise ValueError("Frame rate must be 30, 60 or 120")
    if codec not in ('hevc','h264'):raise ValueError('Benchmark codec must be hevc or h264')
    bitrate={"responsive":8_000_000,"balanced":20_000_000,"detail":40_000_000}[quality]
    arguments=["--display-benchmark",peer,str(width),str(height),str(seconds),
               "auto" if fit_display else "retina" if scale==2 else "native",str(fps),str(bitrate)]
    # Preserve the established HEVC invocation for older prepared builds.
    if codec!='hevc':arguments.append(codec)
    return arguments

def main():
    p=argparse.ArgumentParser();p.add_argument('--app',type=Path,default=Path('/Applications/extend.computer.app'))
    p.add_argument('--connect',help='Start a benchmark display connection to PEER (or account)')
    p.add_argument('--display-profile',help='Fixed logical size and scale: WIDTHxHEIGHT@SCALE')
    p.add_argument('--fit-display',action='store_true',help='Negotiate actual fullscreen viewer geometry; Retina profile is only an initial size hint')
    p.add_argument('--settings-file',type=Path,help='Explicit development-only settings request JSON; the app writes a sibling .ack.json')
    p.add_argument('--fps',type=int,choices=[30,60,120],default=60)
    p.add_argument('--quality',choices=['responsive','balanced','detail'],default='balanced')
    p.add_argument('--codec',choices=['hevc','h264'],default='hevc',help='Benchmark connection codec; HEVC remains the default')
    p.add_argument('--connection-seconds',type=int,default=3600)
    p.add_argument('--reordering-ms',type=reordering_deadline,default=500,help='Development video loss/reordering deadline, 20..1000 ms; default 500')
    p.add_argument('--observe-marker',action='store_true',help='Observe only the known synthetic counter at capture and decode boundaries')
    p.add_argument('--observe-transport',action='store_true',help='Observe bounded numeric UDP/RTP timing without recording payloads')
    p.add_argument('--context-only',action='store_true',help='Retain startup build and periodic route evidence without per-frame stage or transport timing; use for tracing-overhead checks')
    rendering=p.add_mutually_exclusive_group()
    rendering.add_argument('--paced-render',action='store_true',help='Development comparison of panel-paced native rendering')
    rendering.add_argument('--explicit-render',action='store_true',help='Development comparison of immediate native rendering')
    rendering.add_argument('--notification-render',action='store_true',help='Development reference for the previous notification renderer')
    p.add_argument('--buffered-render',action='store_true',help='Development comparison of one-period, three-frame bounded presentation; requires --paced-render')
    p.add_argument('--paced-transport',action='store_true',help='Development comparison of packet pacing on LAN')
    p.add_argument('--video-qos',action='store_true',help='Development IPv4 socket-wide AF41 priority comparison; networks may ignore it')
    p.add_argument('--responsive-service',action='store_true',help='Development macOS screen-sharing socket service comparison')
    encoder_policy=p.add_mutually_exclusive_group()
    encoder_policy.add_argument('--low-latency-encoder',action='store_true',help='Development hardware low-latency rate-control comparison')
    encoder_policy.add_argument('--auto-low-latency',action='store_true',help='Use the ordinary app encoder policy; native evidence records its actual choice')
    p.add_argument('--fixed-bitrate',action='store_true',help='Development benchmark-feature comparison retaining the configured encoder bitrate instead of adapting it')
    arrival_policy=p.add_mutually_exclusive_group()
    arrival_policy.add_argument('--kernel-arrival',action='store_true',help='Development comparison using supported kernel UDP arrival timestamps for transport feedback')
    arrival_policy.add_argument('--auto-kernel-arrival',action='store_true',help='Use the ordinary app UDP arrival policy; socket evidence records its actual choice')
    p.add_argument('--encoder-slots',type=int,choices=[2,3],default=2,help='Development comparison of bounded hardware encoder capacity; production default 2')
    p.add_argument('--capture-bgra',action='store_true',help='Development RGB capture versus preconverted NV12 comparison')
    transfer=p.add_mutually_exclusive_group()
    transfer.add_argument('--nv12-srgb-input',action='store_true',default=True,help=argparse.SUPPRESS)
    transfer.add_argument('--native-capture-transfer',dest='nv12_srgb_input',action='store_false',help='Development reference retaining the original NV12 transfer attachment')
    a=p.parse_args()
    if a.buffered_render and not a.paced_render:p.error('--buffered-render requires --paced-render')
    if a.context_only and (a.observe_marker or a.observe_transport):p.error('--context-only cannot enable --observe-marker or --observe-transport')
    arguments=[]
    if bool(a.connect)!=bool(a.display_profile):p.error('--connect and --display-profile must be supplied together')
    if a.fit_display and not a.connect:p.error('--fit-display requires --connect')
    if a.codec!='hevc' and not a.connect:p.error('--codec h264 requires --connect')
    if a.connect:
        try:arguments=connection_arguments(a.connect,a.display_profile,a.connection_seconds,a.fps,a.quality,a.fit_display,a.codec)
        except ValueError as error:p.error(str(error))
    app=a.app.resolve()
    if not (app/'Contents/Info.plist').exists():raise RuntimeError('Extend app missing; supply --app PATH')
    info=plistlib.loads((app/'Contents/Info.plist').read_bytes())
    exe=app/'Contents/MacOS'/info['CFBundleExecutable']
    if not exe.exists():raise RuntimeError('Extend app missing; supply --app PATH')
    selected=[r for r in processes() if r['path']==str(exe)]
    for r in selected:os.kill(r['pid'],signal.SIGTERM)
    end=time.monotonic()+10
    while selected and time.monotonic()<end:
        live={r['pid'] for r in processes()};selected=[r for r in selected if r['pid'] in live]
        if selected:time.sleep(.1)
    if selected:raise RuntimeError('Existing Extend app did not quit; close it normally')
    out=ROOT/'.cache';out.mkdir(exist_ok=True);trace=out/('extend-'+uuid.uuid4().hex+'.jsonl')
    log=trace.with_suffix('.log');log.touch(mode=0o600)
    pid=launch_app(app,exe,arguments,trace,log,a.observe_marker,a.observe_transport,a.settings_file.resolve() if a.settings_file else None,a.paced_render,a.explicit_render,a.paced_transport,a.notification_render,a.video_qos,a.responsive_service,a.low_latency_encoder,a.capture_bgra,a.nv12_srgb_input,a.reordering_ms,a.encoder_slots,a.context_only,a.buffered_render,fixed_bitrate=a.fixed_bitrate,kernel_arrival=a.kernel_arrival,auto_low_latency=a.auto_low_latency,auto_kernel_arrival=a.auto_kernel_arrival)
    state=out/'extend-session.json';state.write_text(json.dumps({'pid':pid,'trace':str(trace),'log':str(log),'app':str(app),'launcher':'macos-launch-services',
        'requested_display':{'profile':a.display_profile,'fps':a.fps,'quality':a.quality,'fit_display':a.fit_display,'codec':a.codec},
        'diagnostics':{'stage_timing':not a.context_only,'synthetic_marker':a.observe_marker,'transport_timing':a.observe_transport,'video_reordering_ms':a.reordering_ms,'paced_render':a.paced_render,'buffered_render':a.buffered_render,'explicit_render':not a.paced_render and not a.notification_render,'notification_render':a.notification_render,'paced_transport':a.paced_transport,'socket_dscp':34 if a.video_qos else 0,'macos_screen_sharing_service':a.responsive_service,'low_latency_encoder':None if a.auto_low_latency else a.low_latency_encoder,'low_latency_encoder_policy':'auto' if a.auto_low_latency else 'enabled' if a.low_latency_encoder else 'disabled','fixed_bitrate':a.fixed_bitrate,'kernel_arrival':None if a.auto_kernel_arrival else a.kernel_arrival,'kernel_arrival_policy':'auto' if a.auto_kernel_arrival else 'enabled' if a.kernel_arrival else 'disabled','encoder_slots':a.encoder_slots,'capture_bgra':a.capture_bgra,'nv12_srgb_input':a.nv12_srgb_input}}));state.chmod(0o600)
    print(('Extend launched with context-only evidence; per-frame timing disabled. ' if a.context_only else 'Extend launched with numeric stage instrumentation. ')+('Benchmark connection requested; it ends after '+str(a.connection_seconds)+' seconds.' if a.connect else 'Establish the display connection normally.'))
if __name__=='__main__':main()
