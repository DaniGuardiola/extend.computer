//! `self-test`: encodes synthetic frames with the display encoder and decodes
//! them with the viewer's decoder, through the wire framing, on the GPU and
//! in software, checking each picture's size and content. Nothing is
//! captured or shown, so it runs without a desktop.
//!
//! `self-test stream W H FRAMES` writes a synthetic H.264 stream as framed
//! kind 1 packets instead, for replaying into a viewer
//! (scripts/windows/viewer-test.py).
use crate::{
    encoder::{Encoder, Input},
    viewer::decode::{self, Decoder, Mode, Picture},
};
use anyhow::{bail, ensure, Context, Result};
use extend_computer_agent::{
    display::Frame,
    helper::packet::{frame, write_packet, Kind},
};
use std::io::Write;
use windows::Win32::Graphics::Direct3D11::*;

/// The test picture's luma: four columns in two rows, all different, so a
/// mirrored, flipped or shifted picture fails.
const BLOCKS: [[u8; 4]; 2] = [[32, 96, 160, 224], [200, 64, 128, 16]];
/// Allowed difference of a block's mean luma or the mean chroma.
const TOLERANCE: f64 = 10.0;
const FRAMES: u64 = 10;
const FRAME_NS: u64 = 16_666_667;

pub fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        None => round_trip(),
        Some("stream") => stream(args),
        Some(other) => bail!("Unknown self-test: {other}"),
    }
}

fn round_trip() -> Result<()> {
    let sample = Frame {
        timestamp: 7,
        keyframe: true,
        bytes: vec![0, 0, 0, 1, 0x65, 1],
    };
    let framed = frame::decode(&frame::encode(&sample))?;
    ensure!(
        framed.timestamp == 7 && framed.keyframe && framed.bytes == sample.bytes,
        "Frame framing failed"
    );
    let device = decode::device(None)?;
    println!("Adapter: {}", decode::adapter_name(&device));
    let mut failures = 0;
    let mut streams = Vec::new();
    for (width, height) in [(640, 480), (1920, 1080)] {
        let mut encoders = Vec::new();
        for software in [false, true] {
            let (name, frames) = encode(&device, width, height, software)?;
            // Without a hardware encoder both rounds use the same one.
            if encoders.contains(&name) {
                continue;
            }
            encoders.push(name.clone());
            for mode in [Mode::Hardware, Mode::Software] {
                match check(&device, mode, &frames, width, height) {
                    Ok((decoder, pictures, means)) => println!(
                        "ok {width}x{height}: {name} -> {decoder} ({mode:?}), {} frames, {pictures} pictures, luma {means:?}",
                        frames.len()
                    ),
                    Err(error) => {
                        failures += 1;
                        println!("FAIL {width}x{height}: {name} -> {mode:?} decoding: {error:#}");
                    }
                }
            }
            if !software {
                streams.push((frames, width, height));
            }
        }
    }
    // New settings change the stream's size at a keyframe, mid-stream.
    for mode in [Mode::Hardware, Mode::Software] {
        let resized = Decoder::with_mode(&device, mode, (640, 480)).and_then(|mut decoder| {
            for (frames, width, height) in &streams {
                verify(&device, &mut decoder, frames, *width, *height)?;
            }
            Ok(())
        });
        match resized {
            Ok(()) => println!("ok 640x480 then 1920x1080 in one stream ({mode:?})"),
            Err(error) => {
                failures += 1;
                println!("FAIL size change ({mode:?}): {error:#}");
            }
        }
    }
    // Protocol sizes go up to 4096 on each side, which a portrait 4K
    // primary monitor reaches; Microsoft documents its decoder up to
    // 4096x2304. Reported only, for the runtime's size limits.
    let (width, height) = (2160, 3840);
    match encode(&device, width, height, false) {
        Ok((name, frames)) => {
            for mode in [Mode::Hardware, Mode::Software] {
                match check(&device, mode, &frames, width, height) {
                    Ok((decoder, ..)) => {
                        println!("note {width}x{height}: {name} -> {decoder} ({mode:?}) ok")
                    }
                    Err(error) => println!("note {width}x{height}: {mode:?} decoding: {error:#}"),
                }
            }
        }
        Err(error) => println!("note {width}x{height}: encoding: {error:#}"),
    }
    ensure!(failures == 0, "{failures} round trips failed");
    println!("PASS: H.264 encode, framing, hardware and software decode, picture size and content; no screen content captured.");
    Ok(())
}

