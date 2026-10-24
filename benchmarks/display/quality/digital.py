#!/usr/bin/env python3
"""Matched digital-raster quality evidence; never camera photometry."""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def generate(directory, font_path, scale=2):
    if scale not in (1, 2, 3):
        raise ValueError('Scale must be an integer 1, 2 or 3; fractional resampling is not supported')
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    width, height = 1280 * scale, 720 * scale
    image = Image.new('RGB', (width, height), '#eeeeee')
    draw = ImageDraw.Draw(image)
    regions = []

    def region(name, box):
        regions.append({'name': name, 'bounds_px': [int(n * scale) for n in box]})

    for index, size in enumerate((10, 12, 14, 18, 24)):
        top = 25 + index * 43
        font = ImageFont.truetype(str(font_path), size * scale)
        draw.text((25 * scale, top * scale), 'Aa Bb 0123456789 | thin text, scrolling, punctuation: {}, [] /',
                  font=font, fill='#111111')
        region(f'text_{size}pt', (20, top - 3, 1220, top + 37))

    # Physical-pixel stripes expose chroma subsampling even at integral Retina scale.
    array = np.asarray(image).copy()
    for index, period in enumerate((1, 2, 3, 5)):
        left, top, right, bottom = 25 * scale, (270 + index * 25) * scale, 1220 * scale, (290 + index * 25) * scale
        colors = np.array([[255, 0, 0], [0, 130, 255]], dtype=np.uint8)
        stripes = colors[(np.arange(right - left) // period) % 2]
        array[top:bottom, left:right] = stripes[None, :, :]
        regions.append({'name': f'colored_edges_{period}px', 'bounds_px': [left, top, right, bottom]})
    ramp = np.linspace(0, 255, 1195 * scale).round().astype(np.uint8)
    array[420 * scale:475 * scale, 25 * scale:1220 * scale] = ramp[None, :, None]
    region('grayscale_gradient', (25, 420, 1220, 475))
    color = np.stack((ramp, ramp[::-1], np.full_like(ramp, 128)), axis=-1)
    array[515 * scale:570 * scale, 25 * scale:1220 * scale] = color[None, :, :]
    region('color_gradient', (25, 515, 1220, 570))
    image = Image.fromarray(array)
    draw = ImageDraw.Draw(image)
    # Registration fiducials are outside scored areas; capture all four corners.
    for x, y in ((4, 4), (1248, 4), (4, 688), (1248, 688)):
        for row in range(4):
            for column in range(4):
                box = ((x + column * 7) * scale, (y + row * 7) * scale,
                       (x + (column + 1) * 7) * scale - 1, (y + (row + 1) * 7) * scale - 1)
                draw.rectangle(box, fill='black' if (row + column) % 2 else 'white')
    target = directory / 'target.png'
    image.save(target)
    manifest = {'schema_version': 1, 'logical_size': [1280, 720], 'backing_scale': scale,
                'pixel_size': [width, height], 'font_sha256': digest(font_path),
                'font_name': Path(font_path).name, 'target_sha256': digest(target),
                'regions': regions, 'comparison_kind': 'digital_raster'}
    (directory / 'target.json').write_text(json.dumps(manifest, indent=2) + '\n')
    return manifest


def box_mean(values, window=11):
    padded = np.pad(values, ((1, 0), (1, 0)), mode='constant').cumsum(0).cumsum(1)
    return (padded[window:, window:] - padded[:-window, window:] - padded[window:, :-window]
            + padded[:-window, :-window]) / (window * window)


def luma(rgb):
    return rgb @ np.array([.2126, .7152, .0722])


def chroma(rgb):
    y = luma(rgb)
    return np.stack(((rgb[..., 2] - y) / 1.8556, (rgb[..., 0] - y) / 1.5748), axis=-1)


def metrics(reference, received):
    a, b = np.asarray(reference, dtype=float) / 255, np.asarray(received, dtype=float) / 255
    y_a, y_b = luma(a), luma(b)
    mean_a, mean_b = box_mean(y_a), box_mean(y_b)
    variance_a = np.maximum(0, box_mean(y_a * y_a) - mean_a * mean_a)
    variance_b = np.maximum(0, box_mean(y_b * y_b) - mean_b * mean_b)
    covariance = box_mean(y_a * y_b) - mean_a * mean_b
    ssim = ((2 * mean_a * mean_b + .01**2) * (2 * covariance + .03**2)
            / ((mean_a**2 + mean_b**2 + .01**2) * (variance_a + variance_b + .03**2)))
    mse = float(np.mean((a - b)**2))
    gradients = [np.diff(y_a, axis=axis) - np.diff(y_b, axis=axis) for axis in (0, 1)]
    edge_mse = sum(float(np.sum(g * g)) for g in gradients) / sum(g.size for g in gradients)
    return {'rgb_psnr_db': None if mse == 0 else float(-10 * np.log10(mse)),
            'psnr_infinite': mse == 0, 'rgb_rmse': float(np.sqrt(mse)),
            'luma_ssim_box11': 1.0 if mse == 0 else float(np.mean(np.clip(ssim, -1, 1))),
            'edge_gradient_rmse': float(np.sqrt(edge_mse)),
            'chroma_rmse_bt709': float(np.sqrt(np.mean((chroma(a) - chroma(b))**2)))}


def crop(image, box):
    x, y, width, height = box
    if min(x, y) < 0 or min(width, height) <= 0 or x + width > image.width or y + height > image.height:
        raise ValueError('Crop falls outside image or has invalid dimensions')
    return image.crop((x, y, x + width, y + height)).convert('RGB')


def compare(reference, received, manifest, reference_crop, received_crop, registration_note):
    if not registration_note.strip():
        raise ValueError('Explicit registration and scale verification note is required')
    a, b = crop(reference, reference_crop), crop(received, received_crop)
    if a.size != b.size or list(a.size) != manifest['pixel_size']:
        raise ValueError('Crops must match target backing dimensions exactly; no automatic scaling or registration')
    results = {}
    for region in manifest['regions']:
        bounds = region['bounds_px']
        if bounds[2] - bounds[0] < 11 or bounds[3] - bounds[1] < 11:
            raise ValueError('SSIM region must be at least 11x11 pixels')
        if min(bounds) < 0 or bounds[2] > a.width or bounds[3] > a.height:
            raise ValueError('Region outside target')
        results[region['name']] = metrics(a.crop(bounds), b.crop(bounds))
    return {'schema_version': 1, 'comparison_kind': 'matched_digital_raster', 'regions': results,
            'registration': {'note': registration_note, 'reference_crop_xywh': reference_crop,
                             'received_crop_xywh': received_crop, 'resampling': 'none',
                             'backing_scale': manifest['backing_scale']},
            'definitions': {'luma_ssim_box11': 'Mean local SSIM over valid 11x11 uniform windows; C1=.01^2, C2=.03^2',
                            'rgb_psnr_db': 'RGB MSE, normalized peak=1; null plus psnr_infinite=true means exact match',
                            'edge_gradient_rmse': 'RMSE of horizontal and vertical BT.709-luma first differences',
                            'chroma_rmse_bt709': 'RMSE of normalized BT.709 Cb/Cr channels'},
            'limitations': ['Digital capture comparison only, not camera photometry or physical panel quality',
                            'Caller must verify exact content, backing scale, crop alignment and color pipeline',
                            'No automatic geometric, brightness, contrast or color normalization']}


def parse_crop(value):
    numbers = [int(n) for n in value.split(',')]
    if len(numbers) != 4:
        raise argparse.ArgumentTypeError('Expected x,y,width,height in physical pixels')
    return numbers


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_subparsers(dest='action', required=True)
    gen = actions.add_parser('generate')
    gen.add_argument('--output', type=Path, required=True)
    gen.add_argument('--font', type=Path, required=True)
    gen.add_argument('--scale', type=int, default=2)
    score = actions.add_parser('compare')
    for name in ('reference', 'received', 'manifest', 'output'):
        score.add_argument('--' + name, type=Path, required=True)
    for name in ('reference-crop', 'received-crop'):
        score.add_argument('--' + name, type=parse_crop, required=True)
    score.add_argument('--registration-note', required=True)
    args = parser.parse_args()
    if args.action == 'generate':
        generate(args.output, args.font, args.scale)
    else:
        with Image.open(args.reference) as reference, Image.open(args.received) as received:
            report = compare(reference, received, json.loads(args.manifest.read_text()),
                             args.reference_crop, args.received_crop, args.registration_note)
        report['input_sha256'] = {name: digest(getattr(args, name)) for name in ('reference', 'received', 'manifest')}
        args.output.write_text(json.dumps(report, indent=2, allow_nan=False) + '\n')


if __name__ == '__main__':
    main()
