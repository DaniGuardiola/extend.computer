//! Primary-display capture with DXGI Desktop Duplication. The D3D11 video
//! processor scales each frame to the stream size and converts it to NV12 on
//! the GPU; the result is read back for the encoder.
use anyhow::{bail, Context, Result};
use std::time::Duration;
use windows::{
    core::Interface,
    Win32::{
        Foundation::{HMODULE, RECT},
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
            Direct3D11::*,
            Dxgi::{Common::*, *},
        },
        System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
        UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2},
    },
};

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

pub fn primary_size() -> Result<(u32, u32)> {
    let (_, _, r) = primary_output()?;
    Ok(((r.right - r.left) as u32, (r.bottom - r.top) as u32))
}

pub struct Capture {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    output: IDXGIOutput1,
    duplication: Option<IDXGIOutputDuplication>,
    /// Latest desktop image, kept so unchanged screens can be re-encoded.
    desktop: Option<ID3D11Texture2D>,
    source_size: (u32, u32),
    width: u32,
    height: u32,
    video: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    processor: Option<(ID3D11VideoProcessorEnumerator, ID3D11VideoProcessor)>,
    nv12: ID3D11Texture2D,
    nv12_view: Option<ID3D11VideoProcessorOutputView>,
    staging: ID3D11Texture2D,
    buffer: Vec<u8>,
}

impl Capture {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let (adapter, output, _) = primary_output()?;
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                &adapter,
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
        let video: ID3D11VideoDevice = device.cast()?;
        let video_context: ID3D11VideoContext = context.cast()?;
        let nv12 = texture(
            &device,
            width,
            height,
            DXGI_FORMAT_NV12,
            D3D11_USAGE_DEFAULT,
            D3D11_BIND_RENDER_TARGET.0 as u32,
            0,
        )?;
        let staging = texture(
            &device,
            width,
            height,
            DXGI_FORMAT_NV12,
            D3D11_USAGE_STAGING,
            0,
            D3D11_CPU_ACCESS_READ.0 as u32,
        )?;
        let mut capture = Self {
            device,
            context,
            output,
            duplication: None,
            desktop: None,
            source_size: (0, 0),
            width,
            height,
            video,
            video_context,
            processor: None,
            nv12_view: None,
            nv12,
            staging,
            buffer: vec![0; (width * height * 3 / 2) as usize],
        };
        // The output view depends on the processor enumerator, which needs
        // the desktop size; duplicate() builds both.
        capture.duplicate()?;
        Ok(capture)
    }

    fn duplicate(&mut self) -> Result<()> {
        self.duplication = None;
        let duplication = unsafe { self.output.DuplicateOutput(&self.device) }
            .context("Could not capture the screen")?;
        let desc = unsafe { duplication.GetDesc() };
        let size = (desc.ModeDesc.Width, desc.ModeDesc.Height);
        if size != self.source_size || self.processor.is_none() {
            self.source_size = size;
            self.desktop = None;
            self.build_processor()?;
        }
        self.duplication = Some(duplication);
        Ok(())
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
        let mut view = None;
        unsafe {
            self.video.CreateVideoProcessorOutputView(
                &self.nv12,
                &enumerator,
                &view_desc,
                Some(&mut view),
            )?;
        }
        self.nv12_view = Some(view.context("Missing video output view")?);
        unsafe {
            // Keep the full desktop and letterbox nothing: the stream size
            // follows the screen aspect chosen by the app.
            let rect = RECT {
                left: 0,
                top: 0,
                right: source_width as i32,
                bottom: source_height as i32,
            };
            self.video_context
                .VideoProcessorSetStreamSourceRect(&processor, 0, true, Some(&rect));
            let target = RECT {
                left: 0,
                top: 0,
                right: self.width as i32,
                bottom: self.height as i32,
            };
            self.video_context
                .VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&target));
            self.video_context
                .VideoProcessorSetOutputTargetRect(&processor, true, Some(&target));
            self.video_context
                .VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
        }
        self.processor = Some((enumerator, processor));
        Ok(())
    }

    /// Waits up to `timeout` for a new desktop image. Returns whether the
    /// screen changed; the latest image stays available either way.
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
        let changed = info.LastPresentTime != 0;
        if changed || self.desktop.is_none() {
            let resource = resource.context("Missing desktop image")?;
            let image: ID3D11Texture2D = resource.cast()?;
            let copy = match &self.desktop {
                Some(copy) => copy.clone(),
                None => {
                    let mut desc = D3D11_TEXTURE2D_DESC::default();
                    unsafe { image.GetDesc(&mut desc) };
                    let copy = texture(
                        &self.device,
                        desc.Width,
                        desc.Height,
                        desc.Format,
                        D3D11_USAGE_DEFAULT,
                        D3D11_BIND_RENDER_TARGET.0 as u32,
                        0,
                    )?;
                    self.desktop = Some(copy.clone());
                    copy
                }
            };
            unsafe { self.context.CopyResource(&copy, &image) };
        }
        unsafe {
            let _ = duplication.ReleaseFrame();
        }
        Ok(changed)
    }

    /// Scales and converts the latest desktop image to tightly packed NV12.
    pub fn read_nv12(&mut self) -> Result<Option<&[u8]>> {
        let (Some(desktop), Some((enumerator, processor)), Some(output)) =
            (&self.desktop, &self.processor, &self.nv12_view)
        else {
            return Ok(None);
        };
        let input_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            ..Default::default()
        };
        let mut input = None;
        unsafe {
            self.video.CreateVideoProcessorInputView(
                desktop,
                enumerator,
                &input_desc,
                Some(&mut input),
            )?;
        }
        let stream = D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            pInputSurface: std::mem::ManuallyDrop::new(input),
            ..Default::default()
        };
        let streams = [stream];
        let result = unsafe {
            self.video_context
                .VideoProcessorBlt(processor, output, 0, &streams)
        };
        let [stream] = streams;
        drop(std::mem::ManuallyDrop::into_inner(stream.pInputSurface));
        result.context("Could not convert the screen image")?;
        unsafe { self.context.CopyResource(&self.staging, &self.nv12) };
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
        Ok(Some(&self.buffer))
    }
}

fn texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
    usage: D3D11_USAGE,
    bind: u32,
    cpu: u32,
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
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    texture.context("Missing texture")
}
