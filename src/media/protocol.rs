use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{self, Read, Write};

pub const MAX_FRAME_BYTES: u32 = 1 << 20;
pub const MAX_VIDEO_FRAMES: usize = 10;
pub const MIN_LABEL_DIMENSION: u32 = 48;
pub const LABEL_FRAME_SIZE: u32 = 224;
pub const THUMBNAIL_WIDTH: u16 = 512;
pub const LLM_MAX_HEIGHT: u32 = 1024;
pub const DEFAULT_LLM_MAX_PIXELS: u64 = 1_800_000;
pub const INPUT_FD: i32 = 3;

pub fn llm_target_size(width: u32, height: u32, max_pixels: u64) -> (u32, u32) {
    let mut scale = 1.0f64;
    if height > LLM_MAX_HEIGHT {
        scale = LLM_MAX_HEIGHT as f64 / height as f64;
    }
    let scaled_pixels = width as f64 * height as f64 * scale * scale;
    if scaled_pixels > max_pixels as f64 {
        scale *= (max_pixels as f64 / scaled_pixels).sqrt();
    }
    if scale >= 1.0 {
        return (width, height);
    }
    let even = |v: f64| (((v as u32).max(2)) / 2) * 2;
    (even(width as f64 * scale), even(height as f64 * scale))
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SelftestOp {
    OpenPasswd,
    Socket,
    Exec,
    Fork,
    MmapExec,
    MprotectExec,
    Ptrace,
    Mount,
    Unshare,
    ReadInput,
    ListEnv,
    Spin,
    Segv,
    AllocHuge,
    SandboxState,
    Garbage,
    Thread,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "job", rename_all = "snake_case")]
pub enum Job {
    Probe,
    Compress { mime_type: String },
    Thumbnail,
    Rgb,
    LabelImage { width: u32, height: u32 },
    LabelVideo { width: u32, height: u32 },
    LlmJpeg { max_pixels: u64 },
    Selftest { op: SelftestOp },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Request {
    pub job: Job,
    pub max_output_bytes: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct VideoStreamInfo {
    pub width: u32,
    pub height: u32,
    pub codec: i64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ProbeInfo {
    pub format: String,
    pub duration: f32,
    pub bitrate: u64,
    pub stream_count: u32,
    pub best_video: Option<VideoStreamInfo>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Meta {
    Empty,
    Probe(ProbeInfo),
    Compressed {
        width: u32,
        height: u32,
        duration: f32,
        bitrate: u32,
    },
    Image {
        width: u32,
        height: u32,
    },
    Frames {
        count: u32,
        width: u32,
        height: u32,
    },
    Selftest {
        value: i64,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobError {
    Failed { message: String },
    NoMediaStream,
    NoImageData,
    TooSmall { width: u32, height: u32 },
    Corrupt { detail: String },
}

impl fmt::Display for JobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed { message } => f.write_str(message),
            Self::NoMediaStream => f.write_str("No video/image stream found"),
            Self::NoImageData => f.write_str("No image data found"),
            Self::TooSmall { width, height } => write!(f, "Media too small: {}x{}", width, height),
            Self::Corrupt { detail } => write!(f, "Corrupt media: {}", detail),
        }
    }
}

impl std::error::Error for JobError {}

impl From<anyhow::Error> for JobError {
    fn from(e: anyhow::Error) -> Self {
        Self::Failed {
            message: e.to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ok { meta: Meta, payload_len: u64 },
    Err { error: JobError },
}

pub fn write_frame<T: Serialize>(w: &mut impl Write, value: &T) -> io::Result<()> {
    let body = serde_json::to_vec(value).map_err(io::Error::other)?;
    let len = u32::try_from(body.len())
        .ok()
        .filter(|l| *l <= MAX_FRAME_BYTES)
        .ok_or_else(|| io::Error::other("frame too large"))?;
    w.write_all(&len.to_be_bytes())?;
    w.write_all(&body)
}

pub fn read_frame<T: for<'de> Deserialize<'de>>(r: &mut impl Read) -> io::Result<T> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len);
    if len > MAX_FRAME_BYTES {
        return Err(io::Error::other("frame too large"));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_round_trips() {
        let req = Request {
            job: Job::LabelImage {
                width: 224,
                height: 224,
            },
            max_output_bytes: 10,
        };
        let mut buf = Vec::new();
        write_frame(&mut buf, &req).unwrap();
        let back: Request = read_frame(&mut buf.as_slice()).unwrap();
        assert_eq!(back.job, req.job);
    }

    #[test]
    fn oversized_frame_is_rejected() {
        let mut buf = (MAX_FRAME_BYTES + 1).to_be_bytes().to_vec();
        buf.extend_from_slice(b"{}");
        assert!(read_frame::<Request>(&mut buf.as_slice()).is_err());
    }

    #[test]
    fn truncated_frame_is_rejected() {
        let mut buf = 50u32.to_be_bytes().to_vec();
        buf.extend_from_slice(b"{}");
        assert!(read_frame::<Request>(&mut buf.as_slice()).is_err());
    }
}
