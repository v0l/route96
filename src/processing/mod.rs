use std::path::PathBuf;

use crate::processing::probe::FFProbe;
use anyhow::{bail, Error, Result};
use ffmpeg_rs_raw::ffmpeg_sys_the_third::AVPixelFormat::AV_PIX_FMT_YUV420P;
use ffmpeg_rs_raw::{Encoder, StreamType, Transcoder};

#[cfg(feature = "labels")]
pub mod labeling;
mod probe;

pub struct WebpProcessor;

impl Default for WebpProcessor {
    fn default() -> Self {
        Self::new()
    }
}

impl WebpProcessor {
    pub fn new() -> Self {
        Self
    }

    pub fn process_file(&mut self, input: PathBuf, mime_type: &str) -> Result<FileProcessorResult> {
        use ffmpeg_rs_raw::ffmpeg_sys_the_third::AVCodecID::AV_CODEC_ID_WEBP;

        if !mime_type.starts_with("image/") {
            bail!("MIME type not supported");
        }

        if mime_type == "image/webp" {
            return Ok(FileProcessorResult::Skip);
        }

        let mut out_path = input.clone();
        out_path.set_extension("compressed.webp");
        unsafe {
            let mut trans = Transcoder::new(input.to_str().unwrap(), out_path.to_str().unwrap())?;

            let probe = trans.prepare()?;
            let image_stream = probe
                .streams
                .iter()
                .find(|c| c.stream_type == StreamType::Video)
                .ok_or(Error::msg("No image found, cant compress"))?;

            let enc = Encoder::new(AV_CODEC_ID_WEBP)?
                .with_height(image_stream.height as i32)
                .with_width(image_stream.width as i32)
                .with_pix_fmt(AV_PIX_FMT_YUV420P)
                .open(None)?;

            trans.transcode_stream(image_stream, enc)?;
            trans.run()?;

            Ok(FileProcessorResult::NewFile(NewFileProcessorResult {
                result: out_path,
                mime_type: "image/webp".to_string(),
                width: image_stream.width,
                height: image_stream.height,
            }))
        }
    }
}

pub struct ProbeResult {
    pub streams: Vec<ProbeStream>,
}

pub enum ProbeStream {
    Video {
        width: u32,
        height: u32,
        codec: String,
    },
    Audio {
        sample_rate: u32,
        codec: String,
    },
}

pub enum FileProcessorResult {
    NewFile(NewFileProcessorResult),
    Skip,
}

pub struct NewFileProcessorResult {
    pub result: PathBuf,
    pub mime_type: String,
    pub width: usize,
    pub height: usize,
}

pub fn compress_file(in_file: PathBuf, mime_type: &str) -> Result<FileProcessorResult, Error> {
    let proc = if mime_type.starts_with("image/") {
        Some(WebpProcessor::new())
    } else {
        None
    };
    if let Some(mut proc) = proc {
        proc.process_file(in_file, mime_type)
    } else {
        Ok(FileProcessorResult::Skip)
    }
}

pub fn probe_file(in_file: PathBuf) -> Result<Option<(usize, usize)>> {
    let proc = FFProbe::new();
    let info = proc.process_file(in_file)?;
    Ok(info.best_video().map(|v| (v.width, v.height)))
}
