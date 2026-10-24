# Synthetic content cadence diagnostics

`EXTEND_DISPLAY_BENCHMARK_MARKER=1` enables a diagnostic hook in the native display helper. Use it only with the generated campaign workload's timing counter. It is off by default. Enable the same diagnostics on source and receiver for matched diagnostic runs; do not compare their observer overhead with uninstrumented production runs.

The hook reads 3×3 luma samples at the known synthetic counter cells, validates both complementary rows and the four-bit checksum, and records only the 12-bit counter value. It stores no image and does not perform arbitrary screen hashing. It accepts integral backing scales and the negotiated logical dimensions, with NV12 video/full-range or BGRA output. Geometry mismatch, gray transition samples, non-complementary rows and failed checksums produce invalid events instead of inferred counter values.

Compressed decoder IOSurfaces use a cached Metal-backed Core Image context. Only two three-pixel-high strips are cropped and packed into a six-row atlas for one synchronous GPU readback: 14,472 bytes at 2× scale, with a 21,672-byte upper bound at 3×. No full-frame CPU readback or decoder-output-format change occurs. Source linear NV12 samples retain the direct CPU path. Temporary atlas pixels are never written to disk.

`capture_marker_read_us` and `decoded_marker_read_us` report diagnostic work duration in microseconds in the numeric `bytes` field. GPU context setup and synchronization can add observer overhead. Synthetic small-frame timing is not a bound on Retina live-session overhead; these instrumented runs diagnose content cadence and are not independent parity evidence.

Native trace events:

- `capture_marker`: `frame_ns` is the capture frame timestamp; `bytes` is the counter ID, including valid zero.
- `decoded_marker`: `frame_ns` is the receiver frame timestamp; `bytes` is the counter ID.
- `capture_marker_invalid` / `decoded_marker_invalid`: the sampled counter was not reliably readable.

`bytes` is a reused numeric event field here, **not a payload size**. Do not sum it as bandwidth. Counters wrap at 4096. Source and receiver frame timestamp domains can differ because of RTP timestamp mapping; do not join them across hosts without a clock mapping. Counter comparisons need chronological unwrapping and a shared workload interval.

Interpret the measurements in separate steps:

1. Several capture frame timestamps carrying the same counter ID indicate repeated synthetic content before transmission. Capture callback FPS alone cannot rule this out.
2. Decode frames carrying those counter IDs show which content reached the decoder. Compare valid counts and invalid samples independently.
3. Join `decoded_marker` to `presented` using the same receiver-local `frame_ns` to find content actually submitted to compositor presentation. Decode FPS alone cannot establish displayed content cadence.
4. Compare those IDs to camera-observed counters. Brief drawable presentations can be missed by rolling exposure or checksum rejection; compositor timing does not establish physical panel visibility.

These hooks diagnose one known synthetic region. They do not establish entire-screen pixel identity, cursor latency, perceptual quality or the cadence of every application.
