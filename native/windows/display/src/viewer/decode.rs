//! H.264 decoding with Media Foundation's H.264 decoder. Given a DXGI device
//! manager it decodes on the GPU through DXVA; without one, in software.
//! Either way each picture is copied into a small ring of NV12 textures on
//! the viewer's device, cropped to the picture's display area, so the
//! decoder's own surfaces return to it at once.
use anyhow::{ensure, Context, Result};
use extend_computer_agent::display::Frame;
use windows::{
    core::Interface,
    Win32::{
        Foundation::HMODULE,
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN},
            Direct3D10::ID3D10Multithread,
            Direct3D11::*,
            Dxgi::{Common::*, *},
            Gdi::HMONITOR,
        },
        Media::MediaFoundation::*,
        System::{
            Com::{CoInitializeEx, CoTaskMemFree, COINIT_MULTITHREADED},
            Variant::VARIANT,
        },
    },
};

/// Pictures in flight between the decoder and the window: the one shown,
/// the next one and the one being written.
const RING: usize = 3;

/// The adapter that drives `monitor`, so decoding and presenting share it.
pub fn adapter_for(monitor: HMONITOR) -> Option<IDXGIAdapter1> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.ok()?;
    let mut index = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(index) } {
        let mut output = 0;
        while let Ok(candidate) = unsafe { adapter.EnumOutputs(output) } {
            if unsafe { candidate.GetDesc() }.is_ok_and(|desc| desc.Monitor == monitor) {
                return Some(adapter);
            }
            output += 1;
        }
        index += 1;
    }
    None
}

pub fn adapter_name(device: &ID3D11Device) -> String {
    (|| -> Result<String> {
        let adapter = unsafe { device.cast::<IDXGIDevice>()?.GetAdapter()? };
        let desc = unsafe { adapter.GetDesc()? };
        let length = desc.Description.iter().position(|&c| c == 0).unwrap_or(128);
        Ok(String::from_utf16_lossy(&desc.Description[..length]))
    })()
    .unwrap_or_else(|_| "unknown adapter".into())
}

/// A device for video decoding and presentation, safe to use from the
/// decoder's thread and the window's.
pub fn device(adapter: Option<&IDXGIAdapter1>) -> Result<ID3D11Device> {
    let adapter: Option<IDXGIAdapter> = adapter.map(|a| a.cast()).transpose()?;
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            adapter.as_ref(),
            if adapter.is_some() {
                D3D_DRIVER_TYPE_UNKNOWN
            } else {
                D3D_DRIVER_TYPE_HARDWARE
            },
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .context("Could not start the graphics device")?;
    }
    let context: ID3D11DeviceContext = context.context("Missing graphics context")?;
    unsafe {
        let _ = context
            .cast::<ID3D10Multithread>()?
            .SetMultithreadProtected(true);
    }
    device.context("Missing graphics device")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// DXVA on the device's GPU.
    Hardware,
    Software,
}

/// The YCbCr encoding of decoded pictures, from the stream's VUI. Untagged
/// streams are BT.709 limited range, as macOS sources encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub bt601: bool,
    pub full_range: bool,
}

/// A decoded picture on the viewer's device, `width` x `height` NV12.
#[derive(Clone)]
pub struct Picture {
    pub texture: ID3D11Texture2D,
    pub width: u32,
    pub height: u32,
    pub color: Color,
}

/// The negotiated output: the coded size, the visible part of it and the
/// row pitch of pictures in memory.
struct Layout {
    coded: (u32, u32),
    visible: (u32, u32, u32, u32),
    stride: u32,
    color: Color,
}

pub struct Decoder {
    transform: IMFTransform,
    _manager: Option<IMFDXGIDeviceManager>,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    pub mode: Mode,
    pub name: String,
    provides_samples: bool,
    output_size: u32,
    layout: Option<Layout>,
    ring: Vec<ID3D11Texture2D>,
    ring_size: (u32, u32),
    next: usize,
    staging: Option<(ID3D11Texture2D, (u32, u32))>,
    output: Option<(IMFSample, u32)>,
    waiting_keyframe: bool,
    /// Pictures that arrived as GPU surfaces rather than in memory.
    pub surfaces: u64,
}

