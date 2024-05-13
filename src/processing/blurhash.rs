use std::{ptr, slice};
use std::intrinsics::transmute;
use std::time::SystemTime;

use anyhow::Error;
use blurhash::encode;
use ffmpeg_sys_the_third::{av_frame_alloc, av_frame_free, AVFrame, sws_freeContext, sws_getContext, sws_scale_frame};
use ffmpeg_sys_the_third::AVPixelFormat::AV_PIX_FMT_RGBA;
use log::info;

pub unsafe fn make_blur_hash(frame: *mut AVFrame, detail: u32) -> Result<String, Error> {
    let start = SystemTime::now();
    let sws_ctx = sws_getContext((*frame).width,
                                 (*frame).height,
                                 transmute((*frame).format),
                                 (*frame).width,
                                 (*frame).height,
                                 AV_PIX_FMT_RGBA,
                                 0, ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
    if sws_ctx.is_null() {
        return Err(Error::msg("Failed to create sws context"));
    }

    let mut dst_frame = av_frame_alloc();
    let ret = sws_scale_frame(sws_ctx, dst_frame, frame);
    if ret < 0 {
        return Err(Error::msg("Failed to scale frame (blurhash)"));
    }

    let pic_slice = slice::from_raw_parts_mut((*dst_frame).data[0], ((*frame).width * (*frame).height * 4) as usize);
    let bh = encode(detail, detail,
                    (*frame).width as u32,
                    (*frame).height as u32,
                    pic_slice,
    )?;

    av_frame_free(&mut dst_frame);
    sws_freeContext(sws_ctx);

    info!("Generated blurhash in {}ms", SystemTime::now().duration_since(start).unwrap().as_millis());
    Ok(bh)
}