/// NV12 test pattern, with an optional white bar at `bar` for motion.
fn pattern(width: u32, height: u32, bar: Option<u32>) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut image = vec![128u8; w * h * 3 / 2];
    for y in 0..h {
        for x in 0..w {
            image[y * w + x] = BLOCKS[y * 2 / h][x * 4 / w];
        }
    }
    if let Some(bar) = bar {
        let bar = bar as usize % w.saturating_sub(16).max(1);
        for y in 0..h {
            image[y * w + bar..y * w + bar + 16].fill(235);
        }
    }
    image
}

/// An NV12 texture holding `image`, for encoders that read the GPU.
fn upload(device: &ID3D11Device, image: &[u8], width: u32, height: u32) -> Result<ID3D11Texture2D> {
    let texture = decode::texture(
        device,
        width,
        height,
        D3D11_USAGE_DEFAULT,
        D3D11_BIND_RENDER_TARGET.0 as u32,
        0,
    )?;
    let staging = decode::texture(
        device,
        width,
        height,
        D3D11_USAGE_STAGING,
        0,
        D3D11_CPU_ACCESS_WRITE.0 as u32,
    )?;
    let context = unsafe { device.GetImmediateContext()? };
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe { context.Map(&staging, 0, D3D11_MAP_WRITE, 0, Some(&mut mapped))? };
    let w = width as usize;
    for row in 0..height as usize * 3 / 2 {
        unsafe {
            std::ptr::copy_nonoverlapping(
                image[row * w..].as_ptr(),
                (mapped.pData as *mut u8).add(row * mapped.RowPitch as usize),
                w,
            );
        }
    }
    unsafe {
        context.Unmap(&staging, 0);
        context.CopyResource(&texture, &staging);
    }
    Ok(texture)
}

fn encoder(device: &ID3D11Device, width: u32, height: u32, software: bool) -> Result<Encoder> {
    if software {
        std::env::set_var("EXTEND_DISPLAY_ENCODER", "software");
    } else {
        std::env::remove_var("EXTEND_DISPLAY_ENCODER");
    }
    let encoder = Encoder::new(device, width, height, 60, 8_000_000);
    std::env::remove_var("EXTEND_DISPLAY_ENCODER");
    encoder
}

/// Encodes the test pattern, and returns the frames as a viewer reads them.
fn encode(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    software: bool,
) -> Result<(String, Vec<Frame>)> {
    let mut encoder = encoder(device, width, height, software)?;
    let image = pattern(width, height, None);
    let texture = if encoder.wants_texture() {
        Some(upload(device, &image, width, height)?)
    } else {
        None
    };
    let mut frames = Vec::new();
    for index in 0..FRAMES {
        let input = match &texture {
            Some(texture) => Input::Texture(texture),
            None => Input::Memory(&image),
        };
        for encoded in encoder.encode(input, index * FRAME_NS, index == 0)? {
            frames.push(frame::decode(&frame::encode(&encoded))?);
        }
    }
    ensure!(
        frames.first().is_some_and(|f| f.keyframe),
        "{} did not start with a keyframe",
        encoder.name
    );
    ensure!(
        frames.len() as u64 >= FRAMES - 2,
        "{} returned {} of {FRAMES} frames",
        encoder.name,
        frames.len()
    );
    Ok((encoder.name.clone(), frames))
}

/// Decodes `frames` with a new decoder and checks the last picture.
fn check(
    device: &ID3D11Device,
    mode: Mode,
    frames: &[Frame],
    width: u32,
    height: u32,
) -> Result<(String, usize, [[u8; 4]; 2])> {
    let mut decoder = Decoder::with_mode(device, mode, (width, height))?;
    // A viewer that joins mid-stream waits for a keyframe.
    if let Some(later) = frames.iter().find(|f| !f.keyframe) {
        ensure!(
            decoder.decode(later)?.is_empty(),
            "Decoded a frame before the first keyframe"
        );
    }
    let (pictures, luma) = verify(device, &mut decoder, frames, width, height)?;
    // After an error the viewer resets the decoder and waits for a keyframe.
    decoder.reset();
    if let Some(later) = frames.iter().find(|f| !f.keyframe) {
        ensure!(
            decoder.decode(later)?.is_empty(),
            "Decoded a frame after a reset before a keyframe"
        );
    }
    verify(device, &mut decoder, frames, width, height).context("Decoding after a reset")?;
    ensure!(
        (mode == Mode::Hardware) == (decoder.surfaces > 0),
        "{mode:?} decoding returned {} of {pictures} pictures on the GPU",
        decoder.surfaces
    );
    Ok((decoder.name.clone(), pictures, luma))
}

