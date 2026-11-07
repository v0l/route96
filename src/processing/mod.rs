use anyhow::{Error, Result, bail};
use ffmpeg_rs_raw::ffmpeg_sys_the_third::AVPixelFormat::AV_PIX_FMT_YUV420P;
use ffmpeg_rs_raw::{Decoder, Demuxer, DemuxerInfo, Encoder, Scaler, StreamType, Transcoder};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[cfg(feature = "labels")]
pub mod labeling;

/// Demuxer format names (as reported by ffmpeg, comma-separated per
/// container) that take references to additional local or network resources
/// from inside the media itself: HLS playlists (file:// segments by default),
/// concat scripts, DASH manifests, RTSP/SDP descriptions. ffmpeg-rs-raw
/// currently passes no options dictionary to avformat_open_input, so a
/// protocol_whitelist cannot be enforced at open time - these formats are
/// rejected by name instead (audit finding 4.16).
const BLOCKED_DEMUXER_FORMATS: &[&str] = &["hls", "applehttp", "concat", "dash", "rtsp", "sdp"];

/// Cheap content sniff for HLS playlists: ffmpeg detects them from the
/// "#EXTM3U" header regardless of the declared MIME type, and the probe
/// itself opens the referenced segments before any post-probe format check
/// could run. Call this BEFORE demuxing untrusted media.
pub fn reject_playlist_media(path: &Path) -> Result<()> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut head = [0u8; 16];
    let n = file.read(&mut head).unwrap_or(0);
    if head[..n].starts_with(b"#EXTM3U") {
        bail!("playlist media is not allowed on this server");
    }
    Ok(())
}

/// Reject blocked demuxer formats after probing. `format` is ffmpeg's
/// comma-separated list for the container (e.g. "mov,mp4,m4a,3gp,3g2,mj2"),
/// so the comparison is token-based to avoid substring false positives.
pub fn check_demuxer_format(format: &str) -> Result<()> {
    let blocked = format.split(',').any(|token| {
        let token = token.trim().to_ascii_lowercase();
        BLOCKED_DEMUXER_FORMATS.contains(&token.as_str())
    });
    if blocked {
        bail!("media format '{}' is not allowed on this server", format);
    }
    Ok(())
}

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

        reject_playlist_media(input)?;

        unsafe {
            let mut trans = Transcoder::new(input.to_str().unwrap(), out_path.to_str().unwrap())?;

            let probe = trans.prepare()?;
            check_demuxer_format(&probe.format)?;
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

        reject_playlist_media(input)?;

        unsafe {
            let mut input = Demuxer::new(input.to_str().unwrap())?;

            let probe = input.probe_input()?;
            check_demuxer_format(&probe.format)?;

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

            while let Ok((pkt, _)) = input.get_packet() {
                // skip packets not in the image stream
                if let Some(pkt) = pkt.as_ref()
                    && pkt.stream_index != image_stream.index as i32
                {
                    continue;
                }
                if let Ok(results) = decoder.decode_pkt(pkt.as_ref())
                    && let Some((frame, _)) = results.into_iter().next() {
                        let frame_save = sws.process_frame(&frame, w, h, AV_PIX_FMT_YUV420P)?;
                        enc.save_picture(&frame_save, out_path.to_str().unwrap())?;
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
    reject_playlist_media(path)?;
    let mut demuxer = Demuxer::new(path.to_str().unwrap())?;
    let info = unsafe { demuxer.probe_input() }?;
    check_demuxer_format(&info.format)?;
    Ok(info)
}


#[cfg(test)]
mod format_gate_tests {
    use super::*;
    use std::io::Write;

    fn write_file(content: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("r96_gate_{}", Uuid::new_v4()));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content).unwrap();
        path
    }

    #[test]
    fn hls_playlist_content_is_rejected() {
        let path = write_file(b"#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXT-X-STREAM-INF:BANDWIDTH=128000\nfile:///etc/passwd\n");
        let err = reject_playlist_media(&path).unwrap_err();
        assert!(err.to_string().contains("playlist media"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn real_media_magic_passes_the_sniff() {
        // MP4 ftyp header
        let path = write_file(b"\x00\x00\x00\x18ftypmp42\x00\x00\x00\x00mp42isom");
        assert!(reject_playlist_media(&path).is_ok());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn blocked_demuxer_formats_are_rejected_by_token() {
        assert!(check_demuxer_format("hls,applehttp").is_err());
        assert!(check_demuxer_format("concat").is_err());
        assert!(check_demuxer_format("mov,mp4,m4a,3gp,3g2,mj2").is_ok());
        // token-boundary: no substring false positives across commas
        assert!(check_demuxer_format("webm,matroska").is_ok());
    }
}
