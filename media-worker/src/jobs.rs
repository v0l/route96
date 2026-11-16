use anyhow::{Error, Result, bail};
use ffmpeg_rs_raw::ffmpeg_sys_the_third::AVCodecID::{AV_CODEC_ID_MJPEG, AV_CODEC_ID_WEBP};
use ffmpeg_rs_raw::ffmpeg_sys_the_third::AVPixelFormat::{
    AV_PIX_FMT_RGB24, AV_PIX_FMT_YUV420P, AV_PIX_FMT_YUVJ420P,
};
use ffmpeg_rs_raw::ffmpeg_sys_the_third::{AV_NOPTS_VALUE, AVSEEK_FLAG_BACKWARD, av_seek_frame};
use ffmpeg_rs_raw::{
    AvFrameRef, Decoder, Demuxer, DemuxerInfo, Encoder, Muxer, Scaler, StreamInfo, StreamType,
    Transcoder,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Cursor, Seek, SeekFrom, Write};
use std::rc::Rc;
use std::slice;

use crate::protocol::{
    INPUT_FD, Job, JobError, MAX_VIDEO_FRAMES, Meta, MIN_LABEL_DIMENSION,
    ProbeInfo, THUMBNAIL_WIDTH, VideoStreamInfo, llm_target_size,
};

const ALLOWED_INPUT_FORMATS: &str = "mov,mp4,m4a,3gp,3g2,mj2,matroska,webm,avi,mpegts,mpeg,flv,ogg,wav,mp3,gif,apng,image2,image2pipe,jpeg_pipe,png_pipe,webp_pipe,bmp_pipe,tiff_pipe,jpegxl_pipe,avif_pipe,heif";
const BLOCKED_DEMUXER_FORMATS: &[&str] = &["hls", "applehttp", "concat", "dash", "rtsp", "sdp"];

type JobResult = Result<(Meta, Vec<u8>), JobError>;

pub fn run(job: Job) -> JobResult {
    match job {
        Job::Probe => probe(),
        Job::Compress { mime_type } => compress(&mime_type),
        Job::Thumbnail => thumbnail(),
        Job::Rgb => rgb(),
        Job::LabelImage { width, height } => label_image(width, height),
        Job::LabelVideo { width, height } => label_video(width, height),
        Job::LlmJpeg { max_pixels } => llm_jpeg(max_pixels),
        Job::Selftest { op } => crate::selftest::run(op),
    }
}

fn input_url() -> String {
    "fd:".to_string()
}

fn open_demuxer() -> Result<Demuxer> {
    let options = HashMap::from([
        (
            "format_whitelist".to_string(),
            ALLOWED_INPUT_FORMATS.to_string(),
        ),
        ("protocol_whitelist".to_string(), "fd".to_string()),
        ("fd".to_string(), INPUT_FD.to_string()),
    ]);
    Ok(Demuxer::new(&input_url())?.with_open_options(options))
}

fn check_demuxer_format(format: &str) -> Result<()> {
    let blocked = format.split(',').any(|token| {
        let token = token.trim().to_ascii_lowercase();
        BLOCKED_DEMUXER_FORMATS.contains(&token.as_str())
    });
    if blocked {
        bail!("media format '{}' is not allowed on this server", format);
    }
    Ok(())
}

fn probe_demuxer(demux: &mut Demuxer) -> Result<DemuxerInfo> {
    let info = unsafe { demux.probe_input() }?;
    check_demuxer_format(&info.format)?;
    Ok(info)
}

fn check_dimensions(width: u32, height: u32) -> Result<(), JobError> {
    if width < MIN_LABEL_DIMENSION || height < MIN_LABEL_DIMENSION {
        return Err(JobError::TooSmall { width, height });
    }
    Ok(())
}

fn video_sample_times(duration: f64, count: usize) -> Vec<f64> {
    if duration <= 0.0 || count <= 1 {
        return vec![0.0];
    }
    (0..count)
        .map(|i| (i as f64 + 0.5) / count as f64 * duration)
        .collect()
}

