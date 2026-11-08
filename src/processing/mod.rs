use anyhow::{Error, Result, bail};
use ffmpeg_rs_raw::ffmpeg_sys_the_third::AVPixelFormat::AV_PIX_FMT_YUV420P;
use ffmpeg_rs_raw::{Decoder, Demuxer, DemuxerInfo, Encoder, Scaler, StreamType, Transcoder};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[cfg(feature = "labels")]
pub mod labeling;

pub const ALLOWED_INPUT_FORMATS: &str = "mov,mp4,m4a,3gp,3g2,mj2,matroska,webm,avi,mpegts,mpeg,flv,ogg,wav,mp3,gif,apng,image2,image2pipe,jpeg_pipe,png_pipe,webp_pipe,bmp_pipe,tiff_pipe,jpegxl_pipe,avif_pipe,heif";

pub fn input_open_options() -> HashMap<String, String> {
    HashMap::from([
        (
            "format_whitelist".to_string(),
            ALLOWED_INPUT_FORMATS.to_string(),
        ),
        ("protocol_whitelist".to_string(), "file".to_string()),
    ])
}

pub fn open_demuxer(path: &str) -> Result<Demuxer> {
    Ok(Demuxer::new(path)?.with_open_options(input_open_options()))
}

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
            let mut trans = Transcoder::new_with_open_options(
                input.to_str().unwrap(),
                out_path.to_str().unwrap(),
                input_open_options(),
            )?;

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
            let mut input = open_demuxer(input.to_str().unwrap())?;

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
                    && let Some((frame, _)) = results.into_iter().next()
                {
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
    let mut demuxer = open_demuxer(path.to_str().unwrap())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(ext: &str, body: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new().suffix(ext).tempfile().unwrap();
        f.write_all(body.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    fn media_dir_with_manifest(name: &str, manifest: &str) -> (tempfile::TempDir, String) {
        let dir = generate("secret.wav", &["-f", "lavfi", "-i", "sine=duration=1"])
            .expect("ffmpeg could not make secret.wav");
        let video = generate("secret.mp4", &with(VIDEO, &["-c:v", "mpeg4"]))
            .expect("ffmpeg could not make secret.mp4");
        std::fs::copy(
            video.path().join("secret.mp4"),
            dir.path().join("secret.mp4"),
        )
        .unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, manifest).unwrap();
        (dir, path.to_str().unwrap().to_string())
    }

    fn opens_path(path: &str, options: HashMap<String, String>) -> bool {
        let mut d = Demuxer::new(path).unwrap().with_open_options(options);
        unsafe { d.probe_input() }.is_ok()
    }

    const DASH: &str = "<?xml version=\"1.0\"?><MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" mediaPresentationDuration=\"PT1S\" minBufferTime=\"PT1S\" profiles=\"urn:mpeg:dash:profile:isoff-on-demand:2011\"><Period><AdaptationSet mimeType=\"video/mp4\"><Representation id=\"1\" bandwidth=\"1000\"><SegmentList duration=\"1\"><SegmentURL media=\"secret.mp4\"/></SegmentList></Representation></AdaptationSet></Period></MPD>";
    const CONCAT: &str = "ffconcat version 1.0\nfile secret.wav\n";
    const HLS: &str = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\nsecret.wav\n#EXT-X-ENDLIST\n";

    fn assert_blocked(manifest: &str) {
        let (_dir, path) = media_dir_with_manifest("upload.mp4", manifest);
        assert!(!opens_path(&path, input_open_options()));
    }

    #[test]
    fn dash_manifest_fails_to_open() {
        assert_blocked(DASH);
    }

    #[test]
    fn concat_script_fails_to_open() {
        assert_blocked(CONCAT);
    }

    #[test]
    fn hls_playlist_fails_to_open() {
        assert_blocked(HLS);
    }

    #[test]
    fn dash_manifest_opens_without_whitelist() {
        let (_dir, path) = media_dir_with_manifest("upload.mp4", DASH);
        assert!(opens_path(&path, HashMap::new()));
    }

    #[test]
    fn concat_script_opens_without_whitelist() {
        let (_dir, path) = media_dir_with_manifest("upload.mp4", CONCAT);
        assert!(opens_path(&path, HashMap::new()));
    }

    #[test]
    fn transcoder_rejects_concat_script() {
        let f = write_temp(".mp4", "ffconcat version 1.0\nfile /etc/hostname\n");
        let out = tempfile::tempdir().unwrap();
        let r = WebpProcessor::new().compress(f.path(), "video/mp4", out.path());
        assert!(r.is_err());
    }

    fn generate(name: &str, args: &[&str]) -> Option<tempfile::TempDir> {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join(name);
        let status = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y"])
            .args(args)
            .arg(&out)
            .status()
            .ok()?;
        status.success().then_some(dir)
    }

    fn assert_legit_opens(name: &str, args: &[&str]) {
        let dir = generate(name, args).unwrap_or_else(|| panic!("ffmpeg could not make {name}"));
        let path = dir.path().join(name);
        let mut d = open_demuxer(path.to_str().unwrap()).unwrap();
        let info = unsafe { d.probe_input() }.unwrap_or_else(|e| panic!("{name} failed: {e}"));
        assert!(!info.streams.is_empty(), "{name} has no streams");
    }

    const VIDEO: &[&str] = &["-f", "lavfi", "-i", "testsrc=size=64x64:rate=5:duration=1"];
    const AUDIO: &[&str] = &["-f", "lavfi", "-i", "sine=duration=1"];
    const IMAGE: &[&str] = &["-f", "lavfi", "-i", "testsrc=size=64x64", "-frames:v", "1"];

    fn with(base: &[&'static str], extra: &[&'static str]) -> Vec<&'static str> {
        base.iter().chain(extra.iter()).copied().collect()
    }

    #[test]
    fn legit_mp4_opens() {
        assert_legit_opens("a.mp4", &with(VIDEO, &["-c:v", "mpeg4"]));
    }

    #[test]
    fn legit_webm_opens() {
        assert_legit_opens("a.webm", &with(VIDEO, &["-c:v", "libvpx"]));
    }

    #[test]
    fn legit_mkv_opens() {
        assert_legit_opens("a.mkv", &with(VIDEO, &["-c:v", "mpeg4"]));
    }

    #[test]
    fn legit_mp3_opens() {
        assert_legit_opens("a.mp3", &with(AUDIO, &["-c:a", "libmp3lame"]));
    }

    #[test]
    fn legit_ogg_opens() {
        assert_legit_opens("a.ogg", &with(AUDIO, &["-c:a", "libvorbis"]));
    }

    #[test]
    fn legit_wav_opens() {
        assert_legit_opens("a.wav", AUDIO);
    }

    #[test]
    fn legit_gif_opens() {
        assert_legit_opens("a.gif", VIDEO);
    }

    #[test]
    fn legit_png_opens() {
        assert_legit_opens("a.png", IMAGE);
    }

    #[test]
    fn legit_jpeg_opens() {
        assert_legit_opens("a.jpg", IMAGE);
    }

    #[test]
    fn legit_webp_opens() {
        assert_legit_opens("a.webp", IMAGE);
    }

    #[test]
    fn legit_animated_webp_opens() {
        assert_legit_opens("a.webp", &with(VIDEO, &["-loop", "0"]));
    }
}
