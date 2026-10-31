//! Shows decoded pictures in the viewer window: a flip-model swap chain with
//! a frame latency waitable object, and the D3D11 video processor converting
//! NV12 to BGRA, fitting the picture in the window with black bars.
use super::{decode::Picture, fit};
use anyhow::{Context, Result};
use std::time::{Duration, Instant};
use windows::{
    core::Interface,
    Win32::{
        Foundation::{CloseHandle, HANDLE, HWND, RECT},
        Graphics::{
            Direct3D11::*,
            Dxgi::{Common::*, *},
        },
    },
};

const FLAGS: DXGI_SWAP_CHAIN_FLAG = DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT;

struct Processor {
    input: (u32, u32),
    output: (u32, u32),
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    output_view: ID3D11VideoProcessorOutputView,
    /// Input views by texture; the decoder reuses a few textures.
    inputs: Vec<(ID3D11Texture2D, ID3D11VideoProcessorInputView)>,
}

pub struct Presenter {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    video: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    video_context1: Option<ID3D11VideoContext1>,
    swap_chain: IDXGISwapChain2,
    /// Signaled when the swap chain can take another frame without queuing.
    pub waitable: HANDLE,
    pub size: (u32, u32),
    target: Option<(ID3D11Texture2D, ID3D11RenderTargetView)>,
    processor: Option<Processor>,
}

