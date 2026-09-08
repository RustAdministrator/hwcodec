#[cfg(any(
    target_os = "windows",
    target_os = "linux",
    target_os = "macos",
    target_os = "ios"
))]
use super::Priority;
use crate::common::TEST_TIMEOUT_MS;
use crate::ffmpeg::{init_av_log, AVHWDeviceType::*};

use crate::{
    common::DataFormat::*,
    ffmpeg::{AVHWDeviceType, AVPixelFormat},
    ffmpeg_ram::{
        ffmpeg_ram_decode, ffmpeg_ram_free_decoder, ffmpeg_ram_new_decoder, CodecInfo,
        AV_NUM_DATA_POINTERS,
    },
};
use log::error;
use std::{
    ffi::{c_void, CString},
    os::raw::c_int,
    slice::from_raw_parts,
    time::Instant,
    vec,
};

#[derive(Debug, Clone)]
pub struct DecodeContext {
    pub name: String,
    pub device_type: AVHWDeviceType,
    pub thread_count: i32,
}

pub struct DecodeFrame {
    pub pixfmt: AVPixelFormat,
    pub width: i32,
    pub height: i32,
    pub data: Vec<Vec<u8>>,
    pub linesize: Vec<i32>,
    pub key: bool,
}

impl std::fmt::Display for DecodeFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = String::from("data:");
        for data in self.data.iter() {
            s.push_str(format!("{} ", data.len()).as_str());
        }
        s.push_str(", linesize:");
        for linesize in self.linesize.iter() {
            s.push_str(format!("{} ", linesize).as_str());
        }

        write!(
            f,
            "fixfmt:{}, width:{}, height:{},key:{}, {}",
            self.pixfmt as i32, self.width, self.height, self.key, s,
        )
    }
}

pub struct Decoder {
    codec: *mut c_void,
    frames: *mut Vec<DecodeFrame>,
    pub ctx: DecodeContext,
}

unsafe impl Send for Decoder {}
unsafe impl Sync for Decoder {}

impl Decoder {
    pub fn new(ctx: DecodeContext) -> Result<Self, ()> {
        init_av_log();
        unsafe {
            let codec = ffmpeg_ram_new_decoder(
                CString::new(ctx.name.as_str()).map_err(|_| ())?.as_ptr(),
                ctx.device_type as _,
                ctx.thread_count,
                Some(Decoder::callback),
            );

            if codec.is_null() {
                return Err(());
            }

            Ok(Decoder {
                codec,
                frames: Box::into_raw(Box::new(Vec::<DecodeFrame>::new())),
                ctx,
            })
        }
    }

    pub fn decode(&mut self, packet: &[u8]) -> Result<&mut Vec<DecodeFrame>, i32> {
        unsafe {
            (&mut *self.frames).clear();
            let ret = ffmpeg_ram_decode(
                self.codec,
                packet.as_ptr(),
                packet.len() as c_int,
                self.frames as *const _ as *const c_void,
            );

            if ret < 0 {
                Err(ret)
            } else {
                Ok(&mut *self.frames)
            }
        }
    }

    unsafe extern "C" fn callback(
        obj: *const c_void,
        width: c_int,
        height: c_int,
        pixfmt: c_int,
        linesizes: *mut c_int,
        datas: *mut *mut u8,
        key: c_int,
    ) {
        let frames = &mut *(obj as *mut Vec<DecodeFrame>);
        let datas = from_raw_parts(datas, AV_NUM_DATA_POINTERS as _);
        let linesizes = from_raw_parts(linesizes, AV_NUM_DATA_POINTERS as _);

        let mut frame = DecodeFrame {
            pixfmt: std::mem::transmute(pixfmt),
            width,
            height,
            data: vec![],
            linesize: vec![],
            key: key != 0,
        };

        if pixfmt == AVPixelFormat::AV_PIX_FMT_YUV420P as c_int {
            let y = from_raw_parts(datas[0], (linesizes[0] * height) as usize).to_vec();
            let u = from_raw_parts(datas[1], (linesizes[1] * height / 2) as usize).to_vec();
            let v = from_raw_parts(datas[2], (linesizes[2] * height / 2) as usize).to_vec();

            frame.data.push(y);
            frame.data.push(u);
            frame.data.push(v);

            frame.linesize.push(linesizes[0]);
            frame.linesize.push(linesizes[1]);
            frame.linesize.push(linesizes[2]);

            frames.push(frame);
        } else if pixfmt == AVPixelFormat::AV_PIX_FMT_NV12 as c_int {
            let y = from_raw_parts(datas[0], (linesizes[0] * height) as usize).to_vec();
            let uv = from_raw_parts(datas[1], (linesizes[1] * height / 2) as usize).to_vec();

            frame.data.push(y);
            frame.data.push(uv);

            frame.linesize.push(linesizes[0]);
            frame.linesize.push(linesizes[1]);

            frames.push(frame);
        } else {
            error!("unsupported pixfmt {}", pixfmt as i32);
        }
    }

