use std::mem::transmute;
use std::path::PathBuf;
use std::ptr;

use anyhow::Error;
use ffmpeg_sys_the_third::{av_dump_format, av_frame_alloc, av_frame_copy_props, av_frame_free, av_guess_format, av_interleaved_write_frame, av_packet_alloc, av_packet_free, av_packet_rescale_ts, av_packet_unref, av_read_frame, av_write_trailer, avcodec_alloc_context3, avcodec_find_decoder, avcodec_find_encoder, avcodec_free_context, avcodec_open2, avcodec_parameters_from_context, avcodec_parameters_to_context, avcodec_receive_frame, avcodec_receive_packet, avcodec_send_frame, avcodec_send_packet, AVERROR, AVERROR_EOF, avformat_alloc_output_context2, avformat_close_input, avformat_find_stream_info, avformat_free_context, avformat_init_output, avformat_new_stream, avformat_open_input, avformat_write_header, AVFormatContext, AVIO_FLAG_WRITE, avio_open, sws_getContext, sws_scale_frame};
use ffmpeg_sys_the_third::AVPictureType::AV_PICTURE_TYPE_NONE;
use ffmpeg_sys_the_third::AVPixelFormat::AV_PIX_FMT_YUV420P;
use libc::EAGAIN;

use crate::processing::FileProcessor;

pub struct ImageProcessor {}

impl ImageProcessor {
    pub fn new() -> Self {
        Self {}
    }
}