fn pack(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

impl Decoder {
    /// Prefers DXVA unless `EXTEND_DISPLAY_DECODER=software`. `size` is the
    /// expected coded size; the stream's own parameter sets decide.
    pub fn new(device: &ID3D11Device, size: (u32, u32)) -> Result<Self> {
        let software_only = std::env::var("EXTEND_DISPLAY_DECODER").is_ok_and(|v| v == "software");
        if !software_only {
            match Self::with_mode(device, Mode::Hardware, size) {
                Ok(decoder) => return Ok(decoder),
                Err(error) => eprintln!("Display hardware decoding unavailable: {error:#}"),
            }
        }
        Self::with_mode(device, Mode::Software, size)
    }

    pub fn with_mode(device: &ID3D11Device, mode: Mode, size: (u32, u32)) -> Result<Self> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET)
                .context("Could not start Media Foundation")?;
        }
        let mut last = None;
        for (activate, name) in decoders()? {
            match Self::start(&activate, name.clone(), device, mode, size) {
                Ok(decoder) => return Ok(decoder),
                Err(error) => {
                    eprintln!("Display decoder {name} unavailable: {error:#}");
                    last = Some(error);
                }
            }
        }
        Err(last.unwrap_or_else(|| anyhow::anyhow!("No H.264 decoder is available")))
    }

    fn start(
        activate: &IMFActivate,
        name: String,
        device: &ID3D11Device,
        mode: Mode,
        size: (u32, u32),
    ) -> Result<Self> {
        let transform: IMFTransform = unsafe { activate.ActivateObject()? };
        let manager = unsafe {
            let attributes = transform.GetAttributes().ok();
            if let Some(attributes) = &attributes {
                // Show each picture as soon as it's decoded instead of
                // holding a reorder window; sources send no B-frames.
                let _ = attributes.SetUINT32(&MF_LOW_LATENCY, 1);
            }
            if let Ok(codec) = transform.cast::<ICodecAPI>() {
                let _ = codec.SetValue(&CODECAPI_AVLowLatencyMode, &VARIANT::from(true));
            }
            match mode {
                Mode::Software => None,
                Mode::Hardware => {
                    ensure!(
                        attributes
                            .as_ref()
                            .and_then(|a| a.GetUINT32(&MF_SA_D3D11_AWARE).ok())
                            .unwrap_or(0)
                            != 0,
                        "The decoder can't use Direct3D 11"
                    );
                    let mut token = 0;
                    let mut manager = None;
                    MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
                    let manager = manager.context("Missing DXGI device manager")?;
                    manager.ResetDevice(device, token)?;
                    transform
                        .ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize)
                        .context("The decoder refused the graphics device")?;
                    Some(manager)
                }
            }
        };
        unsafe {
            let input = MFCreateMediaType()?;
            input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
            input.SetUINT64(&MF_MT_FRAME_SIZE, pack(size.0, size.1))?;
            input.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            transform
                .SetInputType(0, &input, 0)
                .context("The decoder rejected the H.264 stream settings")?;
        }
        let mut decoder = Self {
            transform,
            _manager: manager,
            device: device.clone(),
            context: unsafe { device.GetImmediateContext()? },
            mode,
            name,
            provides_samples: false,
            output_size: 0,
            layout: None,
            ring: Vec::new(),
            ring_size: (0, 0),
            next: 0,
            staging: None,
            output: None,
            waiting_keyframe: true,
            surfaces: 0,
        };
        decoder.negotiate()?;
        unsafe {
            decoder
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            decoder
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
        }
        Ok(decoder)
    }

    /// Picks NV12 output, at startup and whenever the stream changes size.
    fn negotiate(&mut self) -> Result<()> {
        let mut index = 0;
        let output = loop {
            let candidate = unsafe { self.transform.GetOutputAvailableType(0, index) }
                .context("The decoder offers no NV12 output")?;
            if unsafe { candidate.GetGUID(&MF_MT_SUBTYPE)? } == MFVideoFormat_NV12 {
                break candidate;
            }
            index += 1;
        };
        unsafe { self.transform.SetOutputType(0, &output, 0)? };
        let frame = unsafe { output.GetUINT64(&MF_MT_FRAME_SIZE)? };
        let coded = ((frame >> 32) as u32, frame as u32);
        let visible = [MF_MT_MINIMUM_DISPLAY_APERTURE, MF_MT_GEOMETRIC_APERTURE]
            .iter()
            .find_map(|key| aperture(&output, key, coded))
            .unwrap_or((0, 0, coded.0, coded.1));
        let color = Color {
            bt601: unsafe { output.GetUINT32(&MF_MT_YUV_MATRIX) }
                .is_ok_and(|m| m == MFVideoTransferMatrix_BT601.0 as u32),
            full_range: unsafe { output.GetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE) }
                .is_ok_and(|r| r == MFNominalRange_0_255.0 as u32),
        };
        // NV12 copies need even rectangles.
        let visible = (
            visible.0 & !1,
            visible.1 & !1,
            visible.2 & !1,
            visible.3 & !1,
        );
        ensure!(
            visible.2 > 0 && visible.3 > 0,
            "Invalid decoded picture size"
        );
        let info = unsafe { self.transform.GetOutputStreamInfo(0)? };
        self.provides_samples = info.dwFlags
            & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0)
                as u32
            != 0;
        self.output_size = info.cbSize.max(coded.0 * coded.1 * 3 / 2);
        let stride = unsafe { output.GetUINT32(&MF_MT_DEFAULT_STRIDE) }
            .ok()
            .filter(|&stride| (stride as i32) >= coded.0 as i32)
            .unwrap_or(coded.0);
        self.layout = Some(Layout {
            coded,
            visible,
            stride,
            color,
        });
        Ok(())
    }

    /// Decodes one Annex B frame. Until a keyframe arrives, at startup or
    /// after an error, other frames are dropped. On an error the caller asks
    /// for a keyframe and calls [`Self::reset`].
    pub fn decode(&mut self, frame: &Frame) -> Result<Vec<Picture>> {
        if self.waiting_keyframe && !frame.keyframe {
            return Ok(Vec::new());
        }
        let sample = unsafe {
            let buffer = MFCreateMemoryBuffer(frame.bytes.len() as u32)?;
            let mut data = std::ptr::null_mut();
            buffer.Lock(&mut data, None, None)?;
            std::ptr::copy_nonoverlapping(frame.bytes.as_ptr(), data, frame.bytes.len());
            buffer.Unlock()?;
            buffer.SetCurrentLength(frame.bytes.len() as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            // Media Foundation time is in 100 ns units.
            sample.SetSampleTime((frame.timestamp / 100) as i64)?;
            if frame.keyframe {
                sample.SetUINT32(&MFSampleExtension_CleanPoint, 1)?;
            }
            sample
        };
        let mut pictures = Vec::new();
        match unsafe { self.transform.ProcessInput(0, &sample, 0) } {
            Ok(()) => {}
            Err(error) if error.code() == MF_E_NOTACCEPTING => {
                self.drain(&mut pictures)?;
                unsafe { self.transform.ProcessInput(0, &sample, 0) }
                    .context("H.264 decoding failed")?;
            }
            Err(error) => return Err(error).context("H.264 decoding failed"),
        }
        self.waiting_keyframe = false;
        self.drain(&mut pictures)?;
        Ok(pictures)
    }

    /// Whether frames are being dropped until the next keyframe.
    pub fn waiting(&self) -> bool {
        self.waiting_keyframe
    }

    /// Drops pictures in progress; decoding restarts at the next keyframe.
    pub fn reset(&mut self) {
        unsafe {
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
        }
        self.waiting_keyframe = true;
    }

    fn drain(&mut self, pictures: &mut Vec<Picture>) -> Result<()> {
        let mut changes = 0;
        loop {
            // A decoder that doesn't allocate its output fills ours, which is
            // copied out before the next call.
            let sample = if self.provides_samples {
                None
            } else {
                match &self.output {
                    Some((sample, size)) if *size == self.output_size => unsafe {
                        sample.GetBufferByIndex(0)?.SetCurrentLength(0)?;
                        Some(sample.clone())
                    },
                    _ => unsafe {
                        let sample = MFCreateSample()?;
                        sample.AddBuffer(&MFCreateMemoryBuffer(self.output_size)?)?;
                        self.output = Some((sample.clone(), self.output_size));
                        Some(sample)
                    },
                }
            };
            let mut buffers = [MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: std::mem::ManuallyDrop::new(sample),
                dwStatus: 0,
                pEvents: std::mem::ManuallyDrop::new(None),
            }];
            let mut status = 0;
            let result = unsafe { self.transform.ProcessOutput(0, &mut buffers, &mut status) };
            let [buffer] = buffers;
            drop(std::mem::ManuallyDrop::into_inner(buffer.pEvents));
            let sample = std::mem::ManuallyDrop::into_inner(buffer.pSample);
            match result {
                Ok(()) => {}
                Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
                Err(error) if error.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    // A new size renegotiates; a bound stops a decoder that
                    // keeps changing.
                    changes += 1;
                    ensure!(changes <= 4, "The H.264 decoder kept changing its output");
                    self.negotiate()?;
                    continue;
                }
                Err(error) => return Err(error).context("H.264 decoding failed"),
            }
            if let Some(sample) = sample {
                pictures.push(self.publish(&sample)?);
            }
        }
    }

    fn ring_texture(&mut self, width: u32, height: u32) -> Result<ID3D11Texture2D> {
        if self.ring_size != (width, height) || self.ring.is_empty() {
            self.ring.clear();
            for _ in 0..RING {
                self.ring.push(texture(
                    &self.device,
                    width,
                    height,
                    D3D11_USAGE_DEFAULT,
                    D3D11_BIND_RENDER_TARGET.0 as u32,
                    0,
                )?);
            }
            self.ring_size = (width, height);
            self.next = 0;
        }
        let texture = self.ring[self.next].clone();
        self.next = (self.next + 1) % RING;
        Ok(texture)
    }

    /// Copies a decoded sample's visible area into the next ring texture.
    fn publish(&mut self, sample: &IMFSample) -> Result<Picture> {
        let layout = self.layout.as_ref().context("The decoder has no output")?;
        let (x, y, width, height) = layout.visible;
        let (coded, stride, color) = (layout.coded, layout.stride, layout.color);
        let target = self.ring_texture(width, height)?;
        let buffer = unsafe { sample.GetBufferByIndex(0)? };
        if let Ok(surface) = buffer.cast::<IMFDXGIBuffer>() {
            self.surfaces += 1;
            let source = unsafe {
                let mut raw = std::ptr::null_mut();
                surface.GetResource(&ID3D11Texture2D::IID, &mut raw)?;
                ID3D11Texture2D::from_raw(raw)
            };
            let index = unsafe { surface.GetSubresourceIndex()? };
            let area = D3D11_BOX {
                left: x,
                top: y,
                front: 0,
                right: x + width,
                bottom: y + height,
                back: 1,
            };
            unsafe {
                self.context
                    .CopySubresourceRegion(&target, 0, 0, 0, 0, &source, index, Some(&area))
            };
        } else {
            self.upload(&buffer, coded, stride, (x, y, width, height), &target)?;
        }
        Ok(Picture {
            texture: target,
            width,
            height,
            color,
        })
    }

    /// Software output: NV12 in memory, luma rows then interleaved chroma.
    fn upload(
        &mut self,
        buffer: &IMFMediaBuffer,
        coded: (u32, u32),
        stride: u32,
        (x, y, width, height): (u32, u32, u32, u32),
        target: &ID3D11Texture2D,
    ) -> Result<()> {
        if self.staging.as_ref().map(|(_, size)| *size) != Some((width, height)) {
            let staging = texture(
                &self.device,
                width,
                height,
                D3D11_USAGE_STAGING,
                0,
                D3D11_CPU_ACCESS_WRITE.0 as u32,
            )?;
            self.staging = Some((staging, (width, height)));
        }
        let staging = self.staging.as_ref().unwrap().0.clone();
        let two_d = buffer.cast::<IMF2DBuffer>().ok();
        // A 2D buffer knows its pitch; a plain one uses the type's stride.
        let (source, pitch, length) = unsafe {
            let mut data = std::ptr::null_mut();
            match &two_d {
                Some(two_d) => {
                    let mut pitch = 0;
                    two_d.Lock2D(&mut data, &mut pitch)?;
                    (data as *const u8, pitch, None)
                }
                None => {
                    let mut length = 0;
                    buffer.Lock(&mut data, None, Some(&mut length))?;
                    (data as *const u8, stride as i32, Some(length as usize))
                }
            }
        };
        let result = (|| -> Result<()> {
            ensure!(pitch > 0, "Unsupported decoded picture layout");
            let pitch = pitch as usize;
            let rows = coded.1 as usize * 3 / 2;
            ensure!(
                length.is_none_or(|length| length >= pitch * rows),
                "Truncated decoded picture"
            );
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            unsafe {
                self.context
                    .Map(&staging, 0, D3D11_MAP_WRITE, 0, Some(&mut mapped))?
            };
            let target_pitch = mapped.RowPitch as usize;
            let destination = mapped.pData as *mut u8;
            let (x, y, width, height) = (x as usize, y as usize, width as usize, height as usize);
            let coded_height = coded.1 as usize;
            // The chroma plane follows the coded luma rows in the source and
            // the texture's rows in the staging copy.
            for row in 0..height + height / 2 {
                let from = if row < height {
                    (y + row) * pitch + x
                } else {
                    (coded_height + y / 2 + row - height) * pitch + x
                };
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        source.add(from),
                        destination.add(row * target_pitch),
                        width,
                    );
                }
            }
            unsafe {
                self.context.Unmap(&staging, 0);
                self.context.CopyResource(target, &staging);
            }
            Ok(())
        })();
        unsafe {
            match &two_d {
                Some(two_d) => two_d.Unlock2D()?,
                None => buffer.Unlock()?,
            }
        }
        result
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            let _ = self
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
            let _ = self
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
        }
    }
}

