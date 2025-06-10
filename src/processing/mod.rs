use anyhow::{bail, Error, Result};
use ffmpeg_rs_raw::ffmpeg_sys_the_third::AVPixelFormat::AV_PIX_FMT_YUV420P;
use ffmpeg_rs_raw::ffmpeg_sys_the_third::{av_frame_free, av_packet_free, AVFrame};
use ffmpeg_rs_raw::{Decoder, Demuxer, DemuxerInfo, Encoder, Scaler, StreamType, Transcoder};
use std::path::{Path, PathBuf};
use std::ptr;
use uuid::Uuid;

#[cfg(feature = "labels")]
pub mod labeling;

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

    pub fn compress(
        &mut self,
        input: &Path,
        mime_type: &str,
        out_dir: &Path,
    ) -> Result<NewFileProcessorResult> {
        use ffmpeg_rs_raw::ffmpeg_sys_the_third::AVCodecID::AV_CODEC_ID_WEBP;

        if !mime_type.starts_with("image/") {
            bail!("MIME type not supported");
        }

        if mime_type == "image/webp" {
            bail!("MIME type is already image/webp");
        }

        let uid = Uuid::new_v4();
        let mut out_path = out_dir.join(uid.to_string());
        out_path.set_extension("webp");

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
            trans.run(None)?;

            Ok(NewFileProcessorResult {
                result: out_path,
                mime_type: "image/webp".to_string(),
                width: image_stream.width,
                height: image_stream.height,
                duration: if probe.duration < 0. {
                    0.
                } else {
                    probe.duration
                },
                bitrate: probe.bitrate as u32,
            })
        }
    }

    pub fn thumbnail(&mut self, input: &Path, out_path: &Path) -> Result<()> {
        use ffmpeg_rs_raw::ffmpeg_sys_the_third::AVCodecID::AV_CODEC_ID_WEBP;

        unsafe {
            let mut input = Demuxer::new(input.to_str().unwrap())?;

            let probe = input.probe_input()?;

            let image_stream = probe
                .streams
                .iter()
                .find(|c| c.stream_type == StreamType::Video)
                .ok_or(Error::msg("No image found, cant compress"))?;

            let w = 512u16;
            let scale = w as f32 / image_stream.width as f32;
            let h = (image_stream.height as f32 * scale) as u16;

            let enc = Encoder::new(AV_CODEC_ID_WEBP)?
                .with_height(h as i32)
                .with_width(w as i32)
                .with_pix_fmt(AV_PIX_FMT_YUV420P)
                .with_framerate(1.0)?
                .open(None)?;

            let mut sws = Scaler::new();
            let mut decoder = Decoder::new();
            decoder.setup_decoder(image_stream, None)?;

            while let Ok((mut pkt, _stream)) = input.get_packet() {
                let mut frame_save: *mut AVFrame = ptr::null_mut();
                for (mut frame, _stream) in decoder.decode_pkt(pkt)? {
                    if frame_save.is_null() {
                        frame_save = sws.process_frame(frame, w, h, AV_PIX_FMT_YUV420P)?;
                    }
                    av_frame_free(&mut frame);
                }

                av_packet_free(&mut pkt);
                if !frame_save.is_null() {
                    enc.save_picture(frame_save, out_path.to_str().unwrap())?;
                    av_frame_free(&mut frame_save);
                    return Ok(());
                }
            }

            Ok(())
        }
    }
}

pub struct NewFileProcessorResult {
    pub result: PathBuf,
    pub mime_type: String,
    pub width: usize,
    pub height: usize,
    pub duration: f32,
    pub bitrate: u32,
}

pub fn compress_file(
    path: &Path,
    mime_type: &str,
    out_dir: &Path,
) -> Result<NewFileProcessorResult, Error> {
    if !crate::can_compress(mime_type) {
        bail!("MIME type not supported");
    }

    if mime_type.starts_with("image/") {
        let mut proc = WebpProcessor::new();
        return proc.compress(path, mime_type, out_dir);
    }
    bail!("No media processor")
}

pub fn probe_file(path: &Path) -> Result<DemuxerInfo> {
    let mut demuxer = Demuxer::new(path.to_str().unwrap())?;
    unsafe { demuxer.probe_input() }
}