    pub fn available_decoders() -> Vec<CodecInfo> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        use log::debug;

        #[allow(unused_mut)]
        let mut codecs: Vec<CodecInfo> = vec![];
        // Generic CUDA acceleration uses the native FFmpeg decoders on Linux.
        #[cfg(target_os = "linux")]
        {
            let (nv, _, _) = crate::common::supported_gpu(false);
            debug!("Linux GPU support detected - NV: {}", nv);
            if nv {
                codecs.push(CodecInfo {
                    name: "h264".to_owned(),
                    format: H264,
                    hwdevice: AV_HWDEVICE_TYPE_CUDA,
                    priority: Priority::Good as _,
                    ..Default::default()
                });
                codecs.push(CodecInfo {
                    name: "hevc".to_owned(),
                    format: H265,
                    hwdevice: AV_HWDEVICE_TYPE_CUDA,
                    priority: Priority::Good as _,
                    ..Default::default()
                });
                codecs.push(CodecInfo {
                    name: "av1".to_owned(),
                    format: AV1,
                    hwdevice: AV_HWDEVICE_TYPE_CUDA,
                    priority: Priority::Good as _,
                    ..Default::default()
                });
            }
        }

        #[cfg(target_os = "windows")]
        {
            codecs.append(&mut vec![
                CodecInfo {
                    name: "h264".to_owned(),
                    format: H264,
                    hwdevice: AV_HWDEVICE_TYPE_D3D11VA,
                    priority: Priority::Best as _,
                    ..Default::default()
                },
                CodecInfo {
                    name: "hevc".to_owned(),
                    format: H265,
                    hwdevice: AV_HWDEVICE_TYPE_D3D11VA,
                    priority: Priority::Best as _,
                    ..Default::default()
                },
                CodecInfo {
                    name: "av1".to_owned(),
                    format: AV1,
                    hwdevice: AV_HWDEVICE_TYPE_D3D11VA,
                    priority: Priority::Best as _,
                    ..Default::default()
                },
            ]);
            // Dedicated CUVID decoders also work with FFmpeg distributions that
            // omit the native h264/hevc decoders used by D3D11VA above.
            for (name, format) in [
                ("h264_cuvid", H264),
                ("hevc_cuvid", H265),
                ("av1_cuvid", AV1),
            ] {
                codecs.push(CodecInfo {
                    name: name.to_owned(),
                    format,
                    hwdevice: AV_HWDEVICE_TYPE_CUDA,
                    priority: Priority::Good as _,
                    ..Default::default()
                });
            }
        }

        #[cfg(target_os = "linux")]
        {
            codecs.append(&mut vec![
                CodecInfo {
                    name: "h264".to_owned(),
                    format: H264,
                    hwdevice: AV_HWDEVICE_TYPE_VAAPI,
                    priority: Priority::Good as _,
                    ..Default::default()
                },
                CodecInfo {
                    name: "hevc".to_owned(),
                    format: H265,
                    hwdevice: AV_HWDEVICE_TYPE_VAAPI,
                    priority: Priority::Good as _,
                    ..Default::default()
                },
                CodecInfo {
                    name: "av1".to_owned(),
                    format: AV1,
                    hwdevice: AV_HWDEVICE_TYPE_VAAPI,
                    priority: Priority::Good as _,
                    ..Default::default()
                },
            ]);
        }

        #[cfg(target_os = "macos")]
        {
            let (_, _, h264, h265) = crate::common::get_video_toolbox_codec_support();
            debug!(
                "VideoToolbox decode support - H264: {}, H265: {}",
                h264, h265
            );
            if h264 {
                codecs.push(CodecInfo {
                    name: "h264".to_owned(),
                    format: H264,
                    hwdevice: AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
                    priority: Priority::Best as _,
                    ..Default::default()
                });
            }
            if h265 {
                codecs.push(CodecInfo {
                    name: "hevc".to_owned(),
                    format: H265,
                    hwdevice: AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
                    priority: Priority::Best as _,
                    ..Default::default()
                });
            }
            codecs.push(CodecInfo {
                name: "av1".to_owned(),
                format: AV1,
                hwdevice: AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
                priority: Priority::Best as _,
                ..Default::default()
            });
        }
        #[cfg(target_os = "ios")]
        {
            codecs.push(CodecInfo {
                name: "h264".to_owned(),
                format: H264,
                hwdevice: AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
                priority: Priority::Best as _,
                ..Default::default()
            });
            codecs.push(CodecInfo {
                name: "hevc".to_owned(),
                format: H265,
                hwdevice: AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
                priority: Priority::Best as _,
                ..Default::default()
            });
            codecs.push(CodecInfo {
                name: "av1".to_owned(),
                format: AV1,
                hwdevice: AV_HWDEVICE_TYPE_VIDEOTOOLBOX,
                priority: Priority::Best as _,
                ..Default::default()
            });
        }

