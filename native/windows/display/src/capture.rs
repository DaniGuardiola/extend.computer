//! Primary-display capture with DXGI Desktop Duplication. The pointer is drawn
//! into a copy of each desktop image, and the D3D11 video processor scales the
//! result to fit the stream size, with black bars when the shapes differ, and
//! converts it to NV12 on the GPU ([`Converter`], which the extended display
//! shares).
use anyhow::{bail, Context, Result};
use std::time::Duration;
use windows::{
    core::Interface,
    Win32::{
        Foundation::{HMODULE, POINT, RECT},
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
            Direct3D10::ID3D10Multithread,
            Direct3D11::*,
            Dxgi::{Common::*, *},
            Gdi::{DeleteObject, MonitorFromPoint, MONITOR_DEFAULTTOPRIMARY},
        },
        System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
        UI::{
            HiDpi::{
                GetDpiForMonitor, SetProcessDpiAwarenessContext,
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI,
            },
            WindowsAndMessaging::{
                DrawIconEx, GetCursorInfo, GetIconInfo, CURSORINFO, CURSOR_SHOWING, DI_NORMAL,
                ICONINFO,
            },
        },
    },
};

/// Encoders may still read a converted frame while the next one is made.
const RING: usize = 4;

pub fn dpi_aware() {
    // Without per-monitor awareness DXGI reports scaled desktop coordinates.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

pub fn monotonic_ns() -> u64 {
    let (mut counter, mut frequency) = (0i64, 0i64);
    unsafe {
        let _ = QueryPerformanceCounter(&mut counter);
        let _ = QueryPerformanceFrequency(&mut frequency);
    }
    (counter as u128 * 1_000_000_000 / frequency.max(1) as u128) as u64
}

/// The adapter and output that show the primary desktop (origin 0,0).
fn primary_output() -> Result<(IDXGIAdapter1, IDXGIOutput1, RECT)> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };
    let mut adapter_index = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(adapter_index) } {
        let mut output_index = 0;
        while let Ok(output) = unsafe { adapter.EnumOutputs(output_index) } {
            let desc = unsafe { output.GetDesc()? };
            let r = desc.DesktopCoordinates;
            if desc.AttachedToDesktop.as_bool() && r.left == 0 && r.top == 0 {
                return Ok((adapter, output.cast()?, r));
            }
            output_index += 1;
        }
        adapter_index += 1;
    }
    bail!("No display available.")
}

/// The primary monitor's size in physical pixels and its DPI (96 is 100%),
/// 0 when Windows can't say.
pub fn primary_screen() -> Result<(u32, u32, u32)> {
    let (_, _, r) = primary_output()?;
    let monitor = unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    let (mut dpi, mut dpi_y) = (0, 0);
    if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi, &mut dpi_y) }.is_err() {
        dpi = 0;
    }
    Ok(((r.right - r.left) as u32, (r.bottom - r.top) as u32, dpi))
}

struct Converted {
    texture: ID3D11Texture2D,
    view: ID3D11VideoProcessorOutputView,
}

/// A D3D11 device on `adapter` that the video processor and hardware
/// encoders can use.
pub fn video_device(adapter: &IDXGIAdapter) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
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
    let device = device.context("Missing graphics device")?;
    let context = context.context("Missing graphics context")?;
    // Hardware encoders use the device from their own threads.
    unsafe {
        let _ = context
            .cast::<ID3D10Multithread>()?
            .SetMultithreadProtected(true);
    }
    Ok((device, context))
}

/// Scales a desktop image to the stream size and converts it to NV12 with
/// the D3D11 video processor, into a small ring of textures an encoder can
/// read, or back to memory for the software encoder.
pub struct Converter {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    video: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    processor: Option<(ID3D11VideoProcessorEnumerator, ID3D11VideoProcessor)>,
    source_size: (u32, u32),
    width: u32,
    height: u32,
    ring: Vec<Converted>,
    next: usize,
    staging: ID3D11Texture2D,
    buffer: Vec<u8>,
}