impl FileProcessor for ImageProcessor {
    fn process_file(&mut self, in_file: PathBuf, mime_type: &str) -> Result<PathBuf, Error> {
        unsafe {
            let mut out_path = in_file.clone();
            out_path.set_extension("_compressed");
            
            let mut dec_fmt: *mut AVFormatContext = ptr::null_mut();
            let ret = avformat_open_input(&mut dec_fmt,
                                          format!("{}\0", in_file.into_os_string().into_string().unwrap()).as_ptr() as *const libc::c_char,
                                          ptr::null_mut(),
                                          ptr::null_mut());
            if ret < 0 {
                panic!("Failed to create input context")
            }

            let ret = avformat_find_stream_info(dec_fmt, ptr::null_mut());
            if ret < 0 {
                panic!("Failed to probe input")
            }

            let in_stream = *(*dec_fmt).streams.add(0);
            let decoder = avcodec_find_decoder((*(*in_stream).codecpar).codec_id);
            let mut dec_ctx = avcodec_alloc_context3(decoder);
            if dec_ctx.is_null() {
                panic!("Failed to open decoder")
            }

            let ret = avcodec_parameters_to_context(dec_ctx, (*in_stream).codecpar);
            if ret < 0 {
                panic!("Failed to copy codec params to decoder")
            }

            let ret = avcodec_open2(dec_ctx, decoder, ptr::null_mut());
            if ret < 0 {
                panic!("Failed to open decoder")
            }

            let out_format = av_guess_format("webp\0".as_ptr() as *const libc::c_char,
                                             ptr::null_mut(),
                                             "image/webp\0".as_ptr() as *const libc::c_char);
            let out_filename = format!("{}\0", out_path.clone().into_os_string().into_string().unwrap());
            let mut out_fmt: *mut AVFormatContext = ptr::null_mut();
            let ret = avformat_alloc_output_context2(&mut out_fmt,
                                                     out_format,
                                                     ptr::null_mut(),
                                                     out_filename.as_ptr() as *const libc::c_char);
            if ret < 0 {
                panic!("Failed to create output context")
            }
            
            let ret = avio_open(&mut (*out_fmt).pb, (*out_fmt).url, AVIO_FLAG_WRITE);
            if ret < 0 {
                panic!("Failed to open output IO")
            }
            
            let out_codec = avcodec_find_encoder((*(*out_fmt).oformat).video_codec);
            let stream = avformat_new_stream(out_fmt, out_codec);

            let mut encoder = avcodec_alloc_context3(out_codec);
            if encoder.is_null() {
                panic!("Failed to create encoder context")
            }
            (*encoder).width = (*(*in_stream).codecpar).width;
            (*encoder).height = (*(*in_stream).codecpar).height;
            (*encoder).pix_fmt = AV_PIX_FMT_YUV420P;
            (*encoder).time_base = (*in_stream).time_base;
            (*encoder).framerate = (*in_stream).avg_frame_rate;
            (*stream).time_base = (*encoder).time_base;
            (*stream).avg_frame_rate = (*encoder).framerate;
            (*stream).r_frame_rate = (*encoder).framerate;

            let ret = avcodec_open2(encoder, out_codec, ptr::null_mut());
            if ret < 0 {
                panic!("Failed to open encoder");
            }

            let ret = avcodec_parameters_from_context((*stream).codecpar, encoder);
            if ret < 0 {
                panic!("Failed to open encoder");
            }

            let ret = avformat_init_output(out_fmt, ptr::null_mut());
            if ret < 0 {
                panic!("Failed to write output");
            }

            av_dump_format(out_fmt, 0, ptr::null_mut(), 1);

            let sws_ctx = sws_getContext((*(*in_stream).codecpar).width,
                                         (*(*in_stream).codecpar).height,
                                         transmute((*(*in_stream).codecpar).format),
                                         (*(*stream).codecpar).width,
                                         (*(*stream).codecpar).height,
                                         transmute((*(*stream).codecpar).format),
                                         0, ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
            if sws_ctx.is_null() {
                panic!("Failed to create sws context");
            }
            
            let ret = avformat_write_header(out_fmt, ptr::null_mut());
            if ret < 0 {
                panic!("Failed to write header to output");
            }
            
            let mut pkt = av_packet_alloc();
            loop {
                let ret = av_read_frame(dec_fmt, pkt);
                if ret < 0 {
                    break;
                }

                let in_stream = *(*dec_fmt).streams.add((*pkt).stream_index as usize);
                let out_stream = *(*out_fmt).streams;
                av_packet_rescale_ts(pkt, (*in_stream).time_base, (*out_stream).time_base);

                let ret = avcodec_send_packet(dec_ctx, pkt);
                if ret < 0 {
                    panic!("Failed to decode packet");
                }


                let mut frame = av_frame_alloc();
                let mut frame_out = av_frame_alloc();
                loop {
                    let ret = avcodec_receive_frame(dec_ctx, frame);
                    if ret == AVERROR_EOF || ret == AVERROR(EAGAIN) {
                        break;
                    } else if ret < 0 {
                        panic!("Frame read error")
                    }

                    av_frame_copy_props(frame_out, frame);
                    let ret = sws_scale_frame(sws_ctx, frame_out, frame);
                    if ret < 0 {
                        panic!("Failed to scale frame")
                    }

                    (*frame_out).pict_type = AV_PICTURE_TYPE_NONE;
                    (*frame_out).time_base = (*in_stream).time_base;

                    let ret = avcodec_send_frame(encoder, frame_out);
                    if ret < 0 {
                        panic!("Failed to encode frame")
                    }
                    av_packet_unref(pkt);
                    loop {
                        let ret = avcodec_receive_packet(encoder, pkt);
                        if ret == AVERROR_EOF || ret == AVERROR(EAGAIN) {
                            break;
                        } else if ret < 0 {
                            panic!("Frame read error")
                        }

                        let ret = av_interleaved_write_frame(out_fmt, pkt);
                        if ret < 0 {
                            panic!("Failed to encode frame")
                        }
                    }
                }
                av_frame_free(&mut frame_out);
                av_frame_free(&mut frame);
            }

            // flush encoder
            avcodec_send_frame(encoder, ptr::null_mut());
            loop {
                let ret = avcodec_receive_packet(encoder, pkt);
                if ret == AVERROR_EOF || ret == AVERROR(EAGAIN) {
                    break;
                } else if ret < 0 {
                    panic!("Frame read error")
                }

                let ret = av_interleaved_write_frame(out_fmt, pkt);
                if ret < 0 {
                    panic!("Failed to encode frame")
                }
            }

            av_interleaved_write_frame(out_fmt, ptr::null_mut());
            av_write_trailer(out_fmt);
            
            av_packet_free(&mut pkt);
            avcodec_free_context(&mut dec_ctx);
            avcodec_free_context(&mut encoder);

            avformat_close_input(&mut dec_fmt);
            avformat_free_context(dec_fmt);
            avformat_free_context(out_fmt);

            Ok(out_path)
        }
    }
}