fn probe() -> JobResult {
    let mut demux = open_demuxer()?;
    let info = probe_demuxer(&mut demux)?;
    let best_video = info.best_video().map(|v: &StreamInfo| VideoStreamInfo {
        width: v.width as u32,
        height: v.height as u32,
        codec: v.codec as i64,
    });
    Ok((
        Meta::Probe(ProbeInfo {
            format: info.format.clone(),
            duration: info.duration,
            bitrate: info.bitrate as u64,
            stream_count: info.streams.len() as u32,
            best_video,
        }),
        Vec::new(),
    ))
}

#[derive(Clone, Default)]
struct SharedBuffer(Rc<RefCell<Cursor<Vec<u8>>>>);

impl Write for SharedBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for SharedBuffer {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.0.borrow_mut().seek(pos)
    }
}

fn compress(mime_type: &str) -> JobResult {
    if !mime_type.starts_with("image/") {
        return Err(Error::msg("MIME type not supported").into());
    }
    if mime_type == "image/webp" {
        return Err(Error::msg("MIME type is already image/webp").into());
    }
    let out = SharedBuffer::default();
    unsafe {
        let demuxer = open_demuxer()?;
        let muxer = Muxer::builder()
            .with_output_write_seek(out.clone(), Some("webp"))?
            .build()?;
        let mut trans = Transcoder::new_custom_io(demuxer, muxer);
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

        let bytes = out.0.borrow().get_ref().clone();
        Ok((
            Meta::Compressed {
                width: image_stream.width as u32,
                height: image_stream.height as u32,
                duration: if probe.duration < 0. {
                    0.
                } else {
                    probe.duration
                },
                bitrate: probe.bitrate as u32,
            },
            bytes,
        ))
    }
}

fn encode_picture(enc: Encoder, frame: &AvFrameRef) -> Result<Vec<u8>> {
    let mut enc = enc;
    let mut out = Vec::new();
    for pkt in enc.encode_frame(Some(frame))? {
        out.extend_from_slice(unsafe { slice::from_raw_parts(pkt.data, pkt.size as usize) });
    }
    for pkt in enc.encode_frame(None)? {
        out.extend_from_slice(unsafe { slice::from_raw_parts(pkt.data, pkt.size as usize) });
    }
    Ok(out)
}

fn thumbnail() -> JobResult {
    unsafe {
        let mut input = open_demuxer()?;
        let probe = probe_demuxer(&mut input)?;

        let image_stream = probe
            .streams
            .iter()
            .find(|c| c.stream_type == StreamType::Video)
            .ok_or(Error::msg("No image found, cant compress"))?;

        let w = THUMBNAIL_WIDTH;
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
            if let Some(pkt) = pkt.as_ref()
                && pkt.stream_index != image_stream.index as i32
            {
                continue;
            }
            if let Ok(results) = decoder.decode_pkt(pkt.as_ref())
                && let Some((frame, _)) = results.into_iter().next()
            {
                let frame_save = sws.process_frame(&frame, w, h, AV_PIX_FMT_YUV420P)?;
                let bytes = encode_picture(enc, &frame_save)?;
                return Ok((
                    Meta::Image {
                        width: w as u32,
                        height: h as u32,
                    },
                    bytes,
                ));
            }
        }
        Err(JobError::NoImageData)
    }
}

fn pack_rgb(frame: &AvFrameRef, width: usize, height: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(3 * width * height);
    let stride = frame.linesize[0] as usize;
    for row in 0..height {
        out.extend_from_slice(unsafe {
            slice::from_raw_parts(frame.data[0].add(stride * row), 3 * width)
        });
    }
    out
}

