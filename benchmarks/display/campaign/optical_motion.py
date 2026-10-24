"""Continuous physical pointer motion, observed in one camera timeline.

This measures resolvable motion and holds, not FPS or hardware input latency.
A stalled receiver is a measured outcome, not an excuse to reject the sample.
"""
import json
import math
from pathlib import Path
from metrics import stats


def position(value):
    if not isinstance(value, dict) or set(value) != {'x', 'width', 'spatial_bound'}:
        return None
    if any(type(v) not in (int, float) or not math.isfinite(v) for v in value.values()):
        return None
    if not 24 <= value['x'] <= 352 or not 6 <= value['width'] <= 28 or not 2 <= value['spatial_bound'] <= 6:
        return None
    return value


def crossing_delay(rows, times, source_index, receiver_holds):
    """Bound observed crossings; never synthesize an interpolated timestamp.

    Bounds are detector centroid uncertainty, not a calibrated physical error.
    Interior landmarks and a readable, monotonic 12-pixel guard reject reversals.
    """
    landmarks = (96, 160, 224, 288)
    values = [[], []]
    for row in rows:
        pair = row.get('cursor_positions')
        for i in (0, 1):
            values[i].append(position(pair[i]) if isinstance(pair, list) and len(pair) == 2 else None)

    def crossings(observations, holds):
        accepted = []
        rejected = {'missing_guard': 0, 'reversal_guard': 0, 'hold_or_wide_interval': 0}
        for x in landmarks:
            side = None
            anchor = None
            ordinal = {-1: 0, 1: 0}
            for i, value in enumerate(observations):
                if value is None:
                    side = anchor = None
                    continue
                definite = (-1 if value['x'] + value['spatial_bound'] < x else
                            1 if value['x'] - value['spatial_bound'] > x else 0)
                if not definite:
                    continue
                if side is not None and definite != side:
                    direction = definite
                    ordinal[direction] += 1
                    lo, hi = times[anchor], times[i]
                    reason = None
                    if hi-lo > .1 or any(lo <= h['end_s'] and hi >= h['start_s'] for h in holds):
                        reason = 'hold_or_wide_interval'
                    # Find actual observations at least 12 pixels on each side.
                    # Stop at unreadable data or at 150 ms, including after crossing.
                    left, right = anchor, i
                    while left > 0 and direction*(x-observations[left]['x']) < 12:
                        if times[anchor]-times[left-1] > .15 or observations[left-1] is None:
                            break
                        left -= 1
                    while right+1 < len(times) and direction*(observations[right]['x']-x) < 12:
                        if times[right+1]-times[i] > .15 or observations[right+1] is None:
                            break
                        right += 1
                    guard = observations[left:right+1]
                    if any(v is None for v in guard) or (direction*(x-guard[0]['x']) < 12 or
                                                       direction*(guard[-1]['x']-x) < 12):
                        reason = reason or 'missing_guard'
                    elif any(direction*(b['x']-a['x']) < -(a['spatial_bound']+b['spatial_bound'])
                             for a, b in zip(guard, guard[1:])):
                        reason = reason or 'reversal_guard'
                    if reason:
                        rejected[reason] += 1
                    else:
                        accepted.append({'x': x, 'direction': direction,
                                         'cycle': ordinal[direction], 'lower_s': lo, 'upper_s': hi})
                side, anchor = definite, i
        return accepted, rejected

    source, source_rejected = crossings(values[source_index], [])
    receiver, receiver_rejected = crossings(values[1-source_index], receiver_holds)
    candidates = []
    for s in source:
        candidates.append([j for j, r in enumerate(receiver)
                           if s['x'] == r['x'] and s['direction'] == r['direction'] and
                           (r['lower_s']-s['upper_s'])*1000 >= -125 and
                           (r['upper_s']-s['lower_s'])*1000 <= 500])
    # Mutual uniqueness prevents one receiver event serving two source cycles.
    # Ordinals are diagnostic only: occlusion must not shift all later cycle IDs.
    multiplicity = [sum(j in c for c in candidates) for j in range(len(receiver))]
    matches = []
    ambiguous = unmatched = 0
    for s, choices in zip(source, candidates):
        if not choices:
            unmatched += 1
            continue
        if len(choices) != 1 or multiplicity[choices[0]] != 1:
            ambiguous += 1
            continue
        r = receiver[choices[0]]
        lower = (r['lower_s']-s['upper_s'])*1000
        upper = (r['upper_s']-s['lower_s'])*1000
        matches.append({'x': s['x'], 'direction': 'right' if s['direction'] == 1 else 'left',
                        'source_cycle': s['cycle'], 'receiver_cycle': r['cycle'],
                        'source_interval_s': [s['lower_s'], s['upper_s']],
                        'receiver_interval_s': [r['lower_s'], r['upper_s']],
                        'lower_ms': lower, 'upper_ms': upper, 'midpoint_ms': (lower+upper)/2})
    return {'status': 'measured' if len(matches) >= 8 and not ambiguous else 'insufficient_or_ambiguous',
            'method': 'direction-aware-fixed-x-observed-interval-v1',
            'landmarks_logical_pixels': list(landmarks), 'matching_window_ms': [-125, 500],
            'source_crossings': len(source), 'receiver_crossings': len(receiver),
            'source_crossings_before_guards': len(source)+sum(source_rejected.values()),
            'receiver_crossings_before_guards': len(receiver)+sum(receiver_rejected.values()),
            'matched_crossings': len(matches), 'unmatched_source_crossings': unmatched,
            'ambiguous_source_crossings': ambiguous,
            'unmatched_receiver_crossings': len(receiver)-len(matches),
            'matched_source_coverage': len(matches)/len(source) if source else 0,
            'matched_detected_source_coverage': len(matches)/(len(source)+sum(source_rejected.values()))
                if source or any(source_rejected.values()) else 0,
            'guard_policy': {'travel_each_side_pixels': 12, 'maximum_guard_extension_ms': 150,
                             'maximum_crossing_interval_ms': 100},
            'source_rejected': source_rejected, 'receiver_rejected': receiver_rejected,
            'midpoint_ms': stats([m['midpoint_ms'] for m in matches]),
            'lower_ms': stats([m['lower_ms'] for m in matches]),
            'upper_ms': stats([m['upper_ms'] for m in matches]),
            'interval_width_ms': stats([m['upper_ms']-m['lower_ms'] for m in matches]),
            'crossings': matches,
            'limits': ['Observed software pointer movement delay, not calibrated input or transport latency.',
                       'Interval endpoints use actual readable frames and detector centroid bounds; no interpolation.',
                       'Missing observations, observed holds, wide crossings and reversals are excluded and counted.',
                       'Coverage is conditional on detected crossings; unreadable resets can erase entire crossing candidates.',
                       'Matched crossings are correlated within one clip; they are not independent repeat runs.',
                       'Panel scanout, camera exposure and rolling shutter remain; centroid bounds are not physical calibration.']}


