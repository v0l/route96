use std::intrinsics::transmute;
use std::path::PathBuf;
use std::ptr;

use anyhow::Error;
use ffmpeg_sys_the_third::{av_frame_alloc, AVFrame, AVPixelFormat, sws_freeContext, sws_getContext, sws_scale_frame};

use crate::processing::webp::WebpProcessor;

mod webp;
pub mod labeling;

pub(crate) enum FileProcessorResult {
    NewFile(NewFileProcessorResult),
    Skip,
}

pub(crate) struct NewFileProcessorResult {
    pub result: PathBuf,
    pub mime_type: String,
    pub width: usize,
    pub height: usize,

    /// The image as RBGA
    pub image: Vec<u8>,
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

unsafe fn resize_image(frame: *const AVFrame, width: usize, height: usize, pix_fmt: AVPixelFormat) -> Result<*mut AVFrame, Error> {
    let sws_ctx = sws_getContext((*frame).width,
                                 (*frame).height,
                                 transmute((*frame).format),
                                 width as libc::c_int,
                                 height as libc::c_int,
                                 pix_fmt,
                                 0, ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
    if sws_ctx.is_null() {
        return Err(Error::msg("Failed to create sws context"));
    }

    let dst_frame = av_frame_alloc();
    let ret = sws_scale_frame(sws_ctx, dst_frame, frame);
    if ret < 0 {
        return Err(Error::msg("Failed to scale frame"));
    }

    sws_freeContext(sws_ctx);
    Ok(dst_frame)
}