"""Modelled optical signal alignment, separate from decoded-counter delay bounds.

Uses observed complementary-cell intensities. Never fills missing counter IDs or
claims physical input latency, exact visible FPS, or a per-frame delay distribution.
"""
import json
from pathlib import Path
import numpy as np


def align(rows, source_index, time_factor, maximum_delay_ms=500):
    if source_index not in (0,1) or not 0<time_factor<=1:raise ValueError('Invalid optical orientation or time scale')
    if not 20<=maximum_delay_ms<=1000:raise ValueError('Invalid bounded delay search')
    if len(rows)<120:raise ValueError('Insufficient observed signal samples')
    times=np.asarray([r['playback_s']*time_factor for r in rows],dtype=float)
    if not np.isfinite(times).all() or np.any(np.diff(times)<=0):raise ValueError('Invalid signal timestamps')
    cadence=float(np.median(np.diff(times)))
    if not 1/300<=cadence<=1/90:raise ValueError('Signal cadence outside supported camera range')
    signals=[];masks=[]
    for role in (source_index,1-source_index):
        cells=np.asarray([r['patches'][role] for r in rows],dtype=float)
        if cells.shape!=(len(rows),2,16) or not np.isfinite(cells).all():raise ValueError('Invalid cell signal shape')
        valid=np.asarray([len(r.get('tracked_corners',[[],[]])[role])==4 for r in rows])
        if np.mean(valid)<.95:raise ValueError('Signal markers below tracking gate')
        signals.append(cells[:,0,:]-cells[:,1,:]);masks.append(valid)
    means=[np.mean(signal[mask],axis=0) for signal,mask in zip(signals,masks)]
    deviations=[np.std(signal[mask],axis=0) for signal,mask in zip(signals,masks)]
    channels=(deviations[0]>=20)&(deviations[1]>=20)
    if np.count_nonzero(channels)<4:raise ValueError('Insufficient varying optical channels')
    source,receiver=[(signal[:,channels]-mean[channels])/deviation[channels]
                     for signal,mean,deviation in zip(signals,means,deviations)]
    curves=[]
    for step in range(int(maximum_delay_ms/1000/cadence)+1):
        delay=step*cadence;target=times-delay
        right=np.clip(np.searchsorted(times,target),0,len(times)-1);left=np.maximum(0,right-1)
        nearest=np.where(np.abs(times[left]-target)<=np.abs(times[right]-target),left,right)
        valid=masks[1]&masks[0][nearest]&(np.abs(times[nearest]-target)<=cadence*.8)
        pairs=int(np.count_nonzero(valid))
        if pairs<len(rows)*.8:continue
        a=source[nearest[valid]];b=receiver[valid]
        denominator=np.sqrt(np.sum(a*a)*np.sum(b*b))
        if denominator<=0:continue
        curves.append({'delay_ms':delay*1000,'correlation':float(np.sum(a*b)/denominator),'paired_samples':pairs})
    if not curves:raise ValueError('Insufficient shared signal observations')
    best=max(curves,key=lambda item:item['correlation'])
    near=[item['delay_ms'] for item in curves if item['correlation']>=best['correlation']-.02]
    alternatives=[item['correlation'] for item in curves if abs(item['delay_ms']-best['delay_ms'])>=33]
    margin=best['correlation']-max(alternatives) if alternatives else None
    strong=best['correlation']>=.8 and margin is not None and margin>=.1
    return {'status':'modelled_alignment' if strong else 'ambiguous_alignment','estimated_constant_delay_ms':best['delay_ms'],
        'correlation':best['correlation'],'distant_peak_margin':margin,'near_peak_delay_range_ms':[min(near),max(near)],
        'camera_sample_ms':cadence*1000,'nearest_sample_quantization_ms':cadence*500,
        'varying_channels':int(np.count_nonzero(channels)),'source_tracking_coverage':float(np.mean(masks[0])),
        'receiver_tracking_coverage':float(np.mean(masks[1])),'curve':curves,
        'limits':['Constant-delay signal model over the selected scene, not a per-frame latency distribution or input-to-photon measurement.',
                  'No decoded counter IDs are inferred, repaired or interpolated. Frame cadence still requires separate evidence.',
                  'Nearest observed samples introduce quantization; exposure, rolling shutter and panel scanout add uncalibrated bias.',
                  'The near-peak range is a score threshold, not a statistical confidence interval.',
                  'Variable lag, frame holds and repeated patterns can weaken or bias alignment; ambiguous results are retained.']}


def analyze(folder, source_index):
    folder=Path(folder);summary=json.loads((folder/'summary.json').read_text())
    fps=(summary.get('video_metadata') or {}).get('nominal_playback_fps');hz=summary.get('measured_sensor_hz')
    if type(fps) not in (int,float) or type(hz) not in (int,float) or not 0<fps<=hz:
        raise ValueError('Missing or invalid camera timescale metadata')
    factor=fps/hz
    rows=[json.loads(line) for line in (folder/'cells.jsonl').read_text().splitlines()]
    window=json.loads((folder/'optical-window.json').read_text())
    return align(rows[window['first_sample']:window['last_sample']+1],source_index,factor)