fn rgb() -> JobResult {
    let mut demux = open_demuxer()?;
    let info = probe_demuxer(&mut demux)?;
    let stream = info
        .best_video()
        .ok_or(Error::msg("no video/image stream found"))?;
    let width = stream.width as u32;
    let height = stream.height as u32;

    let mut decoder = Decoder::new();
    decoder.setup_decoder(stream, None)?;
    let mut scaler = Scaler::new();

    while let Ok((pkt, _)) = unsafe { demux.get_packet() } {
        let pkt = match pkt {
            Some(p) => p,
            None => break,
        };
        let decoded = decoder.decode_pkt(Some(&pkt))?;
        if let Some((frame, _)) = decoded.into_iter().next() {
            let scaled =
                scaler.process_frame(&frame, width as u16, height as u16, AV_PIX_FMT_RGB24)?;
            return Ok((
                Meta::Image { width, height },
                pack_rgb(&scaled, width as usize, height as usize),
            ));
        }
    }
    Err(Error::msg("no image data found").into())
}

fn label_image(width: u32, height: u32) -> JobResult {
    let mut demux = open_demuxer()?;
    let info = probe_demuxer(&mut demux)?;
    let image_stream = info.best_video().ok_or(JobError::NoMediaStream)?;
    check_dimensions(image_stream.width as u32, image_stream.height as u32)?;

    let mut decoder = Decoder::new();
    decoder.setup_decoder(image_stream, None)?;
    let mut scaler = Scaler::new();

    let mut scale_first = |decoded: Vec<(AvFrameRef, i32)>| -> Result<Option<Vec<u8>>> {
        let Some((frame, _)) = decoded.into_iter().next() else {
            return Ok(None);
        };
        let scaled = scaler.process_frame(&frame, width as u16, height as u16, AV_PIX_FMT_RGB24)?;
        Ok(Some(pack_rgb(&scaled, width as usize, height as usize)))
    };

    let done = |pixels| Ok((Meta::Image { width, height }, pixels));

    while let Ok((pkt, _)) = unsafe { demux.get_packet() } {
        let pkt = match pkt {
            Some(p) => p,
            None => break,
        };
        let decoded = decoder.decode_pkt(Some(&pkt))?;
        if let Some(pixels) = scale_first(decoded)? {
            return done(pixels);
        }
    }
    let flushed = decoder.decode_pkt(None)?;
    if let Some(pixels) = scale_first(flushed)? {
        return done(pixels);
    }
    Err(JobError::NoImageData)
}

fn label_video(width: u32, height: u32) -> JobResult {
    let mut demux = open_demuxer()?;
    let info = probe_demuxer(&mut demux)?;
    let video_stream = info.best_video().ok_or(JobError::NoMediaStream)?;
    check_dimensions(video_stream.width as u32, video_stream.height as u32)?;

    let stream_index = video_stream.index as i32;
    let (tb_num, tb_den) = (
        video_stream.timebase.0 as f64,
        video_stream.timebase.1 as f64,
    );
    let start_secs = if video_stream.start_time.is_finite() {
        video_stream.start_time as f64
    } else {
        0.0
    };
    let duration = if info.duration.is_finite() && info.duration > 0.0 {
        info.duration as f64
    } else {
        0.0
    };

    let mut scaler = Scaler::new();
    let mut payload = Vec::new();
    let mut count = 0u32;

    for target_secs in video_sample_times(duration, MAX_VIDEO_FRAMES) {
        let target_secs = target_secs + start_secs;
        let seek_ts = (target_secs * tb_den / tb_num) as i64;
        let ret =
            unsafe { av_seek_frame(demux.context(), stream_index, seek_ts, AVSEEK_FLAG_BACKWARD) };
        if ret < 0 {
            continue;
        }

        let mut decoder = Decoder::new();
        decoder.setup_decoder(video_stream, None)?;

        let mut picked = None;
        let mut last = None;
        'packets: while let Ok((pkt, _)) = unsafe { demux.get_packet() } {
            let Some(pkt) = pkt else { break };
            if pkt.stream_index != stream_index {
                continue;
            }
            for (frame, _) in decoder.decode_pkt(Some(&pkt))? {
                let frame_secs = frame.pts as f64 * tb_num / tb_den;
                if frame.pts == AV_NOPTS_VALUE || frame_secs >= target_secs {
                    picked = Some(frame);
                    break 'packets;
                }
                last = Some(frame);
            }
        }
        if picked.is_none() {
            picked = last;
        }

        if let Some(frame) = picked
            && let Ok(scaled) =
                scaler.process_frame(&frame, width as u16, height as u16, AV_PIX_FMT_RGB24)
        {
            payload.extend_from_slice(&pack_rgb(&scaled, width as usize, height as usize));
            count += 1;
        }
    }

    Ok((
        Meta::Frames {
            count,
            width,
            height,
        },
        payload,
    ))
}