        // Software fallbacks must pass the same real-frame test as hardware.
        let soft = CodecInfo::soft();
        codecs.extend(soft.h264);
        codecs.extend(soft.h265);
        Self::probe_decoders(codecs)
    }

    fn probe_decoders(codecs: Vec<CodecInfo>) -> Vec<CodecInfo> {
        use log::debug;
        let buf264 = &crate::common::DATA_H264_720P[..];
        let buf265 = &crate::common::DATA_H265_720P[..];
        let bufav1 = &crate::common::DATA_AV1_720P[..];

        Self::probe_decoders_with(codecs, |codec| {
            debug!(
                "Testing decoder: {} (hwdevice: {:?})",
                codec.name, codec.hwdevice
            );

            let c = DecodeContext {
                name: codec.name.clone(),
                device_type: codec.hwdevice,
                thread_count: 4,
            };

            match Decoder::new(c) {
                Ok(mut decoder) => {
                    debug!("Decoder {} created successfully", codec.name);
                    let data = match codec.format {
                        H264 => buf264,
                        H265 => buf265,
                        AV1 => bufav1,
                        _ => {
                            log::error!("Unsupported format: {:?}, skipping", codec.format);
                            return false;
                        }
                    };

                    let start = Instant::now();

                    match decoder.decode(data) {
                        Ok(frames) if !frames.is_empty() => {
                            let elapsed = start.elapsed().as_millis();

                            if elapsed < TEST_TIMEOUT_MS as _ {
                                debug!("Decoder {} test passed", codec.name);
                                return true;
                            } else {
                                debug!(
                                    "Decoder {} test failed - timeout: {}ms",
                                    codec.name, elapsed
                                );
                            }
                        }
                        Ok(_) => {
                            debug!("Decoder {} produced no usable frame", codec.name);
                        }
                        Err(err) => {
                            debug!("Decoder {} test failed with error: {}", codec.name, err);
                        }
                    }
                }
                Err(_) => {
                    debug!("Failed to create decoder {}", codec.name);
                }
            }
            false
        })
    }

    fn probe_decoders_with(
        codecs: Vec<CodecInfo>,
        mut probe: impl FnMut(&CodecInfo) -> bool,
    ) -> Vec<CodecInfo> {
        let mut res = Vec::<CodecInfo>::with_capacity(codecs.len());
        for codec in codecs {
            // Keep independently validated backends for stream-local fallback.
            if res
                .iter()
                .any(|existing| existing.name == codec.name && existing.hwdevice == codec.hwdevice)
            {
                continue;
            }
            if probe(&codec) {
                res.push(codec);
            }
        }
        res
    }

    pub fn available_software_decoders() -> Vec<CodecInfo> {
        // These facts belong to the loaded FFmpeg library and are independent
        // of GPU state and the user's hardware-acceleration preference.
        static SOFTWARE_DECODERS: std::sync::OnceLock<Vec<CodecInfo>> = std::sync::OnceLock::new();
        SOFTWARE_DECODERS
            .get_or_init(|| {
                let soft = CodecInfo::soft();
                let mut candidates = Vec::with_capacity(2);
                candidates.extend(soft.h264);
                candidates.extend(soft.h265);
                Self::probe_decoders(candidates)
            })
            .clone()
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            ffmpeg_ram_free_decoder(self.codec);
            self.codec = std::ptr::null_mut();
            let _ = Box::from_raw(self.frames);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_probe_keeps_distinct_backends_for_the_same_format() {
        let hardware = CodecInfo {
            name: "h264".into(),
            format: H264,
            hwdevice: AV_HWDEVICE_TYPE_D3D11VA,
            ..Default::default()
        };
        let alternative = CodecInfo {
            name: "h264_cuvid".into(),
            hwdevice: AV_HWDEVICE_TYPE_CUDA,
            ..hardware.clone()
        };
        let software = CodecInfo::soft().h264.unwrap();
        let candidates = vec![hardware.clone(), alternative, software, hardware];
        let mut probes = 0;
        let result = Decoder::probe_decoders_with(candidates, |_| {
            probes += 1;
            true
        });
        assert_eq!(probes, 3);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].hwdevice, AV_HWDEVICE_TYPE_D3D11VA);
        assert_eq!(result[1].name, "h264_cuvid");
        assert_eq!(result[2].hwdevice, AV_HWDEVICE_TYPE_NONE);
    }

    #[test]
    fn failed_probe_does_not_remove_other_formats_or_backends() {
        let candidates = vec![
            CodecInfo {
                name: "h264".into(),
                format: H264,
                ..Default::default()
            },
            CodecInfo {
                name: "h264_cuvid".into(),
                format: H264,
                hwdevice: AV_HWDEVICE_TYPE_CUDA,
                ..Default::default()
            },
            CodecInfo {
                name: "hevc_cuvid".into(),
                format: H265,
                hwdevice: AV_HWDEVICE_TYPE_CUDA,
                ..Default::default()
            },
        ];
        let result = Decoder::probe_decoders_with(candidates, |info| info.name != "h264");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].format, H264);
        assert_eq!(result[1].format, H265);
    }

    #[test]
    fn unavailable_decoder_is_not_advertised() {
        let result = Decoder::probe_decoders(vec![CodecInfo {
            name: "rustadmin_missing_decoder".to_owned(),
            format: H264,
            hwdevice: AV_HWDEVICE_TYPE_NONE,
            ..Default::default()
        }]);
        assert!(result.is_empty());
    }

    #[test]
    #[ignore = "requires Windows NVIDIA CUVID H264 and HEVC decoding"]
    #[cfg(windows)]
    fn windows_cuvid_decode_smoke() {
        let _ = env_logger::builder().is_test(true).try_init();
        for (name, data) in [
            ("h264_cuvid", crate::common::DATA_H264_720P),
            ("hevc_cuvid", crate::common::DATA_H265_720P),
        ] {
            // Exercise repeated decoder creation and GPU-to-RAM delivery.
            for _ in 0..8 {
                let mut decoder = Decoder::new(DecodeContext {
                    name: name.to_owned(),
                    device_type: AV_HWDEVICE_TYPE_CUDA,
                    thread_count: 1,
                })
                .expect("CUVID decoder must open");
                for _ in 0..4 {
                    let frames = decoder.decode(data).expect("CUVID must decode the sample");
                    assert!(!frames.is_empty(), "{name} returned no usable frame");
                    for frame in frames {
                        assert_eq!((frame.width, frame.height), (1280, 720));
                        assert!(!frame.data.is_empty());
                    }
                }
            }
        }
        let codecs = Decoder::available_decoders();
        for format in [H264, H265] {
            assert!(codecs
                .iter()
                .any(|c| c.format == format && c.hwdevice != AV_HWDEVICE_TYPE_NONE));
        }
    }

    #[test]
    #[ignore = "requires Windows NVIDIA NVENC and CUVID H264/HEVC"]
    #[cfg(windows)]
    fn windows_cuvid_reference_stream_smoke() {
        use crate::common::{Quality::Quality_Default, RateControl::RC_CBR};
        use crate::ffmpeg_ram::encode::{EncodeContext, Encoder};
        for (encoder_name, decoder_name) in
            [("h264_nvenc", "h264_cuvid"), ("hevc_nvenc", "hevc_cuvid")]
        {
            let mut encoder = Encoder::new(EncodeContext {
                name: encoder_name.to_owned(),
                mc_name: None,
                width: 640,
                height: 360,
                pixfmt: AVPixelFormat::AV_PIX_FMT_NV12,
                align: 64,
                fps: 30,
                gop: 120,
                rc: RC_CBR,
                quality: Quality_Default,
                kbs: 1000,
                q: -1,
                thread_count: 1,
            })
            .expect("NVENC must open");
            let mut decoder = Decoder::new(DecodeContext {
                name: decoder_name.to_owned(),
                device_type: AV_HWDEVICE_TYPE_CUDA,
                thread_count: 1,
            })
            .expect("CUVID must open");
            let mut input = vec![128; encoder.length as usize];
            let stride = encoder.linesize[0] as usize;
            let mut saw_delta = false;
            for index in 0..12 {
                let luma = 32 + index as u8 * 10;
                for row in 0..360 {
                    input[row * stride..row * stride + 640].fill(luma);
                }
                let packets = encoder
                    .encode(&input, index * 33, false)
                    .expect("NVENC must encode");
                assert!(!packets.is_empty());
                for packet in packets {
                    saw_delta |= packet.key == 0;
                    let frames = decoder
                        .decode(&packet.data)
                        .expect("CUVID must decode each reference-dependent packet");
                    assert_eq!(frames.len(), 1, "each packet must deliver immediately");
                    assert_eq!((frames[0].width, frames[0].height), (640, 360));
                    assert!(
                        (i16::from(frames[0].data[0][0]) - i16::from(luma)).abs() <= 8,
                        "decoder must return the current image, not an older buffered frame"
                    );
                }
            }
            assert!(saw_delta, "the test must exercise dependent frames");
        }
    }
}
