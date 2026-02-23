use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::slice;

use anyhow::{Error, Result};
use candle_core::{D, DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::vit;
use ffmpeg_rs_raw::ffmpeg_sys_the_third::AVPixelFormat::AV_PIX_FMT_RGB24;
use ffmpeg_rs_raw::ffmpeg_sys_the_third::{av_frame_free, av_packet_free};
use ffmpeg_rs_raw::{Decoder, Demuxer, Scaler};
use log::debug;
use nostr::serde_json;
use serde::Deserialize;

/// Minimum confidence threshold for a label to be included
const MIN_CONFIDENCE: f32 = 0.1;

#[derive(Deserialize)]
struct MyVitConfig {
    pub id2label: HashMap<usize, String>,
}

pub fn label_frame(frame: &Path, model: PathBuf, config: PathBuf) -> Result<HashMap<String, f32>> {
    unsafe {
        let device = Device::Cpu;
        let image = load_frame_224(frame)?.to_device(&device)?;

        let vb = VarBuilder::from_mmaped_safetensors(&[model], DType::F32, &device)?;
        let config_data = std::fs::read(config)?;
        let config: vit::Config = serde_json::from_slice(&config_data)?;
        let label_config: MyVitConfig = serde_json::from_slice(&config_data)?;
        let model = vit::Model::new(&config, label_config.id2label.len(), vb)?;
        let logits = model.forward(&image.unsqueeze(0)?)?;
        let prs = candle_nn::ops::softmax(&logits, D::Minus1)?
            .i(0)?
            .to_vec1::<f32>()?;
        let mut prs = prs.iter().enumerate().collect::<Vec<_>>();
        prs.sort_by(|(_, p1), (_, p2)| p2.total_cmp(p1));
        let res: HashMap<String, f32> = prs
            .iter()
            .filter(|&(_c, q)| **q >= MIN_CONFIDENCE)
            .take(5)
            .map(|&(c, q)| (label_config.id2label[&c].to_string(), *q))
            .collect();
        debug!("label results: {:?}", res);
        Ok(res)
    }
}

/// Load an image from disk into RGB pixel buffer
unsafe fn load_image(path_buf: &Path, width: usize, height: usize) -> Result<Vec<u8>> {
    let mut demux = Demuxer::new(path_buf.to_str().unwrap())?;
    let info = demux.probe_input()?;
    let image_stream = info
        .best_video()
        .ok_or(Error::msg("No image stream found"))?;

    let mut decoder = Decoder::new();
    decoder.setup_decoder(image_stream, None)?;

    // TODO: crop image square
    let mut scaler = Scaler::new();
    while let Ok((mut pkt, _)) = demux.get_packet() {
        if let Some(mut frame) = decoder.decode_pkt(pkt)?.into_iter().next() {
            let mut new_frame =
                scaler.process_frame(frame, width as u16, height as u16, AV_PIX_FMT_RGB24)?;
            let mut dst_vec = Vec::with_capacity(3 * width * height);

            for row in 0..height {
                let line_size = (*new_frame).linesize[0] as usize;
                let row_offset = line_size * row;
                let row_slice =
                    slice::from_raw_parts((*new_frame).data[0].add(row_offset), 3 * width);
                dst_vec.extend_from_slice(row_slice);
            }
            av_frame_free(&mut frame);
            av_frame_free(&mut new_frame);
            av_packet_free(&mut pkt);
            return Ok(dst_vec);
        }
    }
    Err(Error::msg("No image data found"))
}

// https://github.com/huggingface/candle/blob/main/candle-examples/src/imagenet.rs
unsafe fn load_frame_224(path: &Path) -> Result<Tensor> {
    let pic = load_image(path, 224, 224)?;

    let d = Device::cuda_if_available(0)?;
    let data = Tensor::from_vec(pic, (224, 224, 3), &d)?.permute((2, 0, 1))?;
    let mean = Tensor::new(&[0.485f32, 0.456, 0.406], &d)?.reshape((3, 1, 1))?;
    let std = Tensor::new(&[0.229f32, 0.224, 0.225], &d)?.reshape((3, 1, 1))?;
    let res = (data.to_dtype(DType::F32)? / 255.)?
        .broadcast_sub(&mean)?
        .broadcast_div(&std)?;
    Ok(res)
}