def summarize(rows, factor, hz, source_index):
    if source_index not in (0, 1) or not 100 <= hz <= 1000 or not 0 < factor <= 1:
        raise ValueError('Unverified continuous cursor timing context')
    times = []
    observations = [[], []]
    bounds = []
    for row in rows:
        t = row['playback_s'] * factor
        if not math.isfinite(t) or (times and (t <= times[-1] or t-times[-1] > .05)):
            raise ValueError('Invalid continuous cursor camera timestamps')
        times.append(t)
        values = row.get('cursor_positions')
        for i in (0, 1):
            value = position(values[i]) if isinstance(values, list) and len(values) == 2 else None
            observations[i].append(value['x'] if value else None)
            if value:
                bounds.append(value['spatial_bound'])
    if len(times) < hz*3:
        raise ValueError('Insufficient continuous cursor observation duration')
    coverage = [sum(v is not None for v in values)/len(times) for values in observations]
    joint = sum(a is not None and b is not None for a, b in zip(*observations))/len(times)
    if min(coverage) < .8 or joint < .8:
        raise ValueError('Insufficient readable continuous cursor coverage')
    source = observations[source_index]
    receiver = observations[1-source_index]
    source_values = [v for v in source if v is not None]
    if max(source_values)-min(source_values) < 200:
        raise ValueError('Source cursor sweep not observed')
    spatial_bound = max(bounds)
    threshold = max(6, 2*spatial_bound)

    def motion(values):
        anchor = None
        changes = []
        obscured_change_gaps = []
        last_change_index = None
        holds = []
        missing_runs = []
        missing_start = None
        for i, value in enumerate(values):
            if value is None:
                if missing_start is None:
                    missing_start = times[i]
                anchor = None
                continue
            if missing_start is not None:
                missing_runs.append((times[i]-missing_start)*1000)
                missing_start = None
            if anchor is None:
                anchor = i
                continue
            if abs(value-values[anchor]) >= threshold or i == len(values)-1:
                elapsed = times[i]-times[anchor]
                visible_source = [v for v in source[anchor:i+1] if v is not None]
                source_span = max(visible_source)-min(visible_source) if visible_source else 0
                if elapsed >= .1 and source_span >= max(20, threshold*3):
                    holds.append({'start_s':times[anchor], 'end_s':times[i],
                                  'duration_ms':elapsed*1000, 'source_travel_pixels':source_span})
                if abs(value-values[anchor]) >= threshold:
                    if last_change_index is not None and any(v is None for v in values[last_change_index:i+1]):
                        obscured_change_gaps.append((times[i]-times[last_change_index])*1000)
                    changes.append(times[i])
                    last_change_index = i
                anchor = i
        if missing_start is not None:
            missing_runs.append((times[-1]-missing_start)*1000)
        return {'resolvable_changes':len(changes),
                'inter_resolvable_change_ms':stats([(b-a)*1000 for a,b in zip(changes,changes[1:])]),
                'observed_holds':holds, 'unreadable_gap_ms':stats(missing_runs),
                'change_gaps_containing_unreadable_frames_ms':stats(obscured_change_gaps)}

    source_motion = motion(source)
    if source_motion['observed_holds']:
        raise ValueError('Source cursor reference stalled during sweep')
    receiver_motion = motion(receiver)
    return {'status':'measured', 'pattern':'horizontal-yellow-sweep-v1',
            'readable_coverage':coverage, 'joint_readable_coverage':joint,
            'duration_s':times[-1]-times[0], 'source_span_pixels':max(source_values)-min(source_values),
            'spatial_bound_pixels':spatial_bound, 'movement_threshold_pixels':threshold,
            'source':source_motion, 'receiver':receiver_motion,
            'longest_observed_receiver_hold_ms':max((v['duration_ms'] for v in receiver_motion['observed_holds']),default=0),
            'camera_sample_period_ms':1000/hz,
            'crossing_delay':crossing_delay(rows, times, source_index, receiver_motion['observed_holds']),
            'limits':['Software pointer sweep, not physical mouse input latency.',
                      'Changes require displacement above the spatial uncertainty; counts are not cursor FPS.',
                      'Holds require at least 100 ms without resolvable pointer movement while the source moves at least 20 pixels.',
                      'Hold boundaries include the movement threshold and camera sampling uncertainty.',
                      'Unreadable intervals reset tracking and remain disclosed; no interpolation across missing observations.',
                      'Zero confirmed holds is a lower bound, not proof of no stalls; change gaps crossing unreadable frames remain unresolved.',
                      'Source reference panel scanout, camera exposure and rolling shutter remain in the observation.']}


def optical_motion(directory, source_index):
    root = Path(directory)
    context = json.loads((root/'summary.json').read_text())
    hz = context['measured_sensor_hz']
    playback = context['video_metadata']['nominal_playback_fps']
    if not 20 <= playback <= 60:
        raise ValueError('Unverified continuous cursor playback timing')
    rows = [json.loads(line) for line in (root/'optical-frames.jsonl').read_text().splitlines()]
    return summarize(rows, playback/hz, hz, source_index)
