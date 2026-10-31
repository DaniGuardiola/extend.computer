//! H.264 encoding with a synchronous Media Foundation transform. Output is an
//! Annex B stream whose keyframes always carry SPS and PPS, which the
//! receivers need to start or restart decoding.
use anyhow::{ensure, Context, Result};
use windows::{
    core::Interface,
    Win32::{
        Media::MediaFoundation::*,
        System::{
            Com::{CoInitializeEx, COINIT_MULTITHREADED},
            Variant::VARIANT,
        },
    },
};

pub struct Frame {
    pub timestamp: u64,
    pub keyframe: bool,
    pub bytes: Vec<u8>,
}

pub struct Encoder {
    transform: IMFTransform,
    codec: Option<ICodecAPI>,
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
    pub fn new(width: u32, height: u32, fps: u32, bitrate: u32) -> Result<Self> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET)
                .context("Could not start Media Foundation")?;
        }
        let transform = find_encoder()?;
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

    pub fn set_bitrate(&mut self, bitrate: u32) -> Result<()> {
        if let Some(codec) = &self.codec {
            unsafe { codec.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &variant_u32(bitrate))? };
        }
        Ok(())
    }

    pub fn encode(&mut self, nv12: &[u8], timestamp: u64, keyframe: bool) -> Result<Vec<Frame>> {
        ensure!(
            nv12.len() == (self.width * self.height * 3 / 2) as usize,
            "Invalid NV12 frame"
        );
        if keyframe {
            if let Some(codec) = &self.codec {
                unsafe {
                    let _ = codec.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &variant_u32(1));
                }
            }
        }
        unsafe {
            let buffer = MFCreateMemoryBuffer(nv12.len() as u32)?;
            let mut data = std::ptr::null_mut();
            buffer.Lock(&mut data, None, None)?;
            std::ptr::copy_nonoverlapping(nv12.as_ptr(), data, nv12.len());
            buffer.Unlock()?;
            buffer.SetCurrentLength(nv12.len() as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            // Media Foundation time is in 100 ns units.
            sample.SetSampleTime((timestamp / 100) as i64)?;
            sample.SetSampleDuration((10_000_000 / self.fps) as i64)?;
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

fn find_encoder() -> Result<IMFTransform> {
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_NV12,
    };
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_H264,
    };
    let mut activates = std::ptr::null_mut();
    let mut count = 0;
    unsafe {
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_ENCODER,
            MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_LOCALMFT | MFT_ENUM_FLAG_SORTANDFILTER,
            Some(&input),
            Some(&output),
            &mut activates,
            &mut count,
        )?;
        let list = std::slice::from_raw_parts_mut(activates, count as usize);
        let mut transform = None;
        // Taking each entry releases it; the array itself is freed below.
        for slot in list.iter_mut() {
            if let Some(activate) = slot.take() {
                if transform.is_none() {
                    transform = activate.ActivateObject::<IMFTransform>().ok();
                }
            }
        }
        windows::Win32::System::Com::CoTaskMemFree(Some(activates as *const _));
        transform.context("No H.264 encoder is available")
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