fn llm_jpeg(max_pixels: u64) -> JobResult {
    unsafe {
        let mut demux = open_demuxer()?;
        let probe = probe_demuxer(&mut demux)?;
        let stream = probe
            .streams
            .iter()
            .find(|s| s.stream_type == StreamType::Video)
            .ok_or(JobError::NoMediaStream)?;

        check_dimensions(stream.width as u32, stream.height as u32)?;

        let (out_width, out_height) =
            llm_target_size(stream.width as u32, stream.height as u32, max_pixels);
        let (out_width, out_height) = (out_width as i32, out_height as i32);

        let enc = Encoder::new(AV_CODEC_ID_MJPEG)?
            .with_width(out_width)
            .with_height(out_height)
            .with_pix_fmt(AV_PIX_FMT_YUVJ420P)
            .open(None)?;

        let mut sws = Scaler::new();
        let mut decoder = Decoder::new();
        decoder.setup_decoder(stream, None)?;
        let stream_index = stream.index as i32;

        let mut packet_count = 0;
        let max_packets = 10;

        while let Ok((pkt, _)) = demux.get_packet() {
            packet_count += 1;
            if packet_count > max_packets {
                return Err(JobError::Corrupt {
                    detail: format!(
                        "exceeded {} packet attempts without producing a frame",
                        max_packets
                    ),
                });
            }

            if let Some(ref pkt) = pkt
                && pkt.stream_index != stream_index
            {
                continue;
            }
            if let Ok(results) = decoder.decode_pkt(pkt.as_ref())
                && let Some((frame, _)) = results.into_iter().next()
            {
                let scaled = sws.process_frame(
                    &frame,
                    out_width as u16,
                    out_height as u16,
                    AV_PIX_FMT_YUVJ420P,
                )?;
                let bytes = encode_picture(enc, &scaled)?;
                return Ok((
                    Meta::Image {
                        width: out_width as u32,
                        height: out_height as u32,
                    },
                    bytes,
                ));
            }
        }
        Err(JobError::NoImageData)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_sample_times_spread_across_duration() {
        let times = video_sample_times(100.0, 10);
        assert_eq!(times.len(), 10);
        assert!((times[0] - 5.0).abs() < 1e-9);
        assert!((times[9] - 95.0).abs() < 1e-9);
        assert_eq!(video_sample_times(0.0, 10), vec![0.0]);
    }

    #[test]
    fn check_dimensions_rejects_degenerate_media() {
        assert!(check_dimensions(224, 224).is_ok());
        assert!(check_dimensions(MIN_LABEL_DIMENSION, MIN_LABEL_DIMENSION).is_ok());
        assert_eq!(
            check_dimensions(900, 1),
            Err(JobError::TooSmall {
                width: 900,
                height: 1
            })
        );
        assert!(check_dimensions(1, 900).is_err());
    }

    #[test]
    fn blocked_demuxer_formats_are_rejected_by_token() {
        assert!(check_demuxer_format("hls,applehttp").is_err());
        assert!(check_demuxer_format("concat").is_err());
        assert!(check_demuxer_format("mov,mp4,m4a,3gp,3g2,mj2").is_ok());
        assert!(check_demuxer_format("webm,matroska").is_ok());
    }
}
