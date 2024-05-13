use std::collections::HashMap;
use std::mem::transmute;
use std::path::PathBuf;
use std::ptr;

use anyhow::Error;
use ffmpeg_sys_the_third::{AV_CODEC_FLAG_GLOBAL_HEADER, av_dump_format, av_find_best_stream, av_frame_alloc, av_frame_copy_props, av_frame_free, av_guess_format, av_interleaved_write_frame, av_packet_alloc, av_packet_free, av_packet_rescale_ts, av_packet_unref, AV_PROFILE_H264_HIGH, av_read_frame, av_write_trailer, AVCodec, avcodec_alloc_context3, avcodec_find_encoder, avcodec_free_context, avcodec_open2, avcodec_parameters_from_context, avcodec_parameters_to_context, avcodec_receive_frame, avcodec_receive_packet, avcodec_send_frame, avcodec_send_packet, AVCodecContext, AVCodecID, AVERROR, AVERROR_EOF, AVERROR_STREAM_NOT_FOUND, AVFMT_GLOBALHEADER, avformat_alloc_output_context2, avformat_close_input, avformat_find_stream_info, avformat_free_context, avformat_init_output, avformat_new_stream, avformat_open_input, avformat_write_header, AVFormatContext, AVIO_FLAG_WRITE, avio_open, AVMediaType, AVPacket, sws_freeContext, sws_getContext, sws_scale_frame, SwsContext};
use ffmpeg_sys_the_third::AVMediaType::{AVMEDIA_TYPE_AUDIO, AVMEDIA_TYPE_VIDEO};
use ffmpeg_sys_the_third::AVPixelFormat::AV_PIX_FMT_YUV420P;
use libc::EAGAIN;

use crate::processing::{FileProcessor, FileProcessorResult, NewFileProcessorResult};
use crate::processing::blurhash::make_blur_hash;

/// Image converter to WEBP
pub struct WebpProcessor {
    encoders: HashMap<usize, *mut AVCodecContext>,
    decoders: HashMap<usize, *mut AVCodecContext>,
    scalers: HashMap<usize, *mut SwsContext>,
    stream_map: HashMap<usize, usize>,
    width: Option<usize>,
    height: Option<usize>,
    blur_hash: Option<String>,
}

unsafe impl Sync for WebpProcessor {}

unsafe impl Send for WebpProcessor {}

impl WebpProcessor {
    pub fn new() -> Self {
        Self {
            encoders: HashMap::new(),
            decoders: HashMap::new(),
            scalers: HashMap::new(),
            stream_map: HashMap::new(),
            width: None,
            height: None,
            blur_hash: None,
        }
    }

    unsafe fn transcode_pkt(&mut self, pkt: *mut AVPacket, in_fmt: *mut AVFormatContext, out_fmt: *mut AVFormatContext) -> Result<(), Error> {
        let idx = (*pkt).stream_index as usize;
        let out_idx = match self.stream_map.get(&idx) {
            Some(i) => i,
            None => return Ok(())
        };
        let in_stream = *(*in_fmt).streams.add(idx);
        let out_stream = *(*out_fmt).streams.add(*out_idx);
        av_packet_rescale_ts(pkt, (*in_stream).time_base, (*out_stream).time_base);

        let dec_ctx = self.decoders.get_mut(&idx).expect("Missing decoder config");
        let enc_ctx = self.encoders.get_mut(&out_idx).expect("Missing encoder config");

        let ret = avcodec_send_packet(*dec_ctx, pkt);
        if ret < 0 {
            return Err(Error::msg("Failed to decode packet"));
        }

        let mut frame = av_frame_alloc();
        let mut frame_out = av_frame_alloc();
        loop {
            let ret = avcodec_receive_frame(*dec_ctx, frame);
            if ret == AVERROR_EOF || ret == AVERROR(EAGAIN) {
                break;
            } else if ret < 0 {
                return Err(Error::msg("Frame read error"));
            }

            let frame_out = match self.scalers.get_mut(&out_idx) {
                Some(sws) => {
                    av_frame_copy_props(frame_out, frame);
                    let ret = sws_scale_frame(*sws, frame_out, frame);
                    if ret < 0 {
                        return Err(Error::msg("Failed to scale frame"));
                    }
                    frame_out
                }
                None => frame
            };

            // take blur_hash from first video frame
            if (*(*out_stream).codecpar).codec_type == AVMEDIA_TYPE_VIDEO && self.blur_hash.is_none() {
                self.blur_hash = Some(make_blur_hash(frame_out, 9)?);
            }

            let ret = avcodec_send_frame(*enc_ctx, frame_out);
            if ret < 0 {
                return Err(Error::msg("Failed to encode frame"));
            }
            av_packet_unref(pkt);
            loop {
                let ret = avcodec_receive_packet(*enc_ctx, pkt);
                if ret == AVERROR_EOF || ret == AVERROR(EAGAIN) {
                    break;
                } else if ret < 0 {
                    return Err(Error::msg("Frame read error"));
                }

                av_packet_rescale_ts(pkt, (*in_stream).time_base, (*out_stream).time_base);
                let ret = av_interleaved_write_frame(out_fmt, pkt);
                if ret < 0 {
                    return Err(Error::msg("Failed to encode frame"));
                }
            }
        }
        av_frame_free(&mut frame_out);
        av_frame_free(&mut frame);
        Ok(())
    }

