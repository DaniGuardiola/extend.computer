#!/usr/bin/env python3
"""Check a generated native cursor in a tablet screenshot; not optical latency."""
import argparse
import base64
import io
import json
from PIL import Image, ImageChops


def generated_png():
    image = Image.new('RGBA', (32, 24), (255, 0, 255, 255))
    image.paste((0, 255, 255, 255), (16, 0, 32, 24))
    output = io.BytesIO()
    image.save(output, format='PNG')
    return base64.b64encode(output.getvalue()).decode('ascii')


def verify(path, source):
    image = Image.open(path).convert('RGB')
    red, green, blue = image.split()
    high = lambda channel: channel.point(lambda value: 255 if value > 220 else 0)
    low = lambda channel: channel.point(lambda value: 255 if value < 50 else 0)
    pink = ImageChops.multiply(ImageChops.multiply(high(red), high(blue)), low(green))
    cyan = ImageChops.multiply(ImageChops.multiply(high(green), high(blue)), low(red))
    boxes = [pink.getbbox(), cyan.getbbox()]
    marker = source['cursor']
    if not marker['visible']:
        assert boxes == [None, None], 'Hidden cursor still visible: ' + str(boxes)
        return {'status': 'passed', 'visible': False, 'color_bounds': boxes, 'size': image.size}
    assert all(boxes), 'Both generated cursor colors required: ' + str(boxes)
    config = source['config']
    width, height = config['width'], config['height']
    scale = min(image.width / width, image.height / height)
    left = (image.width-width*scale)/2 + (marker['x']*width-marker['hotspot_x'])*scale
    top = (image.height-height*scale)/2 + (marker['y']*height-marker['hotspot_y'])*scale
    right, bottom = left+marker['width']*scale, top+marker['height']*scale
    expected = [(left, top, (left+right)/2, bottom), ((left+right)/2, top, right, bottom)]
    tolerance = max(3, scale)
    for color, actual, bounds in zip((pink, cyan), boxes, expected):
        assert all(abs(value-target) <= tolerance for value, target in zip(actual, bounds)), \
            'Cursor geometry/hotspot mismatch: ' + str({'actual': actual, 'expected': bounds})
        expected_area = (bounds[2]-bounds[0])*(bounds[3]-bounds[1])
        assert color.histogram()[255] >= expected_area*.75, 'Cursor color area incomplete'
    for x, expected_rgb in ((left+(right-left)*.25, (255, 0, 255)), (left+(right-left)*.75, (0, 255, 255))):
        for fraction in (.25, .5, .75):
            pixel = image.getpixel((round(x), round(top+(bottom-top)*fraction)))
            assert all(abs(actual-target) < 30 for actual, target in zip(pixel, expected_rgb)), 'Cursor pixels differ: '+str(pixel)
    return {'status': 'passed', 'visible': True, 'color_bounds': boxes, 'expected_bounds': expected,
            'size': image.size, 'source_logical_size': [width, height], 'revision': marker['revision']}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('image', nargs='?')
    parser.add_argument('--source', help='Cursor stage source receipt')
    parser.add_argument('--generate', action='store_true')
    args = parser.parse_args()
    if args.generate:
        print(json.dumps({'png': generated_png()}))
    else:
        assert args.image and args.source, 'Image and source receipt required'
        with open(args.source) as source:
            print(json.dumps(verify(args.image, json.load(source))))
