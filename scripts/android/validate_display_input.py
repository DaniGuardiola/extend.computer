#!/usr/bin/env python3
"""Validate one physical display-input interval from native window receipts.

This reads existing evidence only. It never connects to a device or injects input.
Passing proves the requested observed window events, not latency or lease security.
"""
import argparse
import json
import math
from collections import Counter
from pathlib import Path


ACTION_KINDS = ('down', 'up', 'drag', 'scroll')
KINDS = ('tap', 'double-tap', 'hold-drag', 'two-finger-tap',
         'two-finger-scroll', 'cancel-drag', 'home-drag', 'blocked')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def read(path):
    return json.loads(path.read_text())


def number(value):
    require(isinstance(value, (int, float)) and not isinstance(value, bool)
            and math.isfinite(value), 'Expected a finite number')
    return value


def rectangle(value):
    require(isinstance(value, dict), 'Missing rectangle')
    result = {key: number(value[key]) for key in ('x', 'y', 'width', 'height')}
    require(result['width'] > 0 and result['height'] > 0, 'Empty rectangle')
    return result


def same_rectangle(a, b):
    return all(abs(a[key] - b[key]) < 0.001 for key in a)


def window(snapshot, role):
    matches = [item for item in snapshot['windows'] if item['role'] == role]
    require(len(matches) == 1, f'Expected one {role} window')
    return matches[0]


def delta(before, after):
    keys = set(before.get('counters', {})) | set(after.get('counters', {}))
    result = {key: after.get('counters', {}).get(key, 0)
              - before.get('counters', {}).get(key, 0) for key in keys}
    require(all(value >= 0 for value in result.values()), 'Observer counters reset')
    return result