    unsafe fn setup_decoder(&mut self, in_fmt: *mut AVFormatContext, av_type: AVMediaType) -> Result<i32, Error> {
        let mut decoder: *const AVCodec = ptr::null_mut();
        let stream_idx = av_find_best_stream(in_fmt, av_type, -1, -1, &mut decoder, 0);
        if stream_idx == AVERROR_STREAM_NOT_FOUND {
            return Ok(stream_idx);
        }
        let decoder_ctx = avcodec_alloc_context3(decoder);
        if decoder_ctx.is_null() {
            return Err(Error::msg("Failed to open video decoder"));
        }

        let in_stream = *(*in_fmt).streams.add(stream_idx as usize);
        let ret = avcodec_parameters_to_context(decoder_ctx, (*in_stream).codecpar);
        if ret < 0 {
            return Err(Error::msg("Failed to copy codec params to decoder"));
        }

        let ret = avcodec_open2(decoder_ctx, decoder, ptr::null_mut());
        if ret < 0 {
            return Err(Error::msg("Failed to open decoder"));
        }

        self.decoders.insert(stream_idx as usize, decoder_ctx);
        Ok(stream_idx)
    }

    unsafe fn setup_encoder(&mut self, in_fmt: *mut AVFormatContext, out_fmt: *mut AVFormatContext, in_idx: i32) -> Result<(), Error> {
        let in_stream = *(*in_fmt).streams.add(in_idx as usize);
        let stream_type = (*(*in_stream).codecpar).codec_type;
        let out_codec = match stream_type {
            AVMEDIA_TYPE_VIDEO => avcodec_find_encoder((*(*out_fmt).oformat).video_codec),
            AVMEDIA_TYPE_AUDIO => avcodec_find_encoder((*(*out_fmt).oformat).audio_codec),
            _ => ptr::null_mut()
        };
        // not mapped ignore
        if out_codec.is_null() {
            return Ok(());
        }
        let stream = avformat_new_stream(out_fmt, out_codec);

        let encoder_ctx = avcodec_alloc_context3(out_codec);
        if encoder_ctx.is_null() {
            return Err(Error::msg("Failed to create encoder context"));
        }

        match stream_type {
            AVMEDIA_TYPE_VIDEO => {
                (*encoder_ctx).width = (*(*in_stream).codecpar).width;
                (*encoder_ctx).height = (*(*in_stream).codecpar).height;
                (*encoder_ctx).pix_fmt = AV_PIX_FMT_YUV420P;
                (*encoder_ctx).time_base = (*in_stream).time_base;
                (*encoder_ctx).framerate = (*in_stream).avg_frame_rate;
                if (*out_codec).id == AVCodecID::AV_CODEC_ID_H264 {
                    (*encoder_ctx).profile = AV_PROFILE_H264_HIGH;
                    (*encoder_ctx).level = 50;
                    (*encoder_ctx).qmin = 20;
                    (*encoder_ctx).qmax = 30;
                }
                (*stream).time_base = (*encoder_ctx).time_base;
                (*stream).avg_frame_rate = (*encoder_ctx).framerate;
                (*stream).r_frame_rate = (*encoder_ctx).framerate;
            }
            AVMEDIA_TYPE_AUDIO => {
                (*encoder_ctx).sample_rate = (*(*in_stream).codecpar).sample_rate;
                (*encoder_ctx).sample_fmt = transmute((*(*in_stream).codecpar).format);
                (*encoder_ctx).ch_layout = (*(*in_stream).codecpar).ch_layout;
                (*encoder_ctx).time_base = (*in_stream).time_base;
                (*stream).time_base = (*encoder_ctx).time_base;
            }
            _ => {}
        }

        if (*(*out_fmt).oformat).flags & AVFMT_GLOBALHEADER == AVFMT_GLOBALHEADER {
            (*encoder_ctx).flags |= AV_CODEC_FLAG_GLOBAL_HEADER as libc::c_int;
        }

        let ret = avcodec_open2(encoder_ctx, out_codec, ptr::null_mut());
        if ret < 0 {
            return Err(Error::msg("Failed to open encoder"));
        }

        let ret = avcodec_parameters_from_context((*stream).codecpar, encoder_ctx);
        if ret < 0 {
            return Err(Error::msg("Failed to open encoder"));
        }

        let out_idx = (*stream).index as usize;
        // setup scaler if pix_fmt doesnt match
        if stream_type == AVMEDIA_TYPE_VIDEO &&
            (*(*in_stream).codecpar).format != (*(*stream).codecpar).format {
            let sws_ctx = sws_getContext((*(*in_stream).codecpar).width,
                                         (*(*in_stream).codecpar).height,
                                         transmute((*(*in_stream).codecpar).format),
                                         (*(*stream).codecpar).width,
                                         (*(*stream).codecpar).height,
                                         transmute((*(*stream).codecpar).format),
                                         0, ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
            if sws_ctx.is_null() {
                return Err(Error::msg("Failed to create sws context"));
            }
            self.scalers.insert(out_idx, sws_ctx);
        }

        self.encoders.insert(out_idx, encoder_ctx);
        self.stream_map.insert(in_idx as usize, out_idx);
        Ok(())
    }

    unsafe fn flush_output(&mut self, out_fmt: *mut AVFormatContext) -> Result<(), Error> {
        let mut pkt = av_packet_alloc();
        for encoder in self.encoders.values() {
            avcodec_send_frame(*encoder, ptr::null_mut());
            loop {
                let ret = avcodec_receive_packet(*encoder, pkt);
                if ret == AVERROR_EOF || ret == AVERROR(EAGAIN) {
                    break;
                } else if ret < 0 {
                    return Err(Error::msg("Frame read error"));
                }

                let ret = av_interleaved_write_frame(out_fmt, pkt);
                if ret < 0 {
                    return Err(Error::msg("Failed to encode frame"));
                }
            }
        }
        av_packet_free(&mut pkt);
        Ok(())
    }

    unsafe fn free(&mut self) -> Result<(), Error> {
        for decoders in self.decoders.values_mut() {
            avcodec_free_context(&mut *decoders);
        }
        self.decoders.clear();
        for encoders in self.encoders.values_mut() {
            avcodec_free_context(&mut *encoders);
        }
        self.encoders.clear();
        for scaler in self.scalers.values_mut() {
            sws_freeContext(*scaler);
        }
        self.scalers.clear();

        Ok(())
    }
}

impl Drop for WebpProcessor {
    fn drop(&mut self) {
        unsafe { self.free().unwrap(); }
    }
}

impl FileProcessor for WebpProcessor {
    fn process_file(&mut self, in_file: PathBuf, mime_type: &str) -> Result<FileProcessorResult, Error> {
        unsafe {
            let mut out_path = in_file.clone();
            out_path.set_extension("_compressed");

            let mut dec_fmt: *mut AVFormatContext = ptr::null_mut();
            let ret = avformat_open_input(&mut dec_fmt,
                                          format!("{}\0", in_file.into_os_string().into_string().unwrap()).as_ptr() as *const libc::c_char,
                                          ptr::null_mut(),
                                          ptr::null_mut());
            if ret < 0 {
                return Err(Error::msg("Failed to create input context"));
            }

            let ret = avformat_find_stream_info(dec_fmt, ptr::null_mut());
            if ret < 0 {
                return Err(Error::msg("Failed to probe input"));
            }
            let in_video_stream = self.setup_decoder(dec_fmt, AVMEDIA_TYPE_VIDEO)?;
            let in_audio_stream = self.setup_decoder(dec_fmt, AVMEDIA_TYPE_AUDIO)?;

            let out_format = if mime_type.starts_with("image/") {
                av_guess_format("webp\0".as_ptr() as *const libc::c_char,
                                ptr::null_mut(),
                                ptr::null_mut())
            } else if mime_type.starts_with("video/") {
                av_guess_format("matroska\0".as_ptr() as *const libc::c_char,
                                ptr::null_mut(),
                                ptr::null_mut())
            } else {
                return Err(Error::msg("Mime type not supported"));
            };

            let out_filename = format!("{}\0", out_path.clone().into_os_string().into_string().unwrap());
            let mut out_fmt: *mut AVFormatContext = ptr::null_mut();
            let ret = avformat_alloc_output_context2(&mut out_fmt,
                                                     out_format,
                                                     ptr::null_mut(),
                                                     out_filename.as_ptr() as *const libc::c_char);
            if ret < 0 {
                return Err(Error::msg("Failed to create output context"));
            }

            let ret = avio_open(&mut (*out_fmt).pb, (*out_fmt).url, AVIO_FLAG_WRITE);
            if ret < 0 {
                return Err(Error::msg("Failed to open output IO"));
            }

            if in_video_stream != AVERROR_STREAM_NOT_FOUND {
                self.setup_encoder(dec_fmt, out_fmt, in_video_stream)?;
                let video_stream = *(*dec_fmt).streams.add(in_video_stream as usize);
                self.width = Some((*(*video_stream).codecpar).width as usize);
                self.height = Some((*(*video_stream).codecpar).height as usize);
            }
            if in_audio_stream != AVERROR_STREAM_NOT_FOUND {
                self.setup_encoder(dec_fmt, out_fmt, in_audio_stream)?;
            }

            let ret = avformat_init_output(out_fmt, ptr::null_mut());
            if ret < 0 {
                return Err(Error::msg("Failed to write output"));
            }

            av_dump_format(dec_fmt, 0, ptr::null_mut(), 0);
            av_dump_format(out_fmt, 0, ptr::null_mut(), 1);

            let ret = avformat_write_header(out_fmt, ptr::null_mut());
            if ret < 0 {
                return Err(Error::msg("Failed to write header to output"));
            }

            let mut pkt = av_packet_alloc();
            loop {
                let ret = av_read_frame(dec_fmt, pkt);
                if ret < 0 {
                    break;
                }
                self.transcode_pkt(pkt, dec_fmt, out_fmt)?;
            }

            // flush encoder
            self.flush_output(out_fmt)?;

            av_write_trailer(out_fmt);
            av_packet_free(&mut pkt);

            self.free()?;

            avformat_close_input(&mut dec_fmt);
            avformat_free_context(dec_fmt);
            avformat_free_context(out_fmt);

            Ok(FileProcessorResult::NewFile(
                NewFileProcessorResult {
                    result: out_path,
                    mime_type: "image/webp".to_string(),
                    width: self.width.unwrap_or(0),
                    height: self.height.unwrap_or(0),
                    blur_hash: match &self.blur_hash {
                        Some(s) => s.clone(),
                        None => "".to_string()
                    },
                }))
        }
    }
}