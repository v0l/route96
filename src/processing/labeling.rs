use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::slice;

use anyhow::{Error, Result};
use candle_core::{D, DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::vit;
use ffmpeg_rs_raw::ffmpeg_sys_the_third::AVPixelFormat::AV_PIX_FMT_RGB24;
use ffmpeg_rs_raw::{Decoder, Demuxer, Scaler};
use hf_hub::api::sync::Api;
use log::{debug, info};
use nostr::serde_json;
use serde::Deserialize;

/// Minimum confidence threshold for a label to be included
const MIN_CONFIDENCE: f32 = 0.1;

/// Maximum number of frames to sample from a video (1 per second, up to 60s)
const MAX_VIDEO_FRAMES: usize = 60;

/// Default HuggingFace model repo for ViT-224
const DEFAULT_HF_REPO: &str = "google/vit-base-patch16-224";

#[derive(Deserialize)]
struct MyVitConfig {
    pub id2label: HashMap<usize, String>,
}

/// A loaded ViT model ready for inference
struct VitModel {
    model: vit::Model,
    label_config: MyVitConfig,
    device: Device,
}

impl VitModel {
    /// Load model from explicit file paths
    fn load(model_path: PathBuf, config_path: PathBuf) -> Result<Self> {
        let device = Device::Cpu;
        let vb =
            unsafe { VarBuilder::from_mmaped_safetensors(&[model_path], DType::F32, &device)? };
        let config_data = std::fs::read(config_path)?;
        let config: vit::Config = serde_json::from_slice(&config_data)?;
        let label_config: MyVitConfig = serde_json::from_slice(&config_data)?;
        let model = vit::Model::new(&config, label_config.id2label.len(), vb)?;
        Ok(Self {
            model,
            label_config,
            device,
        })
    }

    /// Download model from HuggingFace and load it
    fn from_hf() -> Result<Self> {
        info!(
            "Downloading ViT model from HuggingFace: {}",
            DEFAULT_HF_REPO
        );
        let api = Api::new()?;
        let repo = api.model(DEFAULT_HF_REPO.to_string());
        let model_path = repo.get("model.safetensors")?;
        let config_path = repo.get("config.json")?;
        Self::load(model_path, config_path)
    }

    /// Run inference on a 224x224 RGB tensor, returns top labels above threshold
    fn classify(&self, image: &Tensor) -> Result<HashMap<String, f32>> {
        let image = image.to_device(&self.device)?;
        let logits = self.model.forward(&image.unsqueeze(0)?)?;
        let prs = candle_nn::ops::softmax(&logits, D::Minus1)?
            .i(0)?
            .to_vec1::<f32>()?;
        let mut prs = prs.iter().enumerate().collect::<Vec<_>>();
        prs.sort_by(|(_, p1), (_, p2)| p2.total_cmp(p1));
        let res: HashMap<String, f32> = prs
            .iter()
            .filter(|&(_c, q)| **q >= MIN_CONFIDENCE)
            .take(5)
            .map(|&(c, q)| (self.label_config.id2label[&c].to_string(), *q))
            .collect();
        Ok(res)
    }
}

/// Label a single image file using the ViT model.
///
/// If `model` and `config` are `None`, the model is automatically downloaded
/// from HuggingFace (`google/vit-base-patch16-224`).
pub fn label_frame(
    frame: &Path,
    model: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<HashMap<String, f32>> {
    let vit = match (model.as_ref(), config.as_ref()) {
        (Some(m), Some(c)) => VitModel::load(m.clone(), c.clone())?,
        _ => VitModel::from_hf()?,
    };
    let image = unsafe { load_frame_224(frame)? };
    let res = vit.classify(&image)?;
    debug!("label results: {:?}", res);
    Ok(res)
}

/// Label a video file by sampling 1 frame per second for up to 60 seconds.
///
/// If `model` and `config` are `None`, the model is automatically downloaded
/// from HuggingFace (`google/vit-base-patch16-224`).
///
/// Returns aggregated labels: for each label seen across sampled frames,
/// the confidence is the average score across all frames where it appeared.
pub fn label_video(
    video: &Path,
    model: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<HashMap<String, f32>> {
    let vit = match (model.as_ref(), config.as_ref()) {
        (Some(m), Some(c)) => VitModel::load(m.clone(), c.clone())?,
        _ => VitModel::from_hf()?,
    };
    let frames = unsafe { extract_video_frames(video)? };

    if frames.is_empty() {
        return Ok(HashMap::new());
    }

    // Accumulate (total_score, count) per label across all frames
    let mut label_acc: HashMap<String, (f32, u32)> = HashMap::new();

    for tensor in &frames {
        match vit.classify(tensor) {
            Ok(labels) => {
                for (label, score) in labels {
                    let entry = label_acc.entry(label).or_insert((0.0, 0));
                    entry.0 += score;
                    entry.1 += 1;
                }
            }
            Err(e) => {
                debug!("Failed to classify video frame: {}", e);
            }
        }
    }

    // Average the scores and filter by threshold
    let result: HashMap<String, f32> = label_acc
        .into_iter()
        .map(|(label, (total, count))| (label, total / count as f32))
        .filter(|(_, avg)| *avg >= MIN_CONFIDENCE)
        .collect();

    debug!(
        "video label results ({} frames sampled): {:?}",
        frames.len(),
        result
    );
    Ok(result)
}

/// Extract up to MAX_VIDEO_FRAMES frames from a video, one per second
unsafe fn extract_video_frames(path: &Path) -> Result<Vec<Tensor>> {
    let mut demux = Demuxer::new(path.to_str().unwrap())?;
    let info = unsafe { demux.probe_input()? };
    let video_stream = info
        .best_video()
        .ok_or(Error::msg("No video stream found"))?;

    let stream_index = video_stream.index as i32;
    let time_base_num = video_stream.timebase.0 as f64;
    let time_base_den = video_stream.timebase.1 as f64;

    let mut decoder = Decoder::new();
    decoder.setup_decoder(video_stream, None)?;

    let mut scaler = Scaler::new();
    let mut frames = Vec::new();
    let mut next_sample_sec: f64 = 0.0;

    while let Ok((pkt, _)) = unsafe { demux.get_packet() } {
        let pkt = match pkt {
            Some(p) => p,
            None => break, // EOF
        };

        // Skip packets from other streams
        if pkt.stream_index != stream_index {
            continue;
        }

        let decoded = decoder.decode_pkt(Some(&pkt))?;
        for (frame, _) in decoded {
            let pts_sec = frame.pts as f64 * time_base_num / time_base_den;

            // Stop after 60 seconds
            if pts_sec >= MAX_VIDEO_FRAMES as f64 {
                return Ok(frames);
            }

            // Sample 1 frame per second
            if pts_sec >= next_sample_sec {
                next_sample_sec = pts_sec.floor() + 1.0;

                match unsafe { frame_to_tensor(&frame, &mut scaler) } {
                    Ok(tensor) => frames.push(tensor),
                    Err(e) => {
                        debug!("Failed to convert video frame at {:.1}s: {}", pts_sec, e);
                    }
                }

                if frames.len() >= MAX_VIDEO_FRAMES {
                    return Ok(frames);
                }
            }
        }
    }

    // Flush decoder
    let flushed = decoder.decode_pkt(None)?;
    for (frame, _) in flushed {
        let pts_sec = frame.pts as f64 * time_base_num / time_base_den;
        if pts_sec >= MAX_VIDEO_FRAMES as f64 || frames.len() >= MAX_VIDEO_FRAMES {
            break;
        }
        if pts_sec >= next_sample_sec {
            if let Ok(tensor) = unsafe { frame_to_tensor(&frame, &mut scaler) } {
                frames.push(tensor);
            }
        }
    }

    Ok(frames)
}

/// Scale a decoded video frame to 224x224 RGB and convert to a normalized tensor
unsafe fn frame_to_tensor(
    frame: &ffmpeg_rs_raw::AvFrameRef,
    scaler: &mut Scaler,
) -> Result<Tensor> {
    let scaled = scaler.process_frame(frame, 224, 224, AV_PIX_FMT_RGB24)?;
    let width = 224usize;
    let height = 224usize;

    let mut dst_vec = Vec::with_capacity(3 * width * height);
    for row in 0..height {
        let line_size = (*scaled).linesize[0] as usize;
        let row_offset = line_size * row;
        let row_slice =
            unsafe { slice::from_raw_parts((*scaled).data[0].add(row_offset), 3 * width) };
        dst_vec.extend_from_slice(row_slice);
    }

    let d = Device::cuda_if_available(0)?;
    let data = Tensor::from_vec(dst_vec, (224, 224, 3), &d)?.permute((2, 0, 1))?;
    let mean = Tensor::new(&[0.485f32, 0.456, 0.406], &d)?.reshape((3, 1, 1))?;
    let std = Tensor::new(&[0.229f32, 0.224, 0.225], &d)?.reshape((3, 1, 1))?;
    let res = (data.to_dtype(DType::F32)? / 255.)?
        .broadcast_sub(&mean)?
        .broadcast_div(&std)?;
    Ok(res)
}

/// Load an image from disk into RGB pixel buffer
unsafe fn load_image(path_buf: &Path, width: usize, height: usize) -> Result<Vec<u8>> {
    let mut demux = Demuxer::new(path_buf.to_str().unwrap())?;
    let info = unsafe { demux.probe_input()? };
    let image_stream = info
        .best_video()
        .ok_or(Error::msg("No image stream found"))?;

    let mut decoder = Decoder::new();
    decoder.setup_decoder(image_stream, None)?;

    // TODO: crop image square
    let mut scaler = Scaler::new();
    while let Ok((pkt, _)) = unsafe { demux.get_packet() } {
        let pkt = match pkt {
            Some(p) => p,
            None => break,
        };
        let decoded = decoder.decode_pkt(Some(&pkt))?;
        if let Some((frame, _)) = decoded.into_iter().next() {
            let new_frame =
                scaler.process_frame(&frame, width as u16, height as u16, AV_PIX_FMT_RGB24)?;
            let mut dst_vec = Vec::with_capacity(3 * width * height);

            for row in 0..height {
                let line_size = (*new_frame).linesize[0] as usize;
                let row_offset = line_size * row;
                let row_slice = unsafe {
                    slice::from_raw_parts((*new_frame).data[0].add(row_offset), 3 * width)
                };
                dst_vec.extend_from_slice(row_slice);
            }
            return Ok(dst_vec);
        }
    }
    Err(Error::msg("No image data found"))
}

// https://github.com/huggingface/candle/blob/main/candle-examples/src/imagenet.rs
unsafe fn load_frame_224(path: &Path) -> Result<Tensor> {
    let pic = unsafe { load_image(path, 224, 224)? };

    let d = Device::cuda_if_available(0)?;
    let data = Tensor::from_vec(pic, (224, 224, 3), &d)?.permute((2, 0, 1))?;
    let mean = Tensor::new(&[0.485f32, 0.456, 0.406], &d)?.reshape((3, 1, 1))?;
    let std = Tensor::new(&[0.229f32, 0.224, 0.225], &d)?.reshape((3, 1, 1))?;
    let res = (data.to_dtype(DType::F32)? / 255.)?
        .broadcast_sub(&mean)?
        .broadcast_div(&std)?;
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::Once;

    const BBB_URL: &str =
        "https://download.blender.org/peach/bigbuckbunny_movies/BigBuckBunny_320x180.mp4";

    static DOWNLOAD_VIDEO: Once = Once::new();

    /// Download the first 5 seconds of Big Buck Bunny (cached across tests)
    fn get_test_video() -> PathBuf {
        let path = std::env::temp_dir().join("route96_test_bbb.mp4");
        DOWNLOAD_VIDEO.call_once(|| {
            if path.exists() {
                return;
            }
            let status = Command::new("ffmpeg")
                .args([
                    "-y",
                    "-t",
                    "5",
                    "-i",
                    BBB_URL,
                    "-c",
                    "copy",
                    path.to_str().unwrap(),
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .expect("ffmpeg must be installed to run labeling tests");
            assert!(
                status.success(),
                "ffmpeg failed to download Big Buck Bunny clip"
            );
        });
        assert!(path.exists(), "test video not found after download");
        path
    }

    #[test]
    fn test_extract_frames_from_real_video() {
        let video = get_test_video();
        let frames = unsafe { extract_video_frames(&video).unwrap() };

        // 5-second clip at 24fps should yield 5 frames (one at 0s, 1s, 2s, 3s, 4s)
        assert_eq!(frames.len(), 5, "expected 5 frames from a 5s video clip");

        // Each frame should be a normalized 224x224 RGB tensor
        for (i, frame) in frames.iter().enumerate() {
            assert_eq!(frame.dims(), &[3, 224, 224], "frame {} has wrong shape", i);
            assert_eq!(frame.dtype(), DType::F32, "frame {} should be F32", i);
        }
    }

    #[test]
    fn test_tensor_values_normalized() {
        let video = get_test_video();
        let frames = unsafe { extract_video_frames(&video).unwrap() };
        assert!(!frames.is_empty(), "should extract at least 1 frame");

        for (i, frame) in frames.iter().enumerate() {
            let flat = frame.flatten_all().unwrap().to_vec1::<f32>().unwrap();

            // ImageNet normalization: (pixel/255 - mean) / std
            // Min possible: (0/255 - 0.485) / 0.229 ≈ -2.12
            // Max possible: (255/255 - 0.406) / 0.225 ≈ 2.64
            for &val in &flat {
                assert!(
                    (-3.0..=3.0).contains(&val),
                    "frame {} has out-of-range normalized value: {}",
                    i,
                    val
                );
            }
        }
    }

    #[test]
    fn test_frames_differ_between_seconds() {
        let video = get_test_video();
        let frames = unsafe { extract_video_frames(&video).unwrap() };
        assert!(
            frames.len() >= 2,
            "need at least 2 frames to compare, got {}",
            frames.len()
        );

        // Frames from different seconds of a real video should not be identical
        let f0 = frames[0].flatten_all().unwrap().to_vec1::<f32>().unwrap();
        let f1 = frames[1].flatten_all().unwrap().to_vec1::<f32>().unwrap();
        assert_eq!(f0.len(), f1.len());

        let diff: f32 = f0.iter().zip(f1.iter()).map(|(a, b)| (a - b).abs()).sum();
        assert!(
            diff > 0.0,
            "frames at 0s and 1s should differ in a real video"
        );
    }

    #[test]
    fn test_label_video_with_model() {
        let video = get_test_video();
        let labels = label_video(&video, None, None).unwrap();

        assert!(!labels.is_empty(), "should produce at least one label");

        // All confidence scores should be between 0 and 1
        for (label, score) in &labels {
            assert!(
                *score >= MIN_CONFIDENCE && *score <= 1.0,
                "label '{}' has invalid confidence: {}",
                label,
                score
            );
        }

        println!("Video labels: {:?}", labels);
    }
}
