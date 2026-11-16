pub mod client;
pub mod protocol;

pub use client::{MediaError, MediaSandbox, RejectReason, configure};
pub use protocol::{Job, JobError, Meta, ProbeInfo, SelftestOp, VideoStreamInfo};

use anyhow::{Error, Result, bail};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use protocol::{LABEL_FRAME_SIZE, MAX_VIDEO_FRAMES};

pub struct CompressedImage {
    pub path: PathBuf,
    pub mime_type: String,
    pub width: usize,
    pub height: usize,
    pub duration: f32,
    pub bitrate: u32,
}

pub struct RgbImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

fn media_err(e: MediaError) -> Error {
    Error::msg(e.to_string())
}

fn unexpected(meta: Meta) -> Error {
    Error::msg(format!("unexpected worker reply: {:?}", meta))
}

pub fn probe_file(path: &Path) -> Result<ProbeInfo> {
    match MediaSandbox::from_config()
        .run(path, Job::Probe)
        .map_err(media_err)?
    {
        (Meta::Probe(info), _) => Ok(info),
        (other, _) => Err(unexpected(other)),
    }
}

pub fn compress_file(path: &Path, mime_type: &str, out_dir: &Path) -> Result<CompressedImage> {
    if !crate::can_compress(mime_type) {
        bail!("MIME type not supported");
    }
    if mime_type == "image/webp" {
        bail!("MIME type is already image/webp");
    }
    let (meta, bytes) = MediaSandbox::from_config()
        .run(
            path,
            Job::Compress {
                mime_type: mime_type.to_string(),
            },
        )
        .map_err(media_err)?;
    let Meta::Compressed {
        width,
        height,
        duration,
        bitrate,
    } = meta
    else {
        return Err(unexpected(meta));
    };
    let mut out = out_dir.join(Uuid::new_v4().to_string());
    out.set_extension("webp");
    std::fs::write(&out, bytes)?;
    Ok(CompressedImage {
        path: out,
        mime_type: "image/webp".to_string(),
        width: width as usize,
        height: height as usize,
        duration,
        bitrate,
    })
}

pub fn thumbnail_file(input: &Path, out_path: &Path) -> Result<()> {
    let (_, bytes) = MediaSandbox::from_config()
        .run(input, Job::Thumbnail)
        .map_err(media_err)?;
    std::fs::write(out_path, bytes)?;
    Ok(())
}

pub fn decode_rgb(path: &Path) -> Result<RgbImage> {
    match MediaSandbox::from_config()
        .run(path, Job::Rgb)
        .map_err(media_err)?
    {
        (Meta::Image { width, height }, pixels) => {
            if pixels.len() as u64 != width as u64 * height as u64 * 3 {
                bail!("pixel buffer size mismatch");
            }
            Ok(RgbImage {
                width,
                height,
                pixels,
            })
        }
        (other, _) => Err(unexpected(other)),
    }
}

pub fn label_image_rgb(path: &Path) -> Result<Vec<u8>, MediaError> {
    let size = LABEL_FRAME_SIZE;
    let (meta, pixels) = MediaSandbox::from_config().run(
        path,
        Job::LabelImage {
            width: size,
            height: size,
        },
    )?;
    match meta {
        Meta::Image { width, height }
            if width == size
                && height == size
                && pixels.len() == (size * size * 3) as usize =>
        {
            Ok(pixels)
        }
        other => Err(MediaError::Job(JobError::Failed {
            message: format!("unexpected worker reply: {:?}", other),
        })),
    }
}

pub fn label_video_frames(path: &Path) -> Result<Vec<Vec<u8>>, MediaError> {
    let size = LABEL_FRAME_SIZE;
    let frame_len = (size * size * 3) as usize;
    let (meta, pixels) = MediaSandbox::from_config().run(
        path,
        Job::LabelVideo {
            width: size,
            height: size,
        },
    )?;
    match meta {
        Meta::Frames {
            count,
            width,
            height,
        } if width == size
            && height == size
            && count as usize <= MAX_VIDEO_FRAMES
            && pixels.len() == frame_len * count as usize =>
        {
            Ok(pixels.chunks_exact(frame_len).map(|c| c.to_vec()).collect())
        }
        other => Err(MediaError::Job(JobError::Failed {
            message: format!("unexpected worker reply: {:?}", other),
        })),
    }
}

pub fn llm_jpeg(path: &Path, max_pixels: u64) -> Result<Vec<u8>, MediaError> {
    MediaSandbox::from_config()
        .run(path, Job::LlmJpeg { max_pixels })
        .map(|(_, bytes)| bytes)
}