impl Presenter {
    pub fn new(device: &ID3D11Device, hwnd: HWND, width: u32, height: u32) -> Result<Self> {
        let factory: IDXGIFactory2 = unsafe {
            device
                .cast::<IDXGIDevice>()?
                .GetAdapter()?
                .GetParent()
                .context("Missing DXGI factory")?
        };
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            Stereo: false.into(),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            Scaling: DXGI_SCALING_NONE,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            Flags: FLAGS.0 as u32,
        };
        let swap_chain: IDXGISwapChain2 = unsafe {
            let swap_chain = factory
                .CreateSwapChainForHwnd(device, hwnd, &desc, None, None)
                .context("Could not create the viewer's swap chain")?;
            // The window is already borderless full screen; DXGI's Alt+Enter
            // would switch to exclusive mode.
            factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER)?;
            swap_chain.cast()?
        };
        unsafe { swap_chain.SetMaximumFrameLatency(1)? };
        let waitable = unsafe { swap_chain.GetFrameLatencyWaitableObject() };
        let context = unsafe { device.GetImmediateContext()? };
        Ok(Self {
            video: device.cast()?,
            video_context: context.cast()?,
            video_context1: context.cast().ok(),
            device: device.clone(),
            context,
            swap_chain,
            waitable,
            size: (width, height),
            target: None,
            processor: None,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if (width, height) == self.size || width == 0 || height == 0 {
            return Ok(());
        }
        // Every reference to the old buffers goes first.
        self.target = None;
        self.processor = None;
        unsafe {
            self.swap_chain
                .ResizeBuffers(0, width, height, DXGI_FORMAT_UNKNOWN, FLAGS)
                .context("Could not resize the viewer")?;
        }
        self.size = (width, height);
        Ok(())
    }

    /// Draws `picture`, or black before the first one, and presents it.
    /// Always presents once, which balances the wait on [`Self::waitable`].
    pub fn draw(&mut self, picture: Option<&Picture>) -> Result<()> {
        if self.target.is_none() {
            let buffer: ID3D11Texture2D = unsafe { self.swap_chain.GetBuffer(0)? };
            let mut view = None;
            unsafe {
                self.device
                    .CreateRenderTargetView(&buffer, None, Some(&mut view))?
            };
            self.target = Some((buffer, view.context("Missing render target")?));
        }
        let drawn = picture.map(|picture| self.blit(picture));
        if !matches!(drawn, Some(Ok(()))) {
            let (_, view) = self.target.as_ref().unwrap();
            unsafe {
                self.context
                    .ClearRenderTargetView(view, &[0.0, 0.0, 0.0, 1.0])
            };
        }
        // Without vsync a newer frame replaces one still waiting for the
        // display; the flip model never tears without ALLOW_TEARING.
        let presented = unsafe { self.swap_chain.Present(0, DXGI_PRESENT(0)) }.ok();
        drawn.transpose().context("Could not draw the picture")?;
        presented.context("Could not present the picture")
    }

    fn blit(&mut self, picture: &Picture) -> Result<()> {
        let input = (picture.width, picture.height);
        if self
            .processor
            .as_ref()
            .is_none_or(|p| p.input != input || p.output != self.size)
        {
            self.processor = None;
            self.processor = Some(self.build_processor(input)?);
        }
        let processor = self.processor.as_mut().unwrap();
        let view = match processor
            .inputs
            .iter()
            .find(|(texture, _)| texture == &picture.texture)
        {
            Some((_, view)) => view.clone(),
            None => {
                let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                    FourCC: 0,
                    ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                    ..Default::default()
                };
                let mut view = None;
                unsafe {
                    self.video.CreateVideoProcessorInputView(
                        &picture.texture,
                        &processor.enumerator,
                        &desc,
                        Some(&mut view),
                    )?;
                }
                let view = view.context("Missing video input view")?;
                if processor.inputs.len() >= 8 {
                    processor.inputs.clear();
                }
                processor
                    .inputs
                    .push((picture.texture.clone(), view.clone()));
                view
            }
        };
        let (width, height) = self.size;
        let (left, top, right, bottom) =
            fit::pixel_rect(width, height, input).context("Invalid picture size")?;
        let source = RECT {
            left: 0,
            top: 0,
            right: input.0 as i32,
            bottom: input.1 as i32,
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
            right: width as i32,
            bottom: height as i32,
        };
        let processor_ref = &processor.processor;
        unsafe {
            let video = &self.video_context;
            video.VideoProcessorSetStreamFrameFormat(
                processor_ref,
                0,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            );
            video.VideoProcessorSetStreamSourceRect(processor_ref, 0, true, Some(&source));
            video.VideoProcessorSetStreamDestRect(processor_ref, 0, true, Some(&destination));
            video.VideoProcessorSetOutputTargetRect(processor_ref, true, Some(&target));
            video.VideoProcessorSetStreamAutoProcessingMode(processor_ref, 0, false);
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
            video.VideoProcessorSetOutputBackgroundColor(processor_ref, false, &black);
            match &self.video_context1 {
                Some(video) => {
                    let space = match (picture.color.bt601, picture.color.full_range) {
                        (false, false) => DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
                        (false, true) => DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P709,
                        (true, false) => DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P601,
                        (true, true) => DXGI_COLOR_SPACE_YCBCR_FULL_G22_NONE_P709_X601,
                    };
                    video.VideoProcessorSetStreamColorSpace1(processor_ref, 0, space);
                    video.VideoProcessorSetOutputColorSpace1(
                        processor_ref,
                        DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
                    );
                }
                None => {
                    // Bits: YCbCr_Matrix (1 = BT.709) at 2, Nominal_Range
                    // (1 = 16-235, 2 = 0-255) at 4; RGB output is full range.
                    let matrix = u32::from(!picture.color.bt601) << 2;
                    let range = if picture.color.full_range { 2 } else { 1 } << 4;
                    video.VideoProcessorSetStreamColorSpace(
                        processor_ref,
                        0,
                        &D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
                            _bitfield: matrix | range,
                        },
                    );
                    video.VideoProcessorSetOutputColorSpace(
                        processor_ref,
                        &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0 },
                    );
                }
            }
        }
        let streams = [D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            pInputSurface: std::mem::ManuallyDrop::new(Some(view)),
            ..Default::default()
        }];
        let result = unsafe {
            self.video_context
                .VideoProcessorBlt(processor_ref, &processor.output_view, 0, &streams)
        };
        let [stream] = streams;
        drop(std::mem::ManuallyDrop::into_inner(stream.pInputSurface));
        result.context("Could not convert the picture")
    }

    fn build_processor(&self, input: (u32, u32)) -> Result<Processor> {
        let output = self.size;
        let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputWidth: input.0,
            InputHeight: input.1,
            OutputWidth: output.0,
            OutputHeight: output.1,
            Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
            ..Default::default()
        };
        let enumerator = unsafe { self.video.CreateVideoProcessorEnumerator(&desc)? };
        let processor = unsafe { self.video.CreateVideoProcessor(&enumerator, 0)? };
        let (buffer, _) = self.target.as_ref().context("Missing back buffer")?;
        let view_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            ..Default::default()
        };
        let mut output_view = None;
        unsafe {
            self.video.CreateVideoProcessorOutputView(
                buffer,
                &enumerator,
                &view_desc,
                Some(&mut output_view),
            )?;
        }
        Ok(Processor {
            input,
            output,
            enumerator,
            processor,
            output_view: output_view.context("Missing video output view")?,
            inputs: Vec::new(),
        })
    }

    /// Waits for the GPU to finish what was submitted, up to `timeout`.
    pub fn finished(&self, timeout: Duration) -> bool {
        let desc = D3D11_QUERY_DESC {
            Query: D3D11_QUERY_EVENT,
            MiscFlags: 0,
        };
        let mut query = None;
        if unsafe { self.device.CreateQuery(&desc, Some(&mut query)) }.is_err() {
            return false;
        }
        let Some(query) = query else { return false };
        unsafe { self.context.End(&query) };
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let mut done = windows::core::BOOL(0);
            let result = unsafe {
                self.context.GetData(
                    &query,
                    Some(&mut done as *mut _ as *mut _),
                    std::mem::size_of::<windows::core::BOOL>() as u32,
                    0,
                )
            };
            // S_FALSE, still running, is a success code that leaves `done` unset.
            if result.is_ok() && done.as_bool() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        false
    }
}

impl Drop for Presenter {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.waitable);
        }
    }
}
