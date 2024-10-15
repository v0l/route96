use std::ffi::CStr;
use std::path::PathBuf;
use std::ptr;

use anyhow::Error;
use ffmpeg_sys_the_third::AVMediaType::{AVMEDIA_TYPE_AUDIO, AVMEDIA_TYPE_VIDEO};
use ffmpeg_sys_the_third::{
    avcodec_get_name, avformat_close_input, avformat_find_stream_info, avformat_free_context,
    avformat_open_input, AVFormatContext,
};

use crate::processing::{FileProcessorResult, ProbeResult, ProbeStream};

/// Image converter to WEBP
pub struct FFProbe {}

impl FFProbe {
    pub fn new() -> Self {
        Self {}
    }

    pub fn process_file(self, in_file: PathBuf) -> Result<FileProcessorResult, Error> {
        unsafe {
            let mut dec_fmt: *mut AVFormatContext = ptr::null_mut();
            let ret = avformat_open_input(
                &mut dec_fmt,
                format!("{}\0", in_file.into_os_string().into_string().unwrap()).as_ptr()
                    as *const libc::c_char,
                ptr::null_mut(),
                ptr::null_mut(),
            );
            if ret < 0 {
                // input might not be media
                return Ok(FileProcessorResult::Skip);
            }

            let ret = avformat_find_stream_info(dec_fmt, ptr::null_mut());
            if ret < 0 {
                return Err(Error::msg("Failed to probe input"));
            }

            let mut stream_info = vec![];
            let mut ptr_x = 0;
            while ptr_x < (*dec_fmt).nb_streams {
                let ptr = *(*dec_fmt).streams.add(ptr_x as usize);
                let codec_par = (*ptr).codecpar;
                let codec = CStr::from_ptr(avcodec_get_name((*codec_par).codec_id))
                    .to_str()?
                    .to_string();
                if (*codec_par).codec_type == AVMEDIA_TYPE_VIDEO {
                    stream_info.push(ProbeStream::Video {
                        width: (*codec_par).width as u32,
                        height: (*codec_par).height as u32,
                        codec,
                    });
                } else if (*codec_par).codec_type == AVMEDIA_TYPE_AUDIO {
                    stream_info.push(ProbeStream::Audio {
                        sample_rate: (*codec_par).sample_rate as u32,
                        codec,
                    });
                }
                ptr_x += 1;
            }

            avformat_close_input(&mut dec_fmt);
            avformat_free_context(dec_fmt);

            Ok(FileProcessorResult::Probe(ProbeResult {
                streams: stream_info,
            }))
        }
    }
}