impl Converter {
    /// Converts to `width` x `height` on `device`, once [`Self::set_source`]
    /// gives the source size.
    pub fn new(device: &ID3D11Device, width: u32, height: u32) -> Result<Self> {
        let context = unsafe { device.GetImmediateContext()? };
        let staging = texture(
            device,
            width,
            height,
            DXGI_FORMAT_NV12,
            D3D11_USAGE_STAGING,
            0,
            D3D11_CPU_ACCESS_READ.0 as u32,
            0,
        )?;
        Ok(Self {
            device: device.clone(),
            video: device.cast()?,
            video_context: context.cast()?,
            context,
            processor: None,
            source_size: (0, 0),
            width,
            height,
            ring: Vec::new(),
            next: 0,
            staging,
            buffer: vec![0; (width * height * 3 / 2) as usize],
        })
    }

    pub fn device(&self) -> &ID3D11Device {
        &self.device
    }

    pub fn context(&self) -> &ID3D11DeviceContext {
        &self.context
    }

    pub fn source_size(&self) -> (u32, u32) {
        self.source_size
    }

    /// Prepares for source images of `size`; rebuilds only when it changed.
    pub fn set_source(&mut self, size: (u32, u32)) -> Result<()> {
        if size == self.source_size && self.processor.is_some() {
            return Ok(());
        }
        self.processor = None;
        self.source_size = size;
        self.build_processor()
    }

    fn build_processor(&mut self) -> Result<()> {
        let (source_width, source_height) = self.source_size;
        let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputWidth: source_width,
            InputHeight: source_height,
            OutputWidth: self.width,
            OutputHeight: self.height,
            Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
            ..Default::default()
        };
        let enumerator = unsafe { self.video.CreateVideoProcessorEnumerator(&desc)? };
        let processor = unsafe { self.video.CreateVideoProcessor(&enumerator, 0)? };
        let view_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            ..Default::default()
        };
        self.ring.clear();
        for _ in 0..RING {
            let texture = texture(
                &self.device,
                self.width,
                self.height,
                DXGI_FORMAT_NV12,
                D3D11_USAGE_DEFAULT,
                D3D11_BIND_RENDER_TARGET.0 as u32,
                0,
                0,
            )?;
            let mut view = None;
            unsafe {
                self.video.CreateVideoProcessorOutputView(
                    &texture,
                    &enumerator,
                    &view_desc,
                    Some(&mut view),
                )?;
            }
            self.ring.push(Converted {
                texture,
                view: view.context("Missing video output view")?,
            });
        }
        // A screen of another shape than the stream keeps its shape, with
        // black bars, rather than being stretched.
        let (left, top, right, bottom) =
            crate::viewer::fit::even_pixel_rect(self.width, self.height, self.source_size)
                .context("Invalid screen size")?;
        unsafe {
            let source = RECT {
                left: 0,
                top: 0,
                right: source_width as i32,
                bottom: source_height as i32,
            };
            let destination = RECT {
                left,
                top,
                right,
                bottom,
            };
            let target = RECT {
                left: 0,
                top: 0,
                right: self.width as i32,
                bottom: self.height as i32,
            };
            self.video_context.VideoProcessorSetStreamSourceRect(
                &processor,
                0,
                true,
                Some(&source),
            );
            self.video_context.VideoProcessorSetStreamDestRect(
                &processor,
                0,
                true,
                Some(&destination),
            );
            self.video_context
                .VideoProcessorSetOutputTargetRect(&processor, true, Some(&target));
            let black = D3D11_VIDEO_COLOR {
                Anonymous: D3D11_VIDEO_COLOR_0 {
                    RGBA: D3D11_VIDEO_COLOR_RGBA {
                        R: 0.0,
                        G: 0.0,
                        B: 0.0,
                        A: 1.0,
                    },
                },
            };
            self.video_context
                .VideoProcessorSetOutputBackgroundColor(&processor, false, &black);
            self.video_context
                .VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
        }
        self.processor = Some((enumerator, processor));
        Ok(())
    }

    /// Converts `image`, which has the source size, to NV12. Returns the
    /// ring slot holding the result.
    pub fn convert(&mut self, image: &ID3D11Texture2D) -> Result<usize> {
        let (enumerator, processor) = self
            .processor
            .as_ref()
            .context("The video processor has no source size")?;
        let input_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            ..Default::default()
        };
        let mut input = None;
        unsafe {
            self.video.CreateVideoProcessorInputView(
                image,
                enumerator,
                &input_desc,
                Some(&mut input),
            )?;
        }
        let slot = self.next;
        self.next = (self.next + 1) % self.ring.len();
        let streams = [D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            pInputSurface: std::mem::ManuallyDrop::new(input),
            ..Default::default()
        }];
        let result = unsafe {
            self.video_context
                .VideoProcessorBlt(processor, &self.ring[slot].view, 0, &streams)
        };
        let [stream] = streams;
        drop(std::mem::ManuallyDrop::into_inner(stream.pInputSurface));
        result.context("Could not convert the screen image")?;
        Ok(slot)
    }

    pub fn texture(&self, slot: usize) -> &ID3D11Texture2D {
        &self.ring[slot].texture
    }

    /// Reads a converted frame back as tightly packed NV12.
    pub fn read(&mut self, slot: usize) -> Result<&[u8]> {
        unsafe {
            self.context
                .CopyResource(&self.staging, &self.ring[slot].texture)
        };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.context
                .Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        }
        let (width, height) = (self.width as usize, self.height as usize);
        let pitch = mapped.RowPitch as usize;
        let source = mapped.pData as *const u8;
        // NV12 stores the interleaved chroma plane directly after `height` luma rows.
        for row in 0..height + height / 2 {
            let from = unsafe { std::slice::from_raw_parts(source.add(row * pitch), width) };
            self.buffer[row * width..(row + 1) * width].copy_from_slice(from);
        }
        unsafe { self.context.Unmap(&self.staging, 0) };
        Ok(&self.buffer)
    }
}

