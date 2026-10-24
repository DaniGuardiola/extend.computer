# Digital image quality

This module compares digital captures of the same synthetic raster. It does not score camera recordings or claim to measure physical panel quality. Optical recording remains useful for visible frame cadence and latency.

## Automated capture

With the authenticated source host running and an extended-display connection active:

```sh
./benchmarks/display/run digital-quality --product extend --output out/quality-run
```

The source host must run the current shared tooling. The command generates one target, displays it in an owned native window over a black synthetic desktop, captures its source raster, and collects three receiver captures before scoring. It retains only the synthetic receiver target and a six-pixel border. It writes `report.json` and opens `report.html` on completion, including failures after capture setup.

Both ends must present identical backing pixels at integral placement. The runner rejects scaling, changed source geometry, restarted apps, unconfirmed direct LAN, missing ICC profiles and ambiguous corner registration. All four corners must agree on one integer translation within six pixels; scoring performs no resampling. Both screenshot profiles are converted to sRGB with a recorded relative-colorimetric transform. No brightness or contrast adjustment is applied.

For a settings comparison, apply each quality preset through `display-settings`, then run this command into a fresh output directory. Preserve resolution, scale, frame rate, route and fixture between captures. These static scores complement the motion/camera settings sweep; they do not measure motion artifacts or establish an overall preset ranking.

## Manual scoring

Generate a target with a known font and backing scale:

```sh
benchmarks/display/automation/.venv/bin/python benchmarks/display/quality/digital.py generate \
  --output out/quality --scale 2 \
  --font '/System/Library/Fonts/Supplemental/Arial.ttf'
```

The PNG contains text at 10, 12, 14, 18 and 24 logical points; colored edges at several physical-pixel widths; grayscale and color gradients; and corner fiducials. The manifest records physical scoring regions, logical dimensions, backing scale, font hash and target hash. Distribute the generated PNG itself to avoid font-rendering differences between hosts.

Display the target at exactly 1280×720 logical points for scale 2, producing 2560×1440 backing pixels. Avoid browser zoom, image fitting, interpolation, fractional placement or a scaled viewer. Capture the source and receiver digitally after the static target has settled. Include all four fiducials. Keep screenshot format lossless and color-management settings matched. A source screenshot can serve as reference to exclude source rasterization differences; the generated PNG can instead serve as reference when its rendering is pixel-identical.

Verify alignment from the fiducials, backing scale and pixel dimensions before scoring. Crop both images to the same target extent. Crop coordinates are **physical pixels** and use x,y,width,height. The standalone comparator below performs no automatic registration, resampling or brightness/color normalization. Its registration note is an operator assertion, unlike the automated runner's checked four-corner evidence. One-pixel misalignment will lower scores even when transmission is lossless.

```sh
benchmarks/display/automation/.venv/bin/python benchmarks/display/quality/digital.py compare \
  --reference out/source.png --received out/receiver.png \
  --manifest out/quality/target.json \
  --reference-crop 0,0,2560,1440 --received-crop 0,0,2560,1440 \
  --registration-note 'Four fiducials aligned; 2x backing scale; no resampling; matched RGB captures' \
  --output out/quality-report.json
```

Scores are per region, not a single opaque quality number:

- RGB PSNR and RMSE measure overall pixel error. Exact matches use `rgb_psnr_db: null` plus `psnr_infinite: true`, so JSON stays valid.
- `luma_ssim_box11` is local SSIM averaged over valid 11×11 uniform windows, with conventional C1/C2 constants. It is explicitly distinguished from Gaussian-window or global SSIM implementations.
- `edge_gradient_rmse` measures errors in horizontal and vertical luma differences, exposing blurred or damaged fine detail.
- `chroma_rmse_bt709` measures Cb/Cr color-detail error independently of luma.

Lower error and higher SSIM/PSNR are better **only for matched captures**. These scores do not establish transport reliability, responsiveness, HDR fidelity, latency or perceptual parity by themselves. Test color management separately before interpreting color errors as codec errors. Reports retain hashes of both captures and the manifest, scoring definitions and registration evidence.

Run focused verification:

```sh
benchmarks/display/automation/.venv/bin/python -m unittest discover \
  -s benchmarks/display/quality -p 'test_*.py'
```

Tests demonstrate response to increasingly blurred text, simulated chroma subsampling, exact identity, different screenshot origins and rejected geometry/registration ambiguity. Target generation requires Pillow and scoring requires NumPy; both are already in the camera environment.