def validate(before, after, descriptor, kind, zone_number, min_drag_distance=20,
             injection=None, package=None, viewport=None, input_state=None, binding=None):
    require(kind in KINDS, 'Unknown gesture kind')
    require(min_drag_distance > 0, 'Drag threshold must be positive')
    for snapshot in (before, after):
        require(snapshot.get('event_source') == 'native-window-events',
                'Expected native observer evidence')
        require(snapshot.get('status') in ('ready', 'observing', 'stopped', 'duration-ended')
                or (snapshot is after and kind == 'home-drag'
                    and snapshot.get('status') == 'display-changed'),
                'Observer ended with an invalid target or failed output')
        require(snapshot['target_display_id'] != snapshot['main_display_id'],
                'Extended-target acceptance requires a separate main guard')
        require(snapshot['target_display_id'] == descriptor['display_id'],
                'Descriptor does not identify the observed capture target')
    require(before['main_display_id'] == after['main_display_id'], 'Main display changed')
    revision = descriptor.get('configuration_id', descriptor.get('revision'))
    require(isinstance(revision, int) and not isinstance(revision, bool)
            and revision >= 0, 'Missing capture configuration ID or scoped revision')
    if 'revision' in descriptor:
        require(descriptor['revision'] == revision, 'Descriptor capture revisions disagree')
    scoped_binding = {key: descriptor[key] for key in ('session', 'epoch', 'revision') if key in descriptor}
    if scoped_binding:
        require(len(scoped_binding) == 3 and isinstance(scoped_binding['session'], str)
                and scoped_binding['session'] and isinstance(scoped_binding['epoch'], int)
                and scoped_binding['epoch'] >= 0, 'Incomplete scoped descriptor binding')
        if binding is not None:
            require(scoped_binding == binding, 'Scoped descriptor differs from expected binding')
    bounds = rectangle(descriptor['bounds'])
    content = rectangle(descriptor['content'])
    canvas = descriptor['canvas']
    require(number(canvas['width']) > 0 and number(canvas['height']) > 0, 'Invalid canvas')
    require(content['x'] >= 0 and content['y'] >= 0
            and content['x'] + content['width'] <= canvas['width'] + 0.001
            and content['y'] + content['height'] <= canvas['height'] + 0.001,
            'Captured content is outside the canvas')
    if input_state is not None:
        require(input_state.get('state') == 'granted', 'Initial display input was not granted')
        granted_binding = input_state['binding']
        require(isinstance(granted_binding['session'], str) and granted_binding['session']
                and isinstance(granted_binding['epoch'], int) and granted_binding['epoch'] >= 0,
                'Invalid granted input binding')
        require(input_state['revision'] == granted_binding['revision'] == revision,
                'Input grant uses a different capture revision')
        if scoped_binding:
            require(all(granted_binding[key] == scoped_binding[key] for key in scoped_binding),
                    'Input grant differs from the scoped source descriptor binding')
        if binding is not None:
            require(all(granted_binding[key] == binding[key] for key in ('session', 'epoch', 'revision')),
                    'Input session, epoch or revision differs from the expected binding')
        geometry = input_state['geometry']
        require(geometry['canvas'] == canvas and ('raster' not in descriptor
                or geometry['raster'] == descriptor['raster']),
                'Granted input canvas or raster differs from capture target')
        require(geometry['source_size'] == {'width': bounds['width'], 'height': bounds['height']},
                'Granted input source size differs from capture target')
        normalized_content = {key: content[key] / canvas['width' if key in ('x', 'width') else 'height']
                              for key in ('x', 'y', 'width', 'height')}
        require(same_rectangle(normalized_content, rectangle(geometry['content'])),
                'Granted fitted content differs from capture target')
    else:
        require(binding is None or scoped_binding, 'Expected binding requires an input-state or scoped descriptor receipt')
    old_target, target = window(before, 'target'), window(after, 'target')
    old_guard, guard = window(before, 'main-guard'), window(after, 'main-guard')
    for old, new in ((old_target, target), (old_guard, guard)):
        require(old['window_number'] == new['window_number'], 'Observer window changed')
        require(old['expected_display_id'] == new['actual_display_id'] == new['expected_display_id']
                == old['actual_display_id'], 'Observer window moved between displays')
        for key in ('source_display_bounds', 'source_content_bounds'):
            require(same_rectangle(rectangle(old[key]), rectangle(new[key])),
                    'Observer geometry changed; start a fresh interval')
    require(target['actual_display_id'] == descriptor['display_id'], 'Wrong target display')
    require(guard['actual_display_id'] == after['main_display_id'], 'Wrong main guard display')
    require(same_rectangle(bounds, rectangle(target['source_display_bounds'])),
            'Capture source bounds differ from observer display bounds')
    target_delta, guard_delta = delta(old_target, target), delta(old_guard, guard)
    require(all(guard_delta.get(key, 0) == 0 for key in ACTION_KINDS),
            'Main guard received click, drag or scroll input')
    start, end = before['sequence'], after['sequence']
    require(isinstance(start, int) and isinstance(end, int) and 0 <= start <= end,
            'Invalid event interval')
    events = [event for event in after['events'] if start < event['sequence'] <= end]
    require([event['sequence'] for event in events] == list(range(start + 1, end + 1)),
            'Native event interval is incomplete or out of order')
    for role, counts in (('target', target_delta), ('main-guard', guard_delta)):
        recorded = Counter(event['kind'] for event in events if event['role'] == role)
        require(all(recorded[key] == counts.get(key, 0) for key in set(counts) | set(recorded)),
                f'{role} counters disagree with retained native events')
    actions = [event for event in events if event['role'] == 'target'
               and event['kind'] in ACTION_KINDS]
    zones = [zone for zone in target['zones'] if zone['number'] == zone_number]
    require(len(zones) == 1, 'Requested observer zone is missing')
    zone = zones[0]
    for axis in ('x', 'y'):
        size_axis = 'width' if axis == 'x' else 'height'
        require(abs(number(zone['source_normalized'][axis])
                    - (number(zone['global_center'][axis]) - bounds[axis]) / bounds[size_axis]) < 0.000001,
                'Zone normalized center does not match actual source bounds')
    for event in actions:
        require(event['actual_display_id'] == descriptor['display_id']
                and event['window_number'] == target['window_number'], 'Wrong event target')
        require(event['zone'] == zone_number, 'Action landed outside the requested zone')
        point, local = event['global_point'], event['local_point']
        for axis in ('x', 'y'):
            require(abs(number(point[axis]) - (target['source_content_bounds'][axis]
                        + number(local[axis]))) < 1, 'Local/global event position mismatch')
        require(bounds['x'] <= point['x'] <= bounds['x'] + bounds['width']
                and bounds['y'] <= point['y'] <= bounds['y'] + bounds['height'],
                'Event escaped the scoped capture bounds')
    held = set()
    held_clicks = {}
    clicks = []
    for event in actions:
        button = event['button']
        if event['kind'] == 'down':
            require(button not in held, 'Button pressed twice without release')
            held.add(button)
            held_clicks[button] = event['click_count']
            clicks.append((button, event['click_count']))
        elif event['kind'] == 'up':
            require(button in held, 'Button release has no observed press')
            require(event['click_count'] == held_clicks[button], 'Press/release click counts differ')
            held.remove(button)
        elif event['kind'] == 'drag':
            require(button in held, 'Drag delivered without an observed held button')
    require(not held, 'Held button was not released in the observed interval')
    if kind == 'blocked':
        require(injection is not None, 'Blocked input requires an actual Android tap attempt receipt')
        require(not actions, 'Blocked input delivered native actions')
    elif kind == 'two-finger-scroll':
        require(not clicks and target_delta.get('drag', 0) == 0,
                'Two-finger scroll produced a button or drag action')
        require(any(abs(number(event['scroll']['dx'])) + abs(number(event['scroll']['dy'])) > 0
                    for event in actions if event['kind'] == 'scroll'), 'No actual scroll delta')
    else:
        expected = [(0, 1), (0, 2)] if kind == 'double-tap' else [(1, 1)] if kind == 'two-finger-tap' else [(0, 1)]
        require(clicks == expected, 'Unexpected native button or click counts')
        require(target_delta.get('scroll', 0) == 0, 'Click gesture produced scroll input')
        if kind in ('hold-drag', 'cancel-drag', 'home-drag'):
            down = next(event for event in actions if event['kind'] == 'down')['global_point']
            require(any(math.hypot(event['global_point']['x'] - down['x'],
                                   event['global_point']['y'] - down['y']) >= min_drag_distance
                        for event in actions if event['kind'] == 'drag'),
                    'No observed drag displacement above the requested threshold')
    if injection is not None:
        require(injection.get('status') == 'passed', 'Android gesture injection failed')
        require(injection.get('gesture') == ('tap' if kind == 'blocked' else kind),
                'Injection gesture differs from the observed interval')
        injected = injection['gestureEvents']
        if isinstance(injected, str):
            injected = json.loads(injected)
        require(injected and all(event.get('accepted') is True for event in injected),
                'Android did not accept every injected touchscreen event')
    if kind == 'home-drag':
        require(injection is not None and package, 'Home release requires injection and target package')
        require(str(injection.get('home')).lower() == 'true'
                and isinstance(injection.get('foregroundAfter'), str)
                and injection['foregroundAfter'] and injection['foregroundAfter'] != package,
                'Android HOME transition was not observed')
    normalized = {axis: content[axis] / canvas['width' if axis == 'x' else 'height']
                  + zone['source_normalized'][axis] * content['width' if axis == 'x' else 'height']
                  / canvas['width' if axis == 'x' else 'height'] for axis in ('x', 'y')}
    output = {'status': 'passed', 'kind': kind, 'display_id': descriptor['display_id'],
              'configuration_id': revision, 'sequence_interval': [start, end],
              'target_counter_delta': target_delta, 'main_guard_counter_delta': guard_delta,
              'zone': zone_number, 'source_global_center': zone['global_center'],
              'canvas_normalized_center': normalized,
              'limitations': ['Native window delivery only; not optical latency.',
                              'Touch toggle, foreground lease state and stale revision rejection require separate native/UI receipts.']}
    if input_state is not None:
        output['initial_granted_binding'] = input_state['binding']
    if scoped_binding:
        output['scoped_source_binding'] = scoped_binding
    if viewport:
        viewport = rectangle(viewport)
        output['android_pixel_center'] = {axis: viewport[axis] + normalized[axis]
                                         * viewport['width' if axis == 'x' else 'height']
                                         for axis in ('x', 'y')}
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', type=Path, required=True)
    parser.add_argument('--after', type=Path, required=True)
    parser.add_argument('--descriptor', type=Path, required=True,
                        help='Raw CaptureTarget/InputDescriptor JSON for this configuration')
    parser.add_argument('--kind', choices=KINDS, required=True)
    parser.add_argument('--zone', type=int, required=True)
    parser.add_argument('--min-drag-distance', type=float, default=20,
                        help='Minimum actual source logical drag displacement')
    parser.add_argument('--injection', type=Path)
    parser.add_argument('--package', help='Expected Android app package for HOME evidence')
    parser.add_argument('--input-state', type=Path, help='Initial native granted input-state JSON')
    parser.add_argument('--binding', type=Path, help='Expected session/epoch/revision JSON')
    parser.add_argument('--viewport', nargs=4, type=float, metavar=('X', 'Y', 'WIDTH', 'HEIGHT'),
                        help='Separately observed fitted video rectangle in Android pixels')
    args = parser.parse_args()
    try:
        result = validate(read(args.before), read(args.after), read(args.descriptor),
                          args.kind, args.zone, args.min_drag_distance,
                          read(args.injection) if args.injection else None, args.package,
                          dict(zip(('x', 'y', 'width', 'height'), args.viewport)) if args.viewport else None,
                          read(args.input_state) if args.input_state else None,
                          read(args.binding) if args.binding else None)
    except (ValueError, KeyError, TypeError, OSError) as error:
        print(json.dumps({'status': 'failed', 'error': str(error)}, indent=2))
        raise SystemExit(1)
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