pub struct Capture {
    output: IDXGIOutput1,
    duplication: Option<IDXGIOutputDuplication>,
    /// Latest desktop image without the pointer, kept so unchanged screens
    /// and pointer-only moves can be re-encoded.
    desktop: Option<ID3D11Texture2D>,
    /// The desktop image with the pointer drawn in; GDI-compatible when the
    /// desktop format allows it.
    composed: Option<(ID3D11Texture2D, bool)>,
    converter: Converter,
}

impl Capture {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let (adapter, output, _) = primary_output()?;
        let (device, _) = video_device(&adapter.cast()?)?;
        let mut capture = Self {
            output,
            duplication: None,
            desktop: None,
            composed: None,
            converter: Converter::new(&device, width, height)?,
        };
        capture.duplicate()?;
        Ok(capture)
    }

    pub fn device(&self) -> &ID3D11Device {
        self.converter.device()
    }

    fn duplicate(&mut self) -> Result<()> {
        self.duplication = None;
        let duplication = unsafe { self.output.DuplicateOutput(self.converter.device()) }
            .context("Could not capture the screen")?;
        let desc = unsafe { duplication.GetDesc() };
        let size = (desc.ModeDesc.Width, desc.ModeDesc.Height);
        if size != self.converter.source_size() {
            self.desktop = None;
            self.composed = None;
        }
        self.converter.set_source(size)?;
        self.duplication = Some(duplication);
        Ok(())
    }

    /// Waits up to `timeout` for a new desktop image or pointer change.
    /// Returns whether anything changed; the latest image stays available.
    pub fn next_frame(&mut self, timeout: Duration) -> Result<bool> {
        if self.duplication.is_none() {
            // Access is lost while the secure desktop (UAC, lock screen) is up.
            if self.duplicate().is_err() {
                std::thread::sleep(timeout);
                return Ok(false);
            }
        }
        let duplication = self.duplication.clone().unwrap();
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None;
        let result = unsafe {
            duplication.AcquireNextFrame(timeout.as_millis() as u32, &mut info, &mut resource)
        };
        match result {
            Ok(()) => {}
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(false),
            Err(error) if error.code() == DXGI_ERROR_ACCESS_LOST => {
                self.duplication = None;
                return Ok(false);
            }
            Err(error) => return Err(error).context("Screen capture failed"),
        }
        let image_changed = info.LastPresentTime != 0;
        if image_changed || self.desktop.is_none() {
            let resource = resource.context("Missing desktop image")?;
            let image: ID3D11Texture2D = resource.cast()?;
            let device = self.converter.device();
            if self.desktop.is_none() {
                let mut desc = D3D11_TEXTURE2D_DESC::default();
                unsafe { image.GetDesc(&mut desc) };
                self.desktop = Some(texture(
                    device,
                    desc.Width,
                    desc.Height,
                    desc.Format,
                    D3D11_USAGE_DEFAULT,
                    0,
                    0,
                    0,
                )?);
                // GDI can only draw the pointer into 8-bit BGRA surfaces.
                let gdi = desc.Format == DXGI_FORMAT_B8G8R8A8_UNORM;
                self.composed = Some((
                    texture(
                        device,
                        desc.Width,
                        desc.Height,
                        desc.Format,
                        D3D11_USAGE_DEFAULT,
                        D3D11_BIND_RENDER_TARGET.0 as u32,
                        0,
                        if gdi {
                            D3D11_RESOURCE_MISC_GDI_COMPATIBLE.0 as u32
                        } else {
                            0
                        },
                    )?,
                    gdi,
                ));
            }
            unsafe {
                self.converter
                    .context()
                    .CopyResource(self.desktop.as_ref().unwrap(), &image)
            };
        }
        unsafe {
            let _ = duplication.ReleaseFrame();
        }
        Ok(image_changed || info.LastMouseUpdateTime != 0)
    }

    /// Draws the pointer over the latest desktop image and converts it to
    /// NV12. Returns the ring slot holding the result.
    pub fn convert(&mut self) -> Result<Option<usize>> {
        let (Some(desktop), Some((composed, gdi))) = (&self.desktop, &self.composed) else {
            return Ok(None);
        };
        unsafe { self.converter.context().CopyResource(composed, desktop) };
        if *gdi {
            // A pointer that cannot be drawn should not stop the stream.
            if let Err(error) = draw_pointer(composed) {
                eprintln!("Display pointer: {error:#}");
            }
        }
        let composed = composed.clone();
        self.converter.convert(&composed).map(Some)
    }

    pub fn texture(&self, slot: usize) -> &ID3D11Texture2D {
        self.converter.texture(slot)
    }

    pub fn read(&mut self, slot: usize) -> Result<&[u8]> {
        self.converter.read(slot)
    }
}

/// Draws the current system pointer with GDI, which handles color,
/// monochrome and inverting pointers the same way the screen does.
fn draw_pointer(target: &ID3D11Texture2D) -> Result<()> {
    let mut cursor = CURSORINFO {
        cbSize: std::mem::size_of::<CURSORINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetCursorInfo(&mut cursor)? };
    if cursor.flags.0 & CURSOR_SHOWING.0 == 0 || cursor.hCursor.is_invalid() {
        return Ok(());
    }
    let mut icon = ICONINFO::default();
    unsafe { GetIconInfo(cursor.hCursor.into(), &mut icon)? };
    unsafe {
        // GetIconInfo hands us copies of the pointer bitmaps.
        if !icon.hbmMask.is_invalid() {
            let _ = DeleteObject(icon.hbmMask.into());
        }
        if !icon.hbmColor.is_invalid() {
            let _ = DeleteObject(icon.hbmColor.into());
        }
    }
    let surface: IDXGISurface1 = target.cast()?;
    let dc = unsafe { surface.GetDC(false)? };
    let drawn = unsafe {
        DrawIconEx(
            dc,
            cursor.ptScreenPos.x - icon.xHotspot as i32,
            cursor.ptScreenPos.y - icon.yHotspot as i32,
            cursor.hCursor.into(),
            0,
            0,
            0,
            None,
            DI_NORMAL,
        )
    };
    unsafe { surface.ReleaseDC(None)? };
    drawn.context("Could not draw the pointer")
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
    usage: D3D11_USAGE,
    bind: u32,
    cpu: u32,
    misc: u32,
) -> Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: usage,
        BindFlags: bind,
        CPUAccessFlags: cpu,
        MiscFlags: misc,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    texture.context("Missing texture")
}
