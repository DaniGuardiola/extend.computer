#!/usr/bin/env python3
"""Offline evidence-validator regressions; no device access or input injection."""
import copy
import unittest
from pathlib import Path

from validate_display_input import read, validate


class EvidenceValidation(unittest.TestCase):
    def setUp(self):
        directory = Path(__file__).parent / 'fixtures/display-input'
        self.before = read(directory / 'before.json')
        self.after = read(directory / 'after-tap.json')
        self.descriptor = read(directory / 'descriptor.json')

    def check(self, kind='tap', **kwargs):
        return validate(self.before, self.after, self.descriptor, kind, 1, **kwargs)

    def test_historical_real_tap_and_fitted_point(self):
        result = self.check(viewport={'x': 0, 'y': 0, 'width': 2560, 'height': 1600})
        self.assertEqual(result['target_counter_delta']['down'], 1)
        self.assertAlmostEqual(result['android_pixel_center']['x'], 442.6666667)
        self.assertAlmostEqual(result['android_pixel_center']['y'], 504.6666667)

    def test_reject_wrong_target_and_wrong_geometry(self):
        for key, value in [('display_id', 1), ('bounds', {'x': 0, 'y': 0, 'width': 1920, 'height': 1200})]:
            with self.subTest(key=key):
                original = self.descriptor
                self.descriptor = {**original, key: value}
                with self.assertRaises(ValueError):
                    self.check()
                self.descriptor = original

    def test_reject_lost_or_duplicate_native_events(self):
        original = copy.deepcopy(self.after)
        for events in (original['events'][1:], original['events'] + [original['events'][-1]]):
            self.after['events'] = events
            with self.assertRaisesRegex(ValueError, 'incomplete or out of order'):
                self.check()

    def test_reject_counters_without_native_delivery(self):
        self.after['windows'][0]['counters']['down'] = 2
        with self.assertRaisesRegex(ValueError, 'counters disagree'):
            self.check()

    def test_reject_main_guard_action(self):
        self.after['windows'][1]['counters']['down'] = 1
        with self.assertRaisesRegex(ValueError, 'Main guard'):
            self.check()

    def test_reject_unreleased_button(self):
        self.after['events'] = self.after['events'][:-1]
        self.after['sequence'] = 50
        del self.after['windows'][0]['counters']['up']
        with self.assertRaisesRegex(ValueError, 'not released'):
            self.check()

    def test_reject_wrong_release_click_count(self):
        self.after['events'][-1]['click_count'] = 2
        with self.assertRaisesRegex(ValueError, 'click counts differ'):
            self.check()

    def test_actual_drag_requires_displacement_not_only_counter(self):
        with self.assertRaisesRegex(ValueError, 'No observed drag displacement'):
            self.check('hold-drag')
        for event in self.after['events'][-2:]:
            event['global_point']['x'] += 80
            event['local_point']['x'] += 80
        self.assertEqual(self.check('hold-drag', min_drag_distance=60)['status'], 'passed')
        with self.assertRaisesRegex(ValueError, 'No observed drag displacement'):
            self.check('hold-drag', min_drag_distance=100)

    def test_blocked_receipt_cannot_pass_delivered_tap(self):
        injection = {'status': 'passed', 'gesture': 'tap', 'gestureEvents': [{'accepted': True}]}
        with self.assertRaisesRegex(ValueError, 'Blocked input'):
            self.check('blocked', injection=injection)
        self.after = copy.deepcopy(self.before)
        with self.assertRaisesRegex(ValueError, 'actual Android tap'):
            self.check('blocked')
        self.assertEqual(self.check('blocked', injection=injection)['status'], 'passed')

    def test_scroll_requires_native_nonzero_delta(self):
        event = copy.deepcopy(self.after['events'][0])
        event.update(kind='scroll', sequence=47, scroll={'dx': 0, 'dy': 0})
        self.after['sequence'] = 47
        self.after['events'] = [event]
        self.after['windows'][0]['counters'] = {'scroll': 1}
        with self.assertRaisesRegex(ValueError, 'actual scroll delta'):
            self.check('two-finger-scroll')
        event['scroll']['dy'] = 13.5
        self.assertEqual(self.check('two-finger-scroll')['status'], 'passed')

    def test_right_tap_and_double_click_counts(self):
        for event in self.after['events']:
            if event['kind'] in ('down', 'up', 'drag'):
                event['button'] = 1
        self.assertEqual(self.check('two-finger-tap')['status'], 'passed')
        with self.assertRaisesRegex(ValueError, 'button or click'):
            self.check('tap')
        for event in self.after['events']:
            event['button'] = 0
        second = copy.deepcopy(self.after['events'][-3:])
        for index, event in enumerate(second, 52):
            event['sequence'] = index
            if event['kind'] in ('down', 'up'):
                event['click_count'] = 2
        self.after['events'].extend(second)
        self.after['sequence'] = 54
        self.after['windows'][0]['counters'] = {'move': 2, 'down': 2, 'up': 2, 'drag': 2}
        self.assertEqual(self.check('double-tap')['status'], 'passed')

    def test_pin_native_grant_revision_and_epoch(self):
        binding = {'session': 'fixture-only', 'epoch': 7, 'revision': 0}
        state = {'state': 'granted', 'revision': 0, 'binding': binding,
                 'geometry': {'canvas': self.descriptor['canvas'], 'raster': self.descriptor['raster'],
                              'source_size': {'width': 1920, 'height': 1200},
                              'content': {'x': 0, 'y': 0, 'width': 1, 'height': 1}}}
        self.assertEqual(self.check(input_state=state, binding=binding)['status'], 'passed')
        with self.assertRaisesRegex(ValueError, 'epoch or revision'):
            self.check(input_state=state, binding={**binding, 'epoch': 8})
        with self.assertRaisesRegex(ValueError, 'capture revision'):
            self.check(input_state={**state, 'revision': 1})
        with self.assertRaisesRegex(ValueError, 'canvas or raster'):
            self.check(input_state={**state, 'geometry': {**state['geometry'], 'raster': {'width': 1280, 'height': 800}}})

    def test_scoped_source_descriptor_binding(self):
        del self.descriptor['configuration_id']
        del self.descriptor['raster']
        binding = {'session': 'fixture-source', 'epoch': 3, 'revision': 0}
        self.descriptor.update(binding)
        self.assertEqual(self.check(binding=binding)['scoped_source_binding'], binding)
        with self.assertRaisesRegex(ValueError, 'expected binding'):
            self.check(binding={**binding, 'epoch': 4})
        self.descriptor['configuration_id'] = 1
        with self.assertRaisesRegex(ValueError, 'revisions disagree'):
            self.check()

    def test_home_needs_real_transition_receipt(self):
        for event in self.after['events'][-2:]:
            event['global_point']['x'] += 80
            event['local_point']['x'] += 80
        with self.assertRaisesRegex(ValueError, 'Home release requires'):
            self.check('home-drag')
        injection = {'status': 'passed', 'gesture': 'home-drag', 'home': 'true',
                     'foregroundAfter': 'android.launcher', 'gestureEvents': [{'accepted': True}]}
        self.assertEqual(self.check('home-drag', injection=injection, package='fixture.app')['status'], 'passed')
        with self.assertRaisesRegex(ValueError, 'HOME transition'):
            self.check('home-drag', injection={**injection, 'foregroundAfter': 'fixture.app'}, package='fixture.app')

    def test_injection_success_cannot_replace_native_tap(self):
        self.after = copy.deepcopy(self.before)
        injection = {'status': 'passed', 'gesture': 'tap', 'gestureEvents': '[{"accepted":true}]'}
        with self.assertRaisesRegex(ValueError, 'button or click'):
            self.check(injection=injection)


if __name__ == '__main__':
    unittest.main()
