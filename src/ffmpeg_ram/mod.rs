#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use crate::common::DataFormat::{self, *};
use crate::ffmpeg::{
    AVHWDeviceType::{self, *},
    AVPixelFormat,
};
use serde_derive::{Deserialize, Serialize};
use std::ffi::c_int;

include!(concat!(env!("OUT_DIR"), "/ffmpeg_ram_ffi.rs"));

pub mod decode;
pub mod encode;

pub enum Priority {
    Best = 0,
    Good = 1,
    Normal = 2,
    Soft = 3,
    Bad = 4,
}

#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
pub struct CodecInfo {
    pub name: String,
    #[serde(skip)]
    pub mc_name: Option<String>,
    pub format: DataFormat,
    pub priority: i32,
    pub hwdevice: AVHWDeviceType,
}

impl Default for CodecInfo {
    fn default() -> Self {
        Self {
            name: Default::default(),
            mc_name: Default::default(),
            format: DataFormat::H264,
            priority: Default::default(),
            hwdevice: AVHWDeviceType::AV_HWDEVICE_TYPE_NONE,
        }
    }
}

impl CodecInfo {
    /// Distributed H.26x encoders must be supplied by a reviewed hardware or
    /// platform backend. Keep this as a fail-closed allow-list: do not add
    /// libx264, libx265, libopenh264, or another CPU H.26x encoder. Software
    /// encoding fallback belongs to the AV1/VP9/VP8 paths in RustAdmin.
    pub fn is_hardware_encoder_name(name: &str) -> bool {
        [
            "_nvenc",
            "_amf",
            "_qsv",
            "_vaapi",
            "_videotoolbox",
            "_mediacodec",
            "_mf",
        ]
        .iter()
        .any(|suffix| name.ends_with(suffix))
    }

    pub fn is_software_encoder_name(name: &str) -> bool {
        !Self::is_hardware_encoder_name(name)
    }

    pub fn is_hardware_encoder(&self) -> bool {
        Self::is_hardware_encoder_name(&self.name)
    }

    pub fn is_software_encoder(&self) -> bool {
        Self::is_software_encoder_name(&self.name)
    }

    pub fn prioritized(coders: Vec<CodecInfo>) -> CodecInfos {
        let mut h264: Option<CodecInfo> = None;
        let mut h265: Option<CodecInfo> = None;
        let mut vp8: Option<CodecInfo> = None;
        let mut vp9: Option<CodecInfo> = None;
        let mut av1: Option<CodecInfo> = None;

        for coder in coders {
            match coder.format {
                DataFormat::H264 => match &h264 {
                    Some(old) => {
                        if old.priority > coder.priority {
                            h264 = Some(coder)
                        }
                    }
                    None => h264 = Some(coder),
                },
                DataFormat::H265 => match &h265 {
                    Some(old) => {
                        if old.priority > coder.priority {
                            h265 = Some(coder)
                        }
                    }
                    None => h265 = Some(coder),
                },
                DataFormat::VP8 => match &vp8 {
                    Some(old) => {
                        if old.priority > coder.priority {
                            vp8 = Some(coder)
                        }
                    }
                    None => vp8 = Some(coder),
                },
                DataFormat::VP9 => match &vp9 {
                    Some(old) => {
                        if old.priority > coder.priority {
                            vp9 = Some(coder)
                        }
                    }
                    None => vp9 = Some(coder),
                },
                DataFormat::AV1 => match &av1 {
                    Some(old) => {
                        if old.priority > coder.priority {
                            av1 = Some(coder)
                        }
                    }
                    None => av1 = Some(coder),
                },
            }
        }
        CodecInfos {
            h264,
            h265,
            vp8,
            vp9,
            av1,
        }
    }

    pub fn soft() -> CodecInfos {
        CodecInfos {
            h264: Some(CodecInfo {
                name: "h264".to_owned(),
                mc_name: Default::default(),
                format: H264,
                hwdevice: AV_HWDEVICE_TYPE_NONE,
                priority: Priority::Soft as _,
            }),
            h265: Some(CodecInfo {
                name: "hevc".to_owned(),
                mc_name: Default::default(),
                format: H265,
                hwdevice: AV_HWDEVICE_TYPE_NONE,
                priority: Priority::Soft as _,
            }),
            vp8: None,
            vp9: None,
            av1: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CodecInfo;

    #[test]
    fn hardware_encoder_allow_list_is_fail_closed() {
        for name in [
            "h264_nvenc",
            "hevc_amf",
            "av1_qsv",
            "h264_vaapi",
            "hevc_videotoolbox",
            "h264_mediacodec",
            "h264_mf",
        ] {
            assert!(CodecInfo::is_hardware_encoder_name(name), "{name}");
            assert!(!CodecInfo::is_software_encoder_name(name), "{name}");
            assert!(CodecInfo {
                name: name.to_owned(),
                ..Default::default()
            }
            .is_hardware_encoder());
        }

        assert!(CodecInfo::is_software_encoder_name("libx264"));
        assert!(CodecInfo::is_software_encoder_name("libx265"));
        assert!(CodecInfo::is_software_encoder_name("libopenh264"));
        assert!(CodecInfo::is_software_encoder_name("h264"));
        assert!(CodecInfo::is_software_encoder_name("hevc"));
        assert!(CodecInfo::is_software_encoder_name("future_h26x_backend"));
    }
}

#[derive(Debug, Eq, PartialEq, Clone, Serialize, Deserialize)]
pub struct CodecInfos {
    pub h264: Option<CodecInfo>,
    pub h265: Option<CodecInfo>,
    pub vp8: Option<CodecInfo>,
    pub vp9: Option<CodecInfo>,
    pub av1: Option<CodecInfo>,
}

impl CodecInfos {
    pub fn serialize(&self) -> Result<String, ()> {
        match serde_json::to_string_pretty(self) {
            Ok(s) => Ok(s),
            Err(_) => Err(()),
        }
    }

    pub fn deserialize(s: &str) -> Result<Self, ()> {
        match serde_json::from_str(s) {
            Ok(c) => Ok(c),
            Err(_) => Err(()),
        }
    }
}

pub fn ffmpeg_linesize_offset_length(
    pixfmt: AVPixelFormat,
    width: usize,
    height: usize,
    align: usize,
) -> Result<(Vec<i32>, Vec<i32>, i32), ()> {
    let mut linesize = Vec::<c_int>::new();
    linesize.resize(AV_NUM_DATA_POINTERS as _, 0);
    let mut offset = Vec::<c_int>::new();
    offset.resize(AV_NUM_DATA_POINTERS as _, 0);
    let mut length = Vec::<c_int>::new();
    length.resize(1, 0);
    unsafe {
        if ffmpeg_ram_get_linesize_offset_length(
            pixfmt as _,
            width as _,
            height as _,
            align as _,
            linesize.as_mut_ptr(),
            offset.as_mut_ptr(),
            length.as_mut_ptr(),
        ) == 0
        {
            return Ok((linesize, offset, length[0]));
        }
    }

    Err(())
}
