"""Compact per-scene timeline: what happened when, small enough to keep forever.

Built from per-frame evidence before it is pruned. Each series is binned on its
own clock, zeroed at the scene start: camera (first positive source counter),
receiver stages and resources (phase acknowledgement), source stages and
resources (source measurement window). Cross-clock alignment is approximate.
"""
import json,math,statistics
from pathlib import Path

BIN_S=.25

def _median(values):return statistics.median(values) if values else None

def _bins(duration):return max(1,math.ceil(duration/BIN_S))

def _put(slots,t,value):
    i=int(t//BIN_S)
    if 0<=i<len(slots):slots[i].append(value)

def camera_series(camera,source_index,count):
    """Visible updates, longest receiver stall and screen-to-screen lag per bin."""
    summary=json.loads((camera/'summary.json').read_text())
    factor=summary['video_metadata']['nominal_playback_fps']/summary['measured_sensor_hz']
    first=[{},{}]
    for line in (camera/'optical-frames.jsonl').read_text().splitlines():
        row=json.loads(line);t=row['playback_s']*factor
        for i,value in enumerate(row['counter_ids'][:2]):
            if value is not None and value>0 and value not in first[i]:first[i][value]=t
    a,b=source_index,1-source_index
    if not first[a]:return None
    t0=min(first[a].values())
    source=[[] for _ in range(count)];receiver=[[] for _ in range(count)];gaps=[[] for _ in range(count)];lag=[[] for _ in range(count)]
    for t in first[a].values():_put(source,t-t0,1)
    updates=sorted(first[b].values())
    for t in updates:_put(receiver,t-t0,1)
    for x,y in zip(updates,updates[1:]):_put(gaps,y-t0,(y-x)*1000)
    for value,t in first[a].items():
        if value in first[b] and value!=min(first[a]):_put(lag,t-t0,(first[b][value]-t)*1000)
    return {'source_updates_hz':[len(v)/BIN_S for v in source],'receiver_updates_hz':[len(v)/BIN_S for v in receiver],
            'receiver_longest_gap_ms':[max(v) if v else None for v in gaps],
            'lag_ms_min':[min(v) if v else None for v in lag],'lag_ms_median':[_median(v) for v in lag],'lag_ms_max':[max(v) if v else None for v in lag]}

def _paired(events,start,before,after,count):
    """Median before->after latency per bin, attributed to the later event."""
    first={};slots=[[] for _ in range(count)]
    for e in events:
        if e.get('stage')==before and e.get('frame_ns'):first[e['frame_ns']]=e['at_ns']
    for e in events:
        if e.get('stage')==after and e.get('frame_ns') in first and e['at_ns']>=first[e['frame_ns']]:
            _put(slots,(e['at_ns']-start)/1e9,(e['at_ns']-first[e['frame_ns']])/1e6)
    return [_median(v) for v in slots] if any(slots) else None

def _counted(events,start,stage,count,value=lambda e:1):
    slots=[0.0]*count;seen=False
    for e in events:
        if e.get('stage')!=stage:continue
        i=int((e['at_ns']-start)/1e9//BIN_S)
        if 0<=i<count:slots[i]+=value(e);seen=True
    return slots if seen else None

def stage_series(stages,count):
    out={}
    receiver=(stages or {}).get('receiver') or {};source=(stages or {}).get('source') or {}
    events=receiver.get('numeric_events');start=(receiver.get('window') or {}).get('start_ns')
    if events and start:
        gpu='gpu_completed' if any(e.get('stage')=='gpu_completed' for e in events) else 'presented'
        out['decode_ms']=_paired(events,start,'decoder_submitted','decoded',count)
        out['render_ms']=_paired(events,start,'decoded',gpu,count)
        decoded=_counted(events,start,'decoded',count)
        out['receiver_decoded_hz']=[v/BIN_S for v in decoded] if decoded else None
    events=source.get('numeric_events');start=(source.get('window') or {}).get('start_ns')
    if events and start:
        out['encode_ms']=_paired(events,start,'submitted','encoded',count)
        encoded=_counted(events,start,'encoded',count,lambda e:e.get('bytes',0))
        out['encoded_mbps']=[v*8/BIN_S/1e6 for v in encoded] if encoded else None
    return {k:v for k,v in out.items() if v is not None}

def resource_points(rows,start,seconds):
    """CPU (one core = 100%) and resident memory per sampler interval."""
    points=[];last=None
    for row in rows or []:
        if last and row['at_ns']>last['at_ns']:
            t=(row['at_ns']-start)/1e9
            previous={p['pid']:p['cpu_s'] for p in last['processes']}
            cpu=sum(max(0,p['cpu_s']-previous[p['pid']]) for p in row['processes'] if p['pid'] in previous)/((row['at_ns']-last['at_ns'])/1e9)*100
            if 0<=t<=seconds+1:points.append([round(t,3),round(cpu,1),round(sum(p['rss_bytes'] for p in row['processes'])/2**20,1)])
        last=row
    return points

def build(folder,row,source_index,seconds):
    """Write folder/timeline.json; returns it. Missing evidence leaves series out."""
    folder=Path(folder);count=_bins(seconds);series={}
    camera=folder/'camera'
    if source_index is not None and (camera/'optical-frames.jsonl').is_file():
        series.update(camera_series(camera,source_index,count) or {})
    series.update(stage_series(row.get('stages'),count))
    resources={}
    receiver_start=((row.get('stages') or {}).get('receiver') or {}).get('window',{}).get('start_ns')
    if receiver_start and (folder/'receiver-resources.json').is_file():
        resources['receiver']=resource_points(json.loads((folder/'receiver-resources.json').read_text()),receiver_start,seconds)
    if (folder/'source.json').is_file():
        source=json.loads((folder/'source.json').read_text())
        start=(source.get('stages') or {}).get('window',{}).get('start_ns') or (source.get('rows') or [{}])[0].get('at_ns')
        if start:resources['source']=resource_points(source.get('rows'),start,seconds)
    def tidy(values):return [None if v is None else round(v,2) for v in values]
    timeline={'schema_version':1,'bin_seconds':BIN_S,'bins':count,'series':{k:tidy(v) for k,v in series.items()},'resources':resources,
              'clocks':'Each series is zeroed at the scene start on its own clock: camera at the first positive source counter, '
                       'receiver at phase acknowledgement, source at its measurement window. Cross-clock alignment is approximate.'}
    (folder/'timeline.json').write_text(json.dumps(timeline,separators=(',',':')))
    return timeline


# ---- Report charts: inline SVG plus a small hover script, no dependencies ----

CHART_CSS='''
:root{--surface:#fcfcfb;--ink:#0b0b0b;--ink-2:#52514e;--ink-3:#8a8984;--grid:#e6e5e1;--rule:#ccc;--s1:#2a78d6;--s2:#eb6834;--s3:#1baf7a}
@media (prefers-color-scheme:dark){:root:not([data-theme="light"]){--surface:#1a1a19;--ink:#f0efec;--ink-2:#c3c2b7;--ink-3:#8f8e86;--grid:#2c2c2a;--rule:#444;--s1:#3987e5;--s2:#d95926;--s3:#199e70}}
:root[data-theme="dark"]{--surface:#1a1a19;--ink:#f0efec;--ink-2:#c3c2b7;--ink-3:#8f8e86;--grid:#2c2c2a;--rule:#444;--s1:#3987e5;--s2:#d95926;--s3:#199e70}
body{background:var(--surface);color:var(--ink)}td,th{border-color:var(--rule)!important}a{color:var(--s1)}
.tl-scene{margin:28px 0}.tl-scene>summary{font-weight:600;cursor:pointer}.tl-note{color:var(--ink-2);font-size:13px}
.tl-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(min(100%,460px),1fr));gap:20px 28px;margin-top:12px}
.tl-chart{position:relative}.tl-chart h3{font-size:14px;margin:0 0 2px;font-weight:600}
.tl-legend{display:flex;gap:14px;font-size:12px;color:var(--ink-2);margin-bottom:4px;min-height:16px}
.tl-legend i{display:inline-block;width:10px;height:3px;border-radius:2px;vertical-align:middle;margin-right:5px}
.tl-chart svg{display:block;width:100%;height:auto;overflow:visible}
.tl-chart text{font:11px system-ui;fill:var(--ink-3)}
.tl-tip{position:absolute;pointer-events:none;background:var(--surface);border:1px solid var(--rule);border-radius:6px;padding:6px 8px;font-size:12px;color:var(--ink);white-space:nowrap;display:none;box-shadow:0 2px 8px rgba(0,0,0,.12);z-index:2}
.tl-tip b{font-weight:600}.tl-tip span{color:var(--ink-2)}
'''

CHART_JS='''
document.querySelectorAll('.tl-chart').forEach(function(box){
  var svg=box.querySelector('svg'),tip=box.querySelector('.tl-tip'),line=svg.querySelector('.tl-cross'),spec=JSON.parse(box.dataset.spec);
  function hide(){tip.style.display='none';line.setAttribute('visibility','hidden')}
  svg.addEventListener('pointerleave',hide);
  svg.addEventListener('pointermove',function(ev){
    var r=svg.getBoundingClientRect(),scale=spec.w/r.width,x=(ev.clientX-r.left)*scale;
    if(x<spec.l||x>spec.w-spec.r){hide();return}
    var t=(x-spec.l)/(spec.w-spec.l-spec.r)*spec.t,rows=[];
    spec.s.forEach(function(s){var best=null,d=1e9;s.p.forEach(function(p){if(p[1]!==null&&Math.abs(p[0]-t)<d){d=Math.abs(p[0]-t);best=p}});
      if(best&&d<=spec.near)rows.push('<i style="background:'+s.c+';display:inline-block;width:8px;height:8px;border-radius:2px;margin-right:5px"></i><span>'+s.n+'</span> <b>'+best[1].toFixed(spec.d)+'</b>'+(best.length>2?' <span>('+best[2].toFixed(spec.d)+'–'+best[3].toFixed(spec.d)+')</span>':''))});
    if(!rows.length){hide();return}
    line.setAttribute('x1',x);line.setAttribute('x2',x);line.setAttribute('visibility','visible');
    tip.innerHTML='<span>'+t.toFixed(2)+' s</span><br>'+rows.join('<br>');tip.style.display='block';
    var px=(ev.clientX-box.getBoundingClientRect().left);tip.style.left=(px+tip.offsetWidth+16>box.clientWidth?px-tip.offsetWidth-12:px+12)+'px';tip.style.top='40px';
  });
});
'''

def _nice(value):
    if not value or value<=0:return 1
    step=10**math.floor(math.log10(value))
    for m in (1,2,2.5,5,10):
        if value<=m*step:return m*step

def chart(title,unit,seconds,series,band=None,digits=1):
    """series: [(label,css color,[(t,v)])]; band: (lower,upper) point lists drawn under series 0."""
    import html
    series=[(n,c,[(t,v) for t,v in pts if 0<=t<=seconds]) for n,c,pts in series]
    series=[item for item in series if item[2]]
    w,h,l,r,top,bottom=480,150,40,8,8,22
    values=[v for _,_,pts in series for _,v in pts if v is not None]+([v for pts in band for _,v in pts if v is not None] if band else [])
    if not values:return ''
    ymax=4*_nice(max(values)*1.05/4);pw,ph=w-l-r,h-top-bottom
    X=lambda t:l+min(max(t,0),seconds)/seconds*pw;Y=lambda v:top+ph-v/ymax*ph
    out=[f'<svg viewBox="0 0 {w} {h}" role="img" aria-label="{html.escape(title)}">']
    for i in range(5):
        v=ymax*i/4;y=Y(v)
        out.append(f'<line x1="{l}" x2="{w-r}" y1="{y:.1f}" y2="{y:.1f}" stroke="var(--grid)" stroke-width="1"/>')
        out.append(f'<text x="{l-6}" y="{y+3.5:.1f}" text-anchor="end">{v:g}</text>')
    step=5 if seconds>15 else 2 if seconds>6 else 1
    for t in range(0,int(seconds)+1,step):
        out.append(f'<text x="{X(t):.1f}" y="{h-6}" text-anchor="middle">{t}s</text>')
    def path(pts):
        d=[];pen=False
        for t,v in pts:
            if v is None:pen=False;continue
            d.append(('L' if pen else 'M')+f'{X(t):.1f},{Y(v):.1f}');pen=True
        return ''.join(d)
    if band:
        lower,upper=band;segment=[]
        def flush():
            if len(segment)>1:
                d='M'+'L'.join(f'{X(t):.1f},{Y(hi):.1f}' for t,_,hi in segment)+'L'+'L'.join(f'{X(t):.1f},{Y(lo):.1f}' for t,lo,_ in reversed(segment))+'Z'
                out.append(f'<path d="{d}" fill="{series[0][1]}" fill-opacity=".18" stroke="none"/>')
        for (t,lo),(_,hi) in zip(lower,upper):
            if lo is None or hi is None:flush();segment.clear()
            else:segment.append((t,lo,hi))
        flush()
    for _,color,pts in series:
        out.append(f'<path d="{path(pts)}" fill="none" stroke="{color}" stroke-width="2" stroke-linejoin="round" stroke-linecap="round"/>')
    out.append(f'<line class="tl-cross" y1="{top}" y2="{top+ph}" stroke="var(--ink-3)" stroke-width="1" visibility="hidden"/></svg>')
    spec_series=[]
    for i,(label,color,pts) in enumerate(series):
        p=[[t,v] for t,v in pts]
        if band and i==0:p=[[t,v,lo,hi] if v is not None and lo is not None and hi is not None else [t,v] for (t,v),(_,lo),(_,hi) in zip(pts,*band)]
        spec_series.append({'n':label,'c':color,'p':p})
    near=max(BIN_S,max((b[0]-a[0] for _,_,pts in series for a,b in zip(pts,pts[1:])),default=BIN_S))
    spec={'w':w,'l':l,'r':r,'t':seconds,'d':digits,'near':near,'s':spec_series}
    legend=''.join(f'<span><i style="background:{c}"></i>{html.escape(n)}</span>' for n,c,_ in series) if len(series)>1 else ''
    return (f'<div class="tl-chart" data-spec="{html.escape(json.dumps(spec,separators=(",",":")))}"><h3>{html.escape(title)} <span class="tl-note">{html.escape(unit)}</span></h3>'
            f'<div class="tl-legend">{legend}</div>'+''.join(out)+'<div class="tl-tip"></div></div>')

def scene_html(timeline,seconds,link):
    s=timeline['series'];bin_s=timeline['bin_seconds']
    def pts(name):return [(i*bin_s+bin_s/2,v) for i,v in enumerate(s[name])] if name in s else None
    def present(*items):return [(n,c,p) for n,c,p in items if p]
    charts=[]
    charts.append(chart('Visible updates','per second, seen by camera',seconds,present(('Source screen','var(--s1)',pts('source_updates_hz')),('Receiver screen','var(--s2)',pts('receiver_updates_hz'))),digits=0))
    if 'lag_ms_median' in s:
        charts.append(chart('Screen-to-screen lag','ms; line median, band min–max',seconds,[('Lag','var(--s1)',pts('lag_ms_median'))],band=(pts('lag_ms_min'),pts('lag_ms_max'))))
    charts.append(chart('Longest receiver gap','ms between visible updates',seconds,present(('Longest gap','var(--s2)',pts('receiver_longest_gap_ms')))))
    charts.append(chart('Pipeline time per frame','ms, median',seconds,present(('Encode (source)','var(--s1)',pts('encode_ms')),('Decode','var(--s2)',pts('decode_ms')),('Render to GPU done','var(--s3)',pts('render_ms'))),digits=2))
    charts.append(chart('Encoded video','Mbps',seconds,present(('Bitrate','var(--s1)',pts('encoded_mbps')))))
    res=timeline.get('resources',{})
    def rpts(role,index):return [(p[0],p[index]) for p in res.get(role) or []] or None
    charts.append(chart('CPU','% of one core',seconds,present(('Source app','var(--s1)',rpts('source',1)),('Receiver app','var(--s2)',rpts('receiver',1)))))
    charts.append(chart('Memory','MB resident',seconds,present(('Source app','var(--s1)',rpts('source',2)),('Receiver app','var(--s2)',rpts('receiver',2))),digits=0))
    charts=[c for c in charts if c]
    if not charts:return ''
    return ('<div class="tl-grid">'+''.join(charts)+'</div>'
            f'<p class="tl-note">{BIN_NOTE.format(bin_ms=int(bin_s*1000))} <a href="{link}">Data (timeline.json)</a></p>')

BIN_NOTE='Values per {bin_ms} ms bin; CPU and memory per sampler interval. Each host and the camera are zeroed at the scene start on their own clocks, so cross-chart alignment is approximate.'

def report_html(root,report):
    import html
    seconds=report['settings']['seconds'];parts=[]
    for index,phase in enumerate(report['phases']):
        name=f"{phase['repetition']:02}-{phase['scene']}";path=Path(root)/name/'timeline.json'
        if not path.is_file():continue
        try:body=scene_html(json.loads(path.read_text()),seconds,f'{name}/timeline.json')
        except (OSError,ValueError,KeyError,TypeError):continue
        if body:parts.append(f'<details class="tl-scene"{" open" if index==0 else ""}><summary>Repeat {phase["repetition"]} · {html.escape(phase["scene"])}</summary>{body}</details>')
    if not parts:return ''
    return '<h2>Scene timelines</h2>'+''.join(parts)+'<script>'+CHART_JS+'</script>'