/// Decodes `frames` and checks the last picture's size and content.
fn verify(
    device: &ID3D11Device,
    decoder: &mut Decoder,
    frames: &[Frame],
    width: u32,
    height: u32,
) -> Result<(usize, [[u8; 4]; 2])> {
    let mut pictures = 0;
    let mut last = None;
    for frame in frames {
        let decoded = decoder.decode(frame)?;
        pictures += decoded.len();
        last = decoded.into_iter().last().or(last);
    }
    let picture = last.context("Nothing was decoded")?;
    ensure!(
        (picture.width, picture.height) == (width, height),
        "Decoded {}x{}, expected {width}x{height}",
        picture.width,
        picture.height
    );
    let (luma, chroma) = means(device, &picture)?;
    let mut rounded = [[0u8; 4]; 2];
    for (row, values) in luma.iter().enumerate() {
        for (column, &value) in values.iter().enumerate() {
            rounded[row][column] = value.round() as u8;
            ensure!(
                (value - f64::from(BLOCKS[row][column])).abs() <= TOLERANCE,
                "Block {row},{column} has luma {value:.1}, expected {}",
                BLOCKS[row][column]
            );
        }
    }
    ensure!(
        chroma.iter().all(|c| (c - 128.0).abs() <= TOLERANCE),
        "Chroma {chroma:?}, expected neutral"
    );
    Ok((pictures, rounded))
}

/// Mean luma of each test block's middle and mean chroma of the picture.
fn means(device: &ID3D11Device, picture: &Picture) -> Result<([[f64; 4]; 2], [f64; 2])> {
    let (width, height) = (picture.width as usize, picture.height as usize);
    let staging = decode::texture(
        device,
        picture.width,
        picture.height,
        D3D11_USAGE_STAGING,
        0,
        D3D11_CPU_ACCESS_READ.0 as u32,
    )?;
    let context = unsafe { device.GetImmediateContext()? };
    unsafe { context.CopyResource(&staging, &picture.texture) };
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))? };
    let pitch = mapped.RowPitch as usize;
    let data =
        unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, pitch * height * 3 / 2) };
    let mut luma = [[0.0; 4]; 2];
    for (row, values) in luma.iter_mut().enumerate() {
        for (column, value) in values.iter_mut().enumerate() {
            let (x0, x1) = (
                column * width / 4 + width / 16,
                (column + 1) * width / 4 - width / 16,
            );
            let (y0, y1) = (
                row * height / 2 + height / 8,
                (row + 1) * height / 2 - height / 8,
            );
            let mut sum = 0u64;
            for y in y0..y1 {
                sum += data[y * pitch + x0..y * pitch + x1]
                    .iter()
                    .map(|&v| u64::from(v))
                    .sum::<u64>();
            }
            *value = sum as f64 / ((x1 - x0) * (y1 - y0)) as f64;
        }
    }
    let mut chroma = [0u64; 2];
    for y in 0..height / 2 {
        let row = &data[(height + y) * pitch..(height + y) * pitch + width];
        for pair in row.as_chunks::<2>().0 {
            chroma[0] += u64::from(pair[0]);
            chroma[1] += u64::from(pair[1]);
        }
    }
    unsafe { context.Unmap(&staging, 0) };
    let samples = (width / 2 * height / 2) as f64;
    Ok((luma, chroma.map(|sum| sum as f64 / samples)))
}

/// Writes `frames` frames of a moving test pattern as framed kind 1 packets.
fn stream(args: &[String]) -> Result<()> {
    let number = |index: usize| -> Result<u32> {
        args.get(index)
            .and_then(|value| value.parse().ok())
            .context("Usage: self-test stream <width> <height> <frames>")
    };
    let (width, height, count) = (number(1)? & !1, number(2)? & !1, number(3)?);
    let device = decode::device(None)?;
    let mut encoder = encoder(&device, width, height, false)?;
    eprintln!("Display encoder: {}", encoder.name);
    let mut output = std::io::stdout().lock();
    for index in 0..u64::from(count) {
        let image = pattern(width, height, Some(index as u32 * 8));
        let texture = if encoder.wants_texture() {
            Some(upload(&device, &image, width, height)?)
        } else {
            None
        };
        let input = match &texture {
            Some(texture) => Input::Texture(texture),
            None => Input::Memory(&image),
        };
        // A keyframe every two seconds lets a replay start anywhere.
        for encoded in encoder.encode(input, index * FRAME_NS, index % 120 == 0)? {
            write_packet(&mut output, Kind::Frame, &frame::encode(&encoded))?;
        }
    }
    output.flush()?;
    Ok(())
}
