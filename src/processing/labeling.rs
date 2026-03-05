use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::slice;

use anyhow::{Error, Result};
use candle_core::{D, DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::vit;
use ffmpeg_rs_raw::ffmpeg_sys_the_third::AVPixelFormat::AV_PIX_FMT_RGB24;
use ffmpeg_rs_raw::{Decoder, Demuxer, Scaler};
use hf_hub::api::sync::{Api, ApiBuilder};
use log::{debug, info};
use nostr::serde_json;
use serde::Deserialize;

/// Minimum confidence threshold for a label to be included
pub const MIN_CONFIDENCE: f32 = 0.25;

/// Maximum number of frames to sample from a video (1 per second, up to 60s)
const MAX_VIDEO_FRAMES: usize = 60;

/// Trait for any media labeling backend (local ViT, external API, etc.).
///
/// Implementations must be `Send` so they can be moved to dedicated worker
/// threads. Each labeler is responsible for a single model / API endpoint.
pub trait MediaLabeler: Send {
    /// Human-readable name stored in the DB alongside each label (e.g. `"vit224"`).
    fn name(&self) -> &str;

    /// Labels to exclude from this labeler's output (exact match, case-insensitive).
    fn label_exclude(&self) -> &[String];

    /// Minimum confidence threshold for this labeler.
    fn min_confidence(&self) -> f32;

    /// Classify a file on disk and return `(label, confidence)` pairs.
    fn label_file(&self, path: &Path, mime_type: &str) -> Result<HashMap<String, f32>>;
}

#[derive(Deserialize)]
struct MyVitConfig {
    pub id2label: HashMap<usize, String>,
}

/// A loaded ViT model ready for inference
pub struct VitModel {
    model: vit::Model,
    label_config: MyVitConfig,
    device: Device,
}

impl VitModel {
    /// Load model files from explicit paths.
    fn load(model_path: PathBuf, config_path: PathBuf, device: Device) -> Result<Self> {
        candle_core::cuda::set_gemm_reduced_precision_f32(true);
        info!("Loading ViT model {:?} on {:?}", model_path, &device);
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

    /// Load (or download) a model from HuggingFace, caching files under `models_dir/<hf_repo>`.
    ///
    /// `models_dir` is the root directory for all cached models.
    /// `hf_repo` is a HuggingFace repo id such as `"google/vit-base-patch16-224"`.
    pub fn load_from_dir(models_dir: &Path, hf_repo: &str, device: Device) -> Result<Self> {
        // Replace `/` in the repo id so it is safe to use as a directory name.
        let cache_subdir = hf_repo.replace('/', "--");
        let model_dir = models_dir.join(&cache_subdir);
        std::fs::create_dir_all(&model_dir)?;

        let model_path = model_dir.join("model.safetensors");
        let config_path = model_dir.join("config.json");

        if !model_path.exists() || !config_path.exists() {
            info!("Downloading ViT model '{}' into {:?}", hf_repo, model_dir);
            let api: Api = ApiBuilder::new()
                .with_cache_dir(models_dir.to_path_buf())
                .build()?;
            let repo = api.model(hf_repo.to_string());
            let dl_model = repo.get("model.safetensors")?;
            let dl_config = repo.get("config.json")?;
            // Copy from the hf-hub cache location into our named directory so
            // subsequent loads are path-stable.
            std::fs::copy(&dl_model, &model_path)?;
            std::fs::copy(&dl_config, &config_path)?;
        }

        Self::load(model_path, config_path, device)
    }

    /// Normalise a raw label string from the model's id2label map.
    /// Many ImageNet-style labels contain comma-separated synonyms
    /// (e.g. `"miniskirt, mini"`); keep only the first token so stored
    /// values are clean single terms that can be queried by exact match.
    fn normalise_label(raw: &str) -> String {
        raw.split(',').next().unwrap_or(raw).trim().to_string()
    }

    /// Run inference on a 224x224 RGB tensor, returns top labels above `min_confidence`.
    fn classify(&self, image: &Tensor, min_confidence: f32) -> Result<HashMap<String, f32>> {
        let image = image.to_device(&self.device)?;
        let logits = self.model.forward(&image.unsqueeze(0)?)?;
        let prs = candle_nn::ops::softmax(&logits, D::Minus1)?
            .i(0)?
            .to_vec1::<f32>()?;
        let mut prs = prs.iter().enumerate().collect::<Vec<_>>();
        prs.sort_by(|(_, p1), (_, p2)| p2.total_cmp(p1));
        let res: HashMap<String, f32> = prs
            .iter()
            .filter(|&(_c, q)| **q >= min_confidence)
            .take(5)
            .map(|&(c, q)| (Self::normalise_label(&self.label_config.id2label[&c]), *q))
            .collect();
        Ok(res)
    }
}

impl VitModel {
    /// Run this model against a file on disk.
    /// Videos are sampled at 1 frame/second; images are classified directly.
    /// `min_confidence` overrides the global `MIN_CONFIDENCE` default.
    pub fn run(
        &self,
        path: &Path,
        mime_type: &str,
        min_confidence: f32,
    ) -> Result<HashMap<String, f32>> {
        if mime_type.starts_with("video/") {
            let frames = unsafe { extract_video_frames(path, &self.device)? };
            classify_frames(self, &frames, min_confidence)
        } else {
            let image = unsafe { load_frame_224(path, &self.device)? };
            self.classify(&image, min_confidence)
        }
    }
}

/// A [`VitModel`] wrapped with its configuration so it implements [`MediaLabeler`].
pub struct VitLabeler {
    vit: VitModel,
    model_name: String,
    label_exclude: Vec<String>,
    min_confidence: f32,
}

impl VitLabeler {
    /// Load a ViT labeler from a HuggingFace repo, caching under `models_dir`.
    pub fn load(
        models_dir: &Path,
        hf_repo: &str,
        model_name: String,
        label_exclude: Vec<String>,
        min_confidence: Option<f32>,
        device: Device,
    ) -> Result<Self> {
        let vit = VitModel::load_from_dir(models_dir, hf_repo, device)?;
        Ok(Self {
            vit,
            model_name,
            label_exclude,
            min_confidence: min_confidence.unwrap_or(MIN_CONFIDENCE),
        })
    }
}

impl MediaLabeler for VitLabeler {
    fn name(&self) -> &str {
        &self.model_name
    }

    fn label_exclude(&self) -> &[String] {
        &self.label_exclude
    }

    fn min_confidence(&self) -> f32 {
        self.min_confidence
    }

    fn label_file(&self, path: &Path, mime_type: &str) -> Result<HashMap<String, f32>> {
        self.vit.run(path, mime_type, self.min_confidence)
    }
}

/// Convenience function: load a model from disk/HF and label one file.
/// Prefer loading models once with [`VitModel::load_from_dir`] and calling
/// [`VitModel::run`] directly when processing multiple files.
pub fn label_file(
    path: &Path,
    mime_type: &str,
    models_dir: &Path,
    hf_repo: &str,
) -> Result<HashMap<String, f32>> {
    let device = Device::cuda_if_available(0)?;
    VitModel::load_from_dir(models_dir, hf_repo, device)?.run(path, mime_type, MIN_CONFIDENCE)
}

/// Classify a sequence of pre-decoded frame tensors, averaging scores per label.
fn classify_frames(
    vit: &VitModel,
    frames: &[Tensor],
    min_confidence: f32,
) -> Result<HashMap<String, f32>> {
    if frames.is_empty() {
        return Ok(HashMap::new());
    }

    let mut label_acc: HashMap<String, (f32, u32)> = HashMap::new();
    for tensor in frames {
        match vit.classify(tensor, min_confidence) {
            Ok(labels) => {
                for (label, score) in labels {
                    let entry = label_acc.entry(label).or_insert((0.0, 0));
                    entry.0 += score;
                    entry.1 += 1;
                }
            }
            Err(e) => {
                debug!("Failed to classify frame: {}", e);
            }
        }
    }

    let result: HashMap<String, f32> = label_acc
        .into_iter()
        .map(|(label, (total, count))| (label, total / count as f32))
        .filter(|(_, avg)| *avg >= min_confidence)
        .collect();

    debug!("{} frames sampled", frames.len());
    Ok(result)
}

/// Extract up to MAX_VIDEO_FRAMES frames from a video, one per second.
unsafe fn extract_video_frames(path: &Path, device: &Device) -> Result<Vec<Tensor>> {
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

        if pkt.stream_index != stream_index {
            continue;
        }

        let decoded = decoder.decode_pkt(Some(&pkt))?;
        for (frame, _) in decoded {
            let pts_sec = frame.pts as f64 * time_base_num / time_base_den;

            if pts_sec >= MAX_VIDEO_FRAMES as f64 {
                return Ok(frames);
            }

            if pts_sec >= next_sample_sec {
                next_sample_sec = pts_sec.floor() + 1.0;

                match unsafe { frame_to_tensor(&frame, &mut scaler, device) } {
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
        if pts_sec >= next_sample_sec
            && let Ok(tensor) = unsafe { frame_to_tensor(&frame, &mut scaler, device) }
        {
            frames.push(tensor);
        }
    }

    Ok(frames)
}

/// Scale a decoded video frame to 224x224 RGB and convert to a normalized tensor.
unsafe fn frame_to_tensor(
    frame: &ffmpeg_rs_raw::AvFrameRef,
    scaler: &mut Scaler,
    device: &Device,
) -> Result<Tensor> {
    let scaled = scaler.process_frame(frame, 224, 224, AV_PIX_FMT_RGB24)?;
    let width = 224usize;
    let height = 224usize;

    let mut dst_vec = Vec::with_capacity(3 * width * height);
    for row in 0..height {
        let line_size = scaled.linesize[0] as usize;
        let row_offset = line_size * row;
        let row_slice = unsafe { slice::from_raw_parts(scaled.data[0].add(row_offset), 3 * width) };
        dst_vec.extend_from_slice(row_slice);
    }

    let data = Tensor::from_vec(dst_vec, (224, 224, 3), device)?.permute((2, 0, 1))?;
    let mean = Tensor::new(&[0.485f32, 0.456, 0.406], device)?.reshape((3, 1, 1))?;
    let std = Tensor::new(&[0.229f32, 0.224, 0.225], device)?.reshape((3, 1, 1))?;
    let res = (data.to_dtype(DType::F32)? / 255.)?
        .broadcast_sub(&mean)?
        .broadcast_div(&std)?;
    Ok(res)
}

/// Load an image from disk, decode and scale it to `width × height` RGB pixels.
unsafe fn load_image(path_buf: &Path, width: usize, height: usize) -> Result<Vec<u8>> {
    let mut demux = Demuxer::new(path_buf.to_str().unwrap())?;
    let info = unsafe { demux.probe_input()? };
    let image_stream = info
        .best_video()
        .ok_or(Error::msg("No image stream found"))?;

    let mut decoder = Decoder::new();
    decoder.setup_decoder(image_stream, None)?;

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
                let line_size = new_frame.linesize[0] as usize;
                let row_offset = line_size * row;
                let row_slice =
                    unsafe { slice::from_raw_parts(new_frame.data[0].add(row_offset), 3 * width) };
                dst_vec.extend_from_slice(row_slice);
            }
            return Ok(dst_vec);
        }
    }
    Err(Error::msg("No image data found"))
}

// https://github.com/huggingface/candle/blob/main/candle-examples/src/imagenet.rs
unsafe fn load_frame_224(path: &Path, device: &Device) -> Result<Tensor> {
    let pic = unsafe { load_image(path, 224, 224)? };

    let data = Tensor::from_vec(pic, (224, 224, 3), device)?.permute((2, 0, 1))?;
    let mean = Tensor::new(&[0.485f32, 0.456, 0.406], device)?.reshape((3, 1, 1))?;
    let std = Tensor::new(&[0.229f32, 0.224, 0.225], device)?.reshape((3, 1, 1))?;
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
    const DEFAULT_HF_REPO: &str = "google/vit-base-patch16-224";

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

    fn test_models_dir() -> PathBuf {
        std::env::temp_dir().join("route96_test_models")
    }

    #[test]
    fn test_extract_frames_from_real_video() {
        let video = get_test_video();
        let frames = unsafe { extract_video_frames(&video, &Device::Cpu).unwrap() };

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
        let frames = unsafe { extract_video_frames(&video, &Device::Cpu).unwrap() };
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
        let frames = unsafe { extract_video_frames(&video, &Device::Cpu).unwrap() };
        assert!(
            frames.len() >= 2,
            "need at least 2 frames to compare, got {}",
            frames.len()
        );

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
    fn test_label_file_video_with_model() {
        let video = get_test_video();
        let models_dir = test_models_dir();
        let labels = label_file(&video, "video/mp4", &models_dir, DEFAULT_HF_REPO).unwrap();

        assert!(!labels.is_empty(), "should produce at least one label");

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

    #[test]
    fn test_load_from_dir_caches_model() {
        let models_dir = test_models_dir();
        // First load (may download)
        VitModel::load_from_dir(&models_dir, DEFAULT_HF_REPO, Device::Cpu)
            .expect("first load should succeed");

        let cache_path = models_dir
            .join(DEFAULT_HF_REPO.replace('/', "--"))
            .join("model.safetensors");
        assert!(cache_path.exists(), "model should be cached on disk");

        // Second load should hit the cache (no network needed)
        VitModel::load_from_dir(&models_dir, DEFAULT_HF_REPO, Device::Cpu)
            .expect("second load from cache should succeed");
    }
}