/// The visible rectangle a media type stores in `key`, inside the coded size.
fn aperture(
    output: &IMFMediaType,
    key: &windows::core::GUID,
    coded: (u32, u32),
) -> Option<(u32, u32, u32, u32)> {
    let mut bytes = [0u8; std::mem::size_of::<MFVideoArea>()];
    let mut size = 0;
    unsafe { output.GetBlob(key, &mut bytes, Some(&mut size)) }.ok()?;
    if size as usize != bytes.len() {
        return None;
    }
    let area: MFVideoArea = unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast()) };
    let (x, y) = (
        area.OffsetX.value.max(0) as u32,
        area.OffsetY.value.max(0) as u32,
    );
    let (width, height) = (area.Area.cx.max(0) as u32, area.Area.cy.max(0) as u32);
    (width > 0 && height > 0 && x + width <= coded.0 && y + height <= coded.1)
        .then_some((x, y, width, height))
}

pub fn texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    usage: D3D11_USAGE,
    bind: u32,
    cpu: u32,
) -> Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_NV12,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: usage,
        BindFlags: bind,
        CPUAccessFlags: cpu,
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    texture.context("Missing texture")
}

/// Registered H.264 to NV12 decoders, best first, with their names.
fn decoders() -> Result<Vec<(IMFActivate, String)>> {
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_H264,
    };
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_NV12,
    };
    let mut activates = std::ptr::null_mut();
    let mut count = 0;
    let mut found = Vec::new();
    unsafe {
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_DECODER,
            MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_LOCALMFT | MFT_ENUM_FLAG_SORTANDFILTER,
            Some(&input),
            Some(&output),
            &mut activates,
            &mut count,
        )?;
        if activates.is_null() {
            return Ok(found);
        }
        // Taking each entry moves ownership out; the array itself is freed below.
        for slot in std::slice::from_raw_parts_mut(activates, count as usize) {
            if let Some(activate) = slot.take() {
                let name = crate::encoder::friendly_name(&activate)
                    .unwrap_or_else(|| "H.264 decoder".into());
                found.push((activate, name));
            }
        }
        CoTaskMemFree(Some(activates as *const _));
    }
    Ok(found)
}
