//! H.264 encoding with Media Foundation. Hardware encoders (NVENC, Quick
//! Sync, AMF) read converted frames straight from GPU textures; the built-in
//! software encoder is the fallback and reads them back to memory. Output is
//! an Annex B stream whose keyframes always carry SPS and PPS, which the
//! receivers need to start or restart decoding.
use anyhow::{bail, ensure, Context, Result};
use extend_computer_agent::display::Frame;
use std::time::{Duration, Instant};
use windows::{
    core::Interface,
    Win32::{
        Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D},
        Media::MediaFoundation::*,
        System::{
            Com::{CoInitializeEx, COINIT_MULTITHREADED},
            Variant::VARIANT,
        },
    },
};

/// How long a hardware encoder may take to return a submitted frame before
/// it is collected on the next call instead.
const OUTPUT_WAIT: Duration = Duration::from_millis(40);

/// Where the encoder reads its input.
pub enum Input<'a> {
    Texture(&'a ID3D11Texture2D),
    Memory(&'a [u8]),
}

struct Hardware {
    events: IMFMediaEventGenerator,
    /// Inputs the encoder has asked for and not yet received.
    credits: u32,
    _manager: IMFDXGIDeviceManager,
}

pub struct Encoder {
    transform: IMFTransform,
    codec: Option<ICodecAPI>,
    hardware: Option<Hardware>,
    pub name: String,
    width: u32,
    height: u32,
    fps: u32,
    provides_samples: bool,
    output_size: u32,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
}

fn pack(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

fn variant_u32(value: u32) -> VARIANT {
    VARIANT::from(value)
}

impl Encoder {
    /// Prefers a hardware encoder on `device`'s adapter unless
    /// `EXTEND_DISPLAY_ENCODER=software`.
    pub fn new(
        device: &ID3D11Device,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
    ) -> Result<Self> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET)
                .context("Could not start Media Foundation")?;
        }
        let software_only = std::env::var("EXTEND_DISPLAY_ENCODER").is_ok_and(|v| v == "software");
        if !software_only {
            for (activate, name) in encoders(true)? {
                match Self::start(
                    activate,
                    name.clone(),
                    Some(device),
                    width,
                    height,
                    fps,
                    bitrate,
                ) {
                    Ok(encoder) => return Ok(encoder),
                    Err(error) => eprintln!("Display encoder {name} unavailable: {error:#}"),
                }
            }
        }
        for (activate, name) in encoders(false)? {
            match Self::start(activate, name.clone(), None, width, height, fps, bitrate) {
                Ok(encoder) => return Ok(encoder),
                Err(error) => eprintln!("Display encoder {name} unavailable: {error:#}"),
            }
        }
        bail!("No H.264 encoder is available")
    }

    fn start(
        activate: IMFActivate,
        name: String,
        device: Option<&ID3D11Device>,
        width: u32,
        height: u32,
        fps: u32,
        bitrate: u32,
    ) -> Result<Self> {
        let transform: IMFTransform = unsafe { activate.ActivateObject()? };
        let hardware = match device {
            Some(device) => unsafe {
                let attributes = transform.GetAttributes()?;
                // Async encoders refuse calls until the caller opts in.
                attributes.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)?;
                let mut token = 0;
                let mut manager = None;
                MFCreateDXGIDeviceManager(&mut token, &mut manager)?;
                let manager = manager.context("Missing DXGI device manager")?;
                manager.ResetDevice(device, token)?;
                transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize)?;
                Some(Hardware {
                    events: transform.cast()?,
                    credits: 0,
                    _manager: manager,
                })
            },
            None => None,
        };
        let codec: Option<ICodecAPI> = transform.cast().ok();
        if let Some(codec) = &codec {
            unsafe {
                // Every frame is shown as soon as it arrives: no B-frames or lookahead.
                let _ = codec.SetValue(&CODECAPI_AVLowLatencyMode, &VARIANT::from(true));
                let _ = codec.SetValue(
                    &CODECAPI_AVEncCommonRateControlMode,
                    &variant_u32(eAVEncCommonRateControlMode_CBR.0 as u32),
                );
                let _ = codec.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &variant_u32(bitrate));
                let _ = codec.SetValue(&CODECAPI_AVEncMPVDefaultBPictureCount, &variant_u32(0));
                // Keyframes come on request; an occasional one bounds error spread.
                let _ = codec.SetValue(&CODECAPI_AVEncMPVGOPSize, &variant_u32(fps * 10));
            }
        }
        unsafe {
            let output = MFCreateMediaType()?;
            output.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            output.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
            output.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
            output.SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height))?;
            output.SetUINT64(&MF_MT_FRAME_RATE, pack(fps, 1))?;
            output.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
            output.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            output.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)?;
            transform
                .SetOutputType(0, &output, 0)
                .context("The H.264 encoder rejected the stream settings")?;
            let input = MFCreateMediaType()?;
            input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
            input.SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height))?;
            input.SetUINT64(&MF_MT_FRAME_RATE, pack(fps, 1))?;
            input.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
            input.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            transform
                .SetInputType(0, &input, 0)
                .context("The H.264 encoder rejected NV12 input")?;
            let info = transform.GetOutputStreamInfo(0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
            Ok(Self {
                transform,
                codec,
                hardware,
                name,
                width,
                height,
                fps,
                provides_samples: info.dwFlags
                    & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0
                        | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0) as u32
                    != 0,
                output_size: info.cbSize.max(width * height * 2),
                sps: None,
                pps: None,
            })
        }
    }

    pub fn wants_texture(&self) -> bool {
        self.hardware.is_some()
    }

    pub fn set_bitrate(&mut self, bitrate: u32) -> Result<()> {
        if let Some(codec) = &self.codec {
            unsafe { codec.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &variant_u32(bitrate))? };
        }
        Ok(())
    }

    pub fn encode(&mut self, input: Input, timestamp: u64, keyframe: bool) -> Result<Vec<Frame>> {
        if keyframe {
            if let Some(codec) = &self.codec {
                unsafe {
                    let _ = codec.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &variant_u32(1));
                }
            }
        }
        let sample = unsafe { MFCreateSample()? };
        unsafe {
            match input {
                Input::Texture(texture) => {
                    ensure!(self.hardware.is_some(), "This encoder reads memory frames");
                    let buffer =
                        MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, texture, 0, false)?;
                    let length = buffer.cast::<IMF2DBuffer>()?.GetContiguousLength()?;
                    buffer.SetCurrentLength(length)?;
                    sample.AddBuffer(&buffer)?;
                }
                Input::Memory(nv12) => {
                    ensure!(
                        nv12.len() == (self.width * self.height * 3 / 2) as usize,
                        "Invalid NV12 frame"
                    );
                    let buffer = MFCreateMemoryBuffer(nv12.len() as u32)?;
                    let mut data = std::ptr::null_mut();
                    buffer.Lock(&mut data, None, None)?;
                    std::ptr::copy_nonoverlapping(nv12.as_ptr(), data, nv12.len());
                    buffer.Unlock()?;
                    buffer.SetCurrentLength(nv12.len() as u32)?;
                    sample.AddBuffer(&buffer)?;
                }
            }
            // Media Foundation time is in 100 ns units.
            sample.SetSampleTime((timestamp / 100) as i64)?;
            sample.SetSampleDuration((10_000_000 / self.fps) as i64)?;
        }
        if self.hardware.is_some() {
            self.encode_async(&sample)
        } else {
            unsafe {
                self.transform
                    .ProcessInput(0, &sample, 0)
                    .context("H.264 encoding failed")?;
            }
            let mut frames = Vec::new();
            while let Some(frame) = self.output()? {
                frames.push(frame);
            }
            Ok(frames)
        }
    }

    /// Async encoders announce when they accept input and when output is
    /// ready. Submit once a slot is free, then wait briefly for the result
    /// so each frame normally leaves in the same call.
    fn encode_async(&mut self, sample: &IMFSample) -> Result<Vec<Frame>> {
        let mut frames = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.hardware.as_ref().unwrap().credits == 0 {
            ensure!(
                Instant::now() < deadline,
                "The hardware encoder stopped accepting frames"
            );
            if !self.pump(&mut frames)? {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        unsafe {
            self.transform
                .ProcessInput(0, sample, 0)
                .context("H.264 encoding failed")?;
        }
        self.hardware.as_mut().unwrap().credits -= 1;
        let submitted = frames.len();
        let deadline = Instant::now() + OUTPUT_WAIT;
        while frames.len() == submitted && Instant::now() < deadline {
            if !self.pump(&mut frames)? {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        // Pick up any further events without waiting.
        while self.pump(&mut frames)? {}
        Ok(frames)
    }

    /// Handles one pending encoder event. Returns false when none is queued.
    fn pump(&mut self, frames: &mut Vec<Frame>) -> Result<bool> {
        let hardware = self.hardware.as_mut().unwrap();
        let event = match unsafe { hardware.events.GetEvent(MF_EVENT_FLAG_NO_WAIT) } {
            Ok(event) => event,
            Err(error) if error.code() == MF_E_NO_EVENTS_AVAILABLE => return Ok(false),
            Err(error) => return Err(error).context("The hardware encoder failed"),
        };
        let kind = unsafe { event.GetType()? };
        if kind == METransformNeedInput.0 as u32 {
            hardware.credits += 1;
        } else if kind == METransformHaveOutput.0 as u32 {
            if let Some(frame) = self.output()? {
                frames.push(frame);
            }
        } else if kind == MEError.0 as u32 {
            let status = unsafe { event.GetStatus()? };
            bail!("The hardware encoder failed: {status:?}");
        }
        Ok(true)
    }

    fn output(&mut self) -> Result<Option<Frame>> {
        let sample = if self.provides_samples {
            None
        } else {
            unsafe {
                let sample = MFCreateSample()?;
                sample.AddBuffer(&MFCreateMemoryBuffer(self.output_size)?)?;
                Some(sample)
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
            Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(None),
            Err(error) => return Err(error).context("H.264 encoding failed"),
        }
        let sample = sample.context("The encoder returned no sample")?;
        unsafe {
            let timestamp = sample.GetSampleTime().unwrap_or(0).max(0) as u64 * 100;
            let clean_point = sample.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) != 0;
            let buffer = sample.ConvertToContiguousBuffer()?;
            let mut data = std::ptr::null_mut();
            let mut length = 0;
            buffer.Lock(&mut data, None, Some(&mut length))?;
            let encoded = std::slice::from_raw_parts(data, length as usize).to_vec();
            buffer.Unlock()?;
            Ok(Some(self.annex_b(&encoded, timestamp, clean_point)))
        }
    }

    /// Normalizes the encoder output: drops access unit delimiters and SEI,
    /// remembers parameter sets, and puts SPS/PPS in front of every keyframe.
    fn annex_b(&mut self, encoded: &[u8], timestamp: u64, clean_point: bool) -> Frame {
        let mut picture = Vec::with_capacity(encoded.len());
        let mut idr = false;
        for nal in nals(encoded) {
            match nal.first().map(|b| b & 31) {
                Some(7) => self.sps = Some(nal.to_vec()),
                Some(8) => self.pps = Some(nal.to_vec()),
                Some(6) | Some(9) | None => {}
                Some(kind) => {
                    idr |= kind == 5;
                    picture.extend_from_slice(&[0, 0, 0, 1]);
                    picture.extend_from_slice(nal);
                }
            }
        }
        let keyframe = idr || clean_point;
        let mut bytes = Vec::with_capacity(picture.len() + 64);
        if keyframe {
            for set in [&self.sps, &self.pps].into_iter().flatten() {
                bytes.extend_from_slice(&[0, 0, 0, 1]);
                bytes.extend_from_slice(set);
            }
        }
        bytes.extend_from_slice(&picture);
        Frame {
            timestamp,
            keyframe,
            bytes,
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            let _ = self
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
            let _ = self
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
            if let Some(hardware) = &self.hardware {
                let _ = hardware.events.cast::<IMFShutdown>().map(|s| s.Shutdown());
            }
        }
    }
}

/// Splits an Annex B stream into NAL units without their start codes.
fn nals(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push((i, i + 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut units = Vec::with_capacity(starts.len());
    for (index, &(_, begin)) in starts.iter().enumerate() {
        let mut end = starts.get(index + 1).map_or(data.len(), |&(next, _)| next);
        // A four-byte start code leaves one zero byte before the next unit.
        while end > begin && data[end - 1] == 0 && index + 1 < starts.len() {
            end -= 1;
        }
        units.push(&data[begin..end]);
    }
    units
}

/// Registered NV12 to H.264 encoders, best first, with their names.
fn encoders(hardware: bool) -> Result<Vec<(IMFActivate, String)>> {
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_NV12,
    };
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_H264,
    };
    let flags = if hardware {
        MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER
    } else {
        MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_LOCALMFT | MFT_ENUM_FLAG_SORTANDFILTER
    };
    let mut activates = std::ptr::null_mut();
    let mut count = 0;
    let mut found = Vec::new();
    unsafe {
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_ENCODER,
            flags,
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
                let name = friendly_name(&activate).unwrap_or_else(|| "H.264 encoder".into());
                found.push((activate, name));
            }
        }
        windows::Win32::System::Com::CoTaskMemFree(Some(activates as *const _));
    }
    Ok(found)
}

pub(crate) fn friendly_name(activate: &IMFActivate) -> Option<String> {
    let mut text = windows::core::PWSTR::null();
    let mut length = 0;
    unsafe {
        activate
            .GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut text, &mut length)
            .ok()?;
        let name = text.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(text.0 as *const _));
        name
    }
}

#[cfg(test)]
mod tests {
    use super::nals;

    #[test]
    fn splits_three_and_four_byte_start_codes() {
        let data = [
            0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 4,
        ];
        let units = nals(&data);
        assert_eq!(
            units,
            vec![&[0x67, 1, 2][..], &[0x68, 3][..], &[0x65, 4][..]]
        );
    }
}
