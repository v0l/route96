use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Error, Result};
use async_openai::{
    Client,
    config::OpenAIConfig,
    types::chat::{
        ChatCompletionRequestMessage, ChatCompletionRequestMessageContentPartImageArgs,
        ChatCompletionRequestUserMessageArgs, ChatCompletionRequestUserMessageContent,
        ChatCompletionRequestUserMessageContentPart, CreateChatCompletionRequest, ImageDetail,
        ImageUrlArgs,
    },
};
use base64::Engine;
use candle_core::{D, DType, Device, IndexOp, Tensor};
use candle_nn::{Module, VarBuilder};
use candle_transformers::models::{siglip, vit};
use hf_hub::api::sync::{Api, ApiBuilder};
use log::{debug, info, warn};
use nostr::serde_json;
use serde::Deserialize;

/// Minimum confidence threshold for a label to be included
/// Increased from 0.25 to 0.4 for videos to reduce low-quality labels
pub const MIN_CONFIDENCE: f32 = 0.4;

/// Maximum number of frames to sample from a video (max 10 frames, spread over entire file)
pub use crate::media::protocol::MAX_VIDEO_FRAMES;

/// Media smaller than this on either axis is not worth classifying: the model
/// input is 224x224, so a degenerate strip (e.g. 900x1) only yields garbage
/// labels or upsets the scaler.
pub use crate::media::protocol::MIN_LABEL_DIMENSION;

// ── Labeling error type ────────────────────────────────────────────────────

/// Error type for labeling operations.
///
/// Errors are classified as either **permanent** (the file itself is the
/// problem and will never succeed) or **transient** (external factors like
/// network failures that may resolve on retry).
#[derive(Debug)]
pub enum LabelError {
    // ── Permanent: file-level problems ──────────────────────────────────
    /// The file does not exist on disk.
    FileNotFound(PathBuf),
    /// The file path contains non-UTF-8 characters.
    InvalidPath,
    /// ffmpeg could not find a video/image stream in the file.
    NoMediaStream,
    /// ffmpeg decoded packets but produced no usable image data.
    NoImageData,
    /// The media is too small on at least one axis to label usefully.
    TooSmall { width: u32, height: u32 },
    /// The file appears corrupt: ffmpeg exceeded the packet limit without
    /// producing a decodable frame.
    CorruptMedia { detail: String },
    /// An I/O error occurred reading or writing a temporary file during
    /// media processing.
    TempFileIo(String),
    /// A local ffmpeg/candle operation (demux, decode, scale, encode, tensor)
    /// failed. These are permanent because they indicate the file is unreadable.
    Ffmpeg(Error),

    // ── Transient: external service problems ────────────────────────────
    /// The remote API request timed out.
    ApiTimeout,
    /// The remote API could not be reached (network, DNS, connection reset).
    ApiFailed(String),
    /// The remote API answered with an error, for example an image it refused.
    ApiRejected(String),
    /// The remote API returned a response with no usable content.
    /// Treated as transient because the model may produce content on retry.
    ApiEmptyResponse,

    // ── Permanent: programming / configuration errors ───────────────────
    /// Failed to construct the API request (bad URL, missing fields, etc.).
    RequestBuild(String),
}

impl LabelError {
    /// Returns `true` if this error is transient and the file should be
    /// retried later rather than permanently marked as done.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::ApiTimeout | Self::ApiFailed(_) | Self::ApiRejected(_) | Self::ApiEmptyResponse
        )
    }

    /// Returns `true` when the file was deliberately skipped rather than
    /// failing, so callers can log it quietly.
    pub fn is_skip(&self) -> bool {
        matches!(self, Self::TooSmall { .. })
    }
}

impl fmt::Display for LabelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileNotFound(p) => write!(f, "File not found: {}", p.display()),
            Self::InvalidPath => write!(f, "Non-UTF-8 file path"),
            Self::NoMediaStream => write!(f, "No video/image stream found"),
            Self::NoImageData => write!(f, "No image data found"),
            Self::TooSmall { width, height } => write!(
                f,
                "Media too small to label: {}x{} (minimum {}px on each axis)",
                width, height, MIN_LABEL_DIMENSION
            ),
            Self::CorruptMedia { detail } => write!(f, "Corrupt media: {}", detail),
            Self::TempFileIo(e) => write!(f, "Temp file I/O error: {}", e),
            Self::Ffmpeg(e) => write!(f, "FFmpeg error: {}", e),
            Self::ApiTimeout => write!(f, "API request timed out"),
            Self::ApiFailed(e) => write!(f, "API request failed: {}", e),
            Self::ApiRejected(e) => write!(f, "API rejected request: {}", e),
            Self::ApiEmptyResponse => write!(f, "No content in API response"),
            Self::RequestBuild(e) => write!(f, "Failed to build request: {}", e),
        }
    }
}

impl std::error::Error for LabelError {}

/// Reject media that is too small on either axis to produce meaningful labels.
pub use crate::media::protocol::{DEFAULT_LLM_MAX_PIXELS, LLM_MAX_HEIGHT, llm_target_size};
const LLM_RETRY_PIXEL_DIVISOR: u64 = 4;

impl From<crate::media::MediaError> for LabelError {
    fn from(e: crate::media::MediaError) -> Self {
        use crate::media::{JobError, MediaError};
        match e {
            MediaError::Job(JobError::NoMediaStream) => LabelError::NoMediaStream,
            MediaError::Job(JobError::NoImageData) => LabelError::NoImageData,
            MediaError::Job(JobError::TooSmall { width, height }) => {
                LabelError::TooSmall { width, height }
            }
            MediaError::Job(JobError::Corrupt { detail }) => LabelError::CorruptMedia { detail },
            other => LabelError::Ffmpeg(Error::msg(other.to_string())),
        }
    }
}

impl From<candle_core::Error> for LabelError {
    fn from(e: candle_core::Error) -> Self {
        LabelError::Ffmpeg(Error::new(e))
    }
}

/// Trait for any media labeling backend (local ViT, external API, etc.).
///
/// Implementations must be `Send` so they can be moved to dedicated worker
/// threads. Each labeler is responsible for a single model / API endpoint.
pub trait MediaLabeler: Send + Sync {
    /// Human-readable name stored in the DB alongside each label (e.g. `"vit224"`).
    fn name(&self) -> &str;

    /// Labels to exclude from this labeler's output (exact match, case-insensitive).
    fn label_exclude(&self) -> &[String];

    /// Minimum confidence threshold for this labeler.
    fn min_confidence(&self) -> f32;

    /// Classify a file on disk and return `(label, confidence)` pairs.
    fn label_file(&self, path: &Path, mime_type: &str) -> Result<HashMap<String, f32>, LabelError>;

    fn moderates(&self) -> bool {
        true
    }
}

#[derive(Deserialize)]
struct MyVitConfig {
    pub id2label: HashMap<usize, String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Normalization {
    #[serde(rename = "image_mean", default = "Normalization::half")]
    mean: [f32; 3],
    #[serde(rename = "image_std", default = "Normalization::half")]
    std: [f32; 3],
}

impl Default for Normalization {
    fn default() -> Self {
        Self {
            mean: Self::half(),
            std: Self::half(),
        }
    }
}

impl Normalization {
    fn half() -> [f32; 3] {
        [0.5; 3]
    }

    fn from_preprocessor_config(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }

    fn apply(
        &self,
        rgb: Vec<u8>,
        width: usize,
        height: usize,
        device: &Device,
    ) -> candle_core::Result<Tensor> {
        let data = Tensor::from_vec(rgb, (height, width, 3), device)?.permute((2, 0, 1))?;
        let mean = Tensor::new(&self.mean, device)?.reshape((3, 1, 1))?;
        let std = Tensor::new(&self.std, device)?.reshape((3, 1, 1))?;
        (data.to_dtype(DType::F32)? / 255.)?
            .broadcast_sub(&mean)?
            .broadcast_div(&std)
    }
}

/// A loaded ViT model ready for inference
pub struct VitModel {
    model: vit::Model,
    label_config: MyVitConfig,
    normalization: Normalization,
    device: Device,
}

impl VitModel {
    /// Load model files from explicit paths.
    fn load(
        model_path: PathBuf,
        config_path: PathBuf,
        normalization: Normalization,
        device: Device,
    ) -> Result<Self> {
        candle_core::cuda::set_gemm_reduced_precision_f32(true);
        info!("Loading ViT model {:?} on {:?}", model_path, device);
        let vb =
            unsafe { VarBuilder::from_mmaped_safetensors(&[model_path], DType::F32, &device)? };
        let config_data = std::fs::read(config_path)?;
        let config: vit::Config = serde_json::from_slice(&config_data)?;
        let label_config: MyVitConfig = serde_json::from_slice(&config_data)?;
        let model = vit::Model::new(&config, label_config.id2label.len(), vb)?;
        Ok(Self {
            model,
            label_config,
            normalization,
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
        let preprocessor_path = model_dir.join("preprocessor_config.json");

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

        if !preprocessor_path.exists() {
            let fetched = ApiBuilder::new()
                .with_cache_dir(models_dir.to_path_buf())
                .build()
                .and_then(|api| {
                    api.model(hf_repo.to_string())
                        .get("preprocessor_config.json")
                });
            match fetched {
                Ok(dl_preprocessor) => {
                    std::fs::copy(&dl_preprocessor, &preprocessor_path)?;
                }
                Err(e) => warn!("No preprocessor_config.json for '{}': {}", hf_repo, e),
            }
        }

        let normalization = Normalization::from_preprocessor_config(&preprocessor_path);
        Self::load(model_path, config_path, normalization, device)
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
    ) -> Result<HashMap<String, f32>, LabelError> {
        if mime_type.starts_with("video/") {
            let frames = extract_video_frames(path, &self.normalization, &self.device)?;
            classify_frames(self, &frames, min_confidence).map_err(LabelError::Ffmpeg)
        } else if mime_type.starts_with("image/svg") {
            Ok(HashMap::new())
        } else {
            let image = load_frame_224(path, &self.normalization, &self.device)?;
            self.classify(&image, min_confidence)
                .map_err(LabelError::Ffmpeg)
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

    fn label_file(&self, path: &Path, mime_type: &str) -> Result<HashMap<String, f32>, LabelError> {
        self.vit.run(path, mime_type, self.min_confidence)
    }
}

pub struct GenericLlmLabeler {
    client: Client<OpenAIConfig>,
    model: String,
    prompt: Option<String>,
    label_exclude: Vec<String>,
    min_confidence: f32,
    name: String,
    max_pixels: u64,
}

impl GenericLlmLabeler {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        api_url: String,
        model: String,
        api_key: Option<String>,
        prompt: Option<String>,
        label_exclude: Vec<String>,
        min_confidence: Option<f32>,
        name: String,
        max_pixels: Option<u64>,
    ) -> Result<Self> {
        let config = OpenAIConfig::new()
            .with_api_base(&api_url)
            .with_api_key(api_key.unwrap_or_default());
        let client = Client::with_config(config);

        Ok(Self {
            client,
            model,
            prompt,
            label_exclude,
            min_confidence: min_confidence.unwrap_or(MIN_CONFIDENCE),
            name,
            max_pixels: max_pixels.unwrap_or(DEFAULT_LLM_MAX_PIXELS),
        })
    }

    fn compress_image_to_jpeg(path: &Path, max_pixels: u64) -> Result<Vec<u8>, LabelError> {
        if !path.exists() {
            return Err(LabelError::FileNotFound(path.to_path_buf()));
        }
        crate::media::llm_jpeg(path, max_pixels).map_err(LabelError::from)
    }

    fn encode_image_to_base64(&self, path: &Path, max_pixels: u64) -> Result<String, LabelError> {
        let bytes = Self::compress_image_to_jpeg(path, max_pixels)?;
        Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
    }

    fn build_messages(
        &self,
        mime_type: &str,
        image_base64: &str,
    ) -> Result<Vec<ChatCompletionRequestMessage>, LabelError> {
        let default_prompt = format!(
            "Analyze this {} image and return labels in format: label_name=confidence_score (one per line, e.g. cat=0.95). Return up to 5 labels, highest confidence first. Only return the labels, no other text.",
            mime_type
        );
        let prompt = if let Some(additional) = &self.prompt {
            format!("{} {}", default_prompt, additional)
        } else {
            default_prompt
        };

        let image_url = ImageUrlArgs::default()
            .url(format!("data:image/jpeg;base64,{}", image_base64))
            .detail(ImageDetail::Auto)
            .build()
            .map_err(|e| LabelError::RequestBuild(format!("Failed to build image URL: {}", e)))?;

        let image_part = ChatCompletionRequestMessageContentPartImageArgs::default()
            .image_url(image_url)
            .build()
            .map_err(|e| LabelError::RequestBuild(format!("Failed to build image part: {}", e)))?;

        let text_part =
            async_openai::types::chat::ChatCompletionRequestMessageContentPartText { text: prompt };

        let content = ChatCompletionRequestUserMessageContent::Array(vec![
            ChatCompletionRequestUserMessageContentPart::ImageUrl(image_part),
            ChatCompletionRequestUserMessageContentPart::Text(text_part),
        ]);

        let message = ChatCompletionRequestUserMessageArgs::default()
            .content(content)
            .build()
            .map_err(|e| LabelError::RequestBuild(format!("Failed to build chat message: {}", e)))?
            .into();

        Ok(vec![message])
    }

    async fn call_api(
        &self,
        image_base64: &str,
        mime_type: &str,
    ) -> Result<HashMap<String, f32>, LabelError> {
        let messages = self.build_messages(mime_type, image_base64)?;

        let request = CreateChatCompletionRequest {
            model: self.model.clone(),
            messages,
            ..Default::default()
        };

        let response: async_openai::types::chat::CreateChatCompletionResponse =
            tokio::time::timeout(
                std::time::Duration::from_secs(120),
                self.client.chat().create(request),
            )
            .await
            .map_err(|_| LabelError::ApiTimeout)?
            .map_err(|e| match e {
                async_openai::error::OpenAIError::Reqwest(_) => {
                    LabelError::ApiFailed(e.to_string())
                }
                _ => LabelError::ApiRejected(e.to_string()),
            })?;

        let content = response
            .choices
            .first()
            .and_then(|c| c.message.content.as_ref())
            .ok_or(LabelError::ApiEmptyResponse)?;

        info!("Prompt response: {:?}", content);

        let mut result = HashMap::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(eq_pos) = line.find('=') {
                let label = line[..eq_pos].trim().to_string();
                let score_str = line[eq_pos + 1..].trim();
                if let Ok(score) = score_str.parse::<f32>() {
                    result.insert(label, score);
                }
            }
        }

        Ok(result)
    }
}

impl GenericLlmLabeler {
    fn request_labels(
        &self,
        rt: &tokio::runtime::Handle,
        path: &Path,
        mime_type: &str,
    ) -> Result<HashMap<String, f32>, LabelError> {
        let image_base64 = self.encode_image_to_base64(path, self.max_pixels)?;
        match rt.block_on(self.call_api(&image_base64, mime_type)) {
            Err(LabelError::ApiRejected(first)) => {
                let reduced = (self.max_pixels / LLM_RETRY_PIXEL_DIVISOR).max(1);
                warn!(
                    "LLM labeler '{}' rejected {} ({}), retrying within {} pixels",
                    self.name,
                    path.display(),
                    first,
                    reduced
                );
                let image_base64 = self.encode_image_to_base64(path, reduced)?;
                rt.block_on(self.call_api(&image_base64, mime_type))
            }
            other => other,
        }
    }
}

impl MediaLabeler for GenericLlmLabeler {
    fn name(&self) -> &str {
        &self.name
    }

    fn label_exclude(&self) -> &[String] {
        &self.label_exclude
    }

    fn min_confidence(&self) -> f32 {
        self.min_confidence
    }

    fn label_file(&self, path: &Path, mime_type: &str) -> Result<HashMap<String, f32>, LabelError> {
        let rt = tokio::runtime::Handle::current();
        let labels = self.request_labels(&rt, path, mime_type)?;

        let filtered: HashMap<String, f32> = labels
            .into_iter()
            .filter(|(label, score)| {
                let lower = label.to_lowercase();
                !self
                    .label_exclude
                    .iter()
                    .any(|ex| ex.to_lowercase() == lower)
                    && *score >= self.min_confidence
            })
            .collect();

        debug!(
            "Generic LLM labeler '{}' produced {} labels for {}",
            self.name,
            filtered.len(),
            path.display()
        );

        Ok(filtered)
    }
}

pub const TAXONOMY_BACKBONE: &str = "google/siglip2-base-patch16-224";
const TAXONOMY_HEAD_FILE: &str = "head.safetensors";
const SAFETENSORS_MAX_HEADER: u64 = 64 * 1024 * 1024;

fn hf_file(models_dir: &Path, repo: &str, file: &str) -> Result<PathBuf> {
    let api: Api = ApiBuilder::new()
        .with_cache_dir(models_dir.to_path_buf())
        .build()?;
    Ok(api.model(repo.to_string()).get(file)?)
}

fn safetensors_metadata(path: &Path) -> Result<HashMap<String, String>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut len = [0u8; 8];
    file.read_exact(&mut len)?;
    let len = u64::from_le_bytes(len);
    if len > SAFETENSORS_MAX_HEADER {
        return Err(Error::msg(format!("safetensors header too large: {len}")));
    }
    let mut header = vec![0u8; len as usize];
    file.read_exact(&mut header)?;
    let header: serde_json::Value = serde_json::from_slice(&header)?;
    Ok(header
        .get("__metadata__")
        .and_then(|m| serde_json::from_value(m.clone()).ok())
        .unwrap_or_default())
}

struct TaxonomyHead {
    linear: candle_nn::Linear,
    classes: Vec<String>,
    thresholds: Vec<f32>,
}

impl TaxonomyHead {
    fn load(path: &Path, hidden_size: usize, device: &Device) -> Result<Self> {
        let classes: Vec<String> = serde_json::from_str(
            safetensors_metadata(path)?
                .get("classes")
                .ok_or_else(|| Error::msg("head has no classes in its metadata"))?,
        )?;
        let mut tensors = candle_core::safetensors::load(path, device)?;
        let mut take = |name: &str| {
            tensors
                .remove(name)
                .ok_or_else(|| Error::msg(format!("head has no '{name}' tensor")))
                .and_then(|t| Ok(t.to_dtype(DType::F32)?))
        };
        let weight = take("weight")?;
        let bias = take("bias")?;
        let thresholds = take("thresholds")?.to_vec1::<f32>()?;
        let n = classes.len();
        if weight.dims() != [n, hidden_size] || bias.dims() != [n] || thresholds.len() != n {
            return Err(Error::msg(format!(
                "head shapes disagree: {} classes, weight {:?}, bias {:?}, {} thresholds, tower width {}",
                n,
                weight.dims(),
                bias.dims(),
                thresholds.len(),
                hidden_size
            )));
        }
        Ok(Self {
            linear: candle_nn::Linear::new(weight, Some(bias)),
            classes,
            thresholds,
        })
    }
}

pub struct TaxonomyModel {
    backbone: siglip::VisionModel,
    head: TaxonomyHead,
    device: Device,
}

impl TaxonomyModel {
    pub fn load(backbone_dir: &Path, head_path: &Path, device: Device) -> Result<Self> {
        Self::load_files(
            &backbone_dir.join("config.json"),
            &backbone_dir.join("model.safetensors"),
            head_path,
            device,
        )
    }

    fn load_files(
        config_path: &Path,
        weights_path: &Path,
        head_path: &Path,
        device: Device,
    ) -> Result<Self> {
        info!(
            "Loading taxonomy model {:?} with head {:?} on {:?}",
            weights_path, head_path, device
        );
        let config: serde_json::Value = serde_json::from_slice(&std::fs::read(config_path)?)?;
        let vision = config.get("vision_config").cloned().unwrap_or(config);
        let cfg: siglip::VisionConfig = serde_json::from_value(vision)?;
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path], DType::F32, &device)?
        };
        let backbone = siglip::VisionModel::new(&cfg, true, vb.pp("vision_model"))?;
        let head = TaxonomyHead::load(head_path, cfg.hidden_size, &device)?;
        Ok(Self {
            backbone,
            head,
            device,
        })
    }

    pub fn resolve(
        models_dir: &Path,
        head_path: Option<&Path>,
        hf_repo: Option<&str>,
        backbone: Option<&str>,
        device: Device,
    ) -> Result<Self> {
        let head = match (head_path, hf_repo) {
            (Some(p), _) => p.to_path_buf(),
            (None, Some(repo)) => hf_file(models_dir, repo, TAXONOMY_HEAD_FILE)?,
            (None, None) => {
                return Err(Error::msg("taxonomy model needs head_path or hf_repo"));
            }
        };
        let backbone = backbone.unwrap_or(TAXONOMY_BACKBONE);
        let local = Path::new(backbone);
        if local.is_dir() {
            return Self::load(local, &head, device);
        }
        let config = hf_file(models_dir, backbone, "config.json")?;
        let weights = hf_file(models_dir, backbone, "model.safetensors")?;
        Self::load_files(&config, &weights, &head, device)
    }

    pub fn classes(&self) -> &[String] {
        &self.head.classes
    }

    fn mean_probabilities(&self, frames: &[Tensor]) -> Result<Vec<f32>> {
        let xs = Tensor::stack(frames, 0)?.to_device(&self.device)?;
        let logits = self.backbone.forward(&xs)?.apply(&self.head.linear)?;
        Ok(candle_nn::ops::sigmoid(&logits)?
            .mean(0)?
            .to_vec1::<f32>()?)
    }

    fn labels_over_threshold(&self, probs: &[f32], min_confidence: f32) -> HashMap<String, f32> {
        self.head
            .classes
            .iter()
            .zip(&self.head.thresholds)
            .zip(probs)
            .filter(|((_, t), p)| **p >= t.max(min_confidence))
            .map(|((c, _), p)| (c.clone(), *p))
            .collect()
    }

    pub fn run(
        &self,
        path: &Path,
        mime_type: &str,
        min_confidence: f32,
    ) -> Result<HashMap<String, f32>, LabelError> {
        let normalization = Normalization::default();
        let frames = if mime_type.starts_with("video/") {
            extract_video_frames(path, &normalization, &Device::Cpu)?
        } else if mime_type.starts_with("image/svg") {
            Vec::new()
        } else {
            vec![load_frame_224(path, &normalization, &Device::Cpu)?]
        };
        if frames.is_empty() {
            return Ok(HashMap::new());
        }
        let probs = self
            .mean_probabilities(&frames)
            .map_err(LabelError::Ffmpeg)?;
        Ok(self.labels_over_threshold(&probs, min_confidence))
    }
}

pub struct TaxonomyLabeler {
    model: TaxonomyModel,
    name: String,
    label_exclude: Vec<String>,
    min_confidence: f32,
}

impl TaxonomyLabeler {
    pub fn new(
        model: TaxonomyModel,
        name: String,
        label_exclude: Vec<String>,
        min_confidence: Option<f32>,
    ) -> Self {
        Self {
            model,
            name,
            label_exclude,
            min_confidence: min_confidence.unwrap_or(0.0),
        }
    }
}

impl MediaLabeler for TaxonomyLabeler {
    fn name(&self) -> &str {
        &self.name
    }

    fn label_exclude(&self) -> &[String] {
        &self.label_exclude
    }

    fn min_confidence(&self) -> f32 {
        self.min_confidence
    }

    fn label_file(&self, path: &Path, mime_type: &str) -> Result<HashMap<String, f32>, LabelError> {
        self.model.run(path, mime_type, self.min_confidence)
    }

    fn moderates(&self) -> bool {
        false
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
) -> Result<HashMap<String, f32>, LabelError> {
    let device = Device::cuda_if_available(0)?;
    VitModel::load_from_dir(models_dir, hf_repo, device)
        .map_err(LabelError::Ffmpeg)?
        .run(path, mime_type, MIN_CONFIDENCE)
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

fn extract_video_frames(
    path: &Path,
    normalization: &Normalization,
    device: &Device,
) -> Result<Vec<Tensor>, LabelError> {
    let size = crate::media::protocol::LABEL_FRAME_SIZE as usize;
    crate::media::label_video_frames(path)?
        .into_iter()
        .map(|rgb| Ok(normalization.apply(rgb, size, size, device)?))
        .collect()
}

fn load_frame_224(
    path: &Path,
    normalization: &Normalization,
    device: &Device,
) -> Result<Tensor, LabelError> {
    let size = crate::media::protocol::LABEL_FRAME_SIZE as usize;
    let pic = crate::media::label_image_rgb(path)?;
    Ok(normalization.apply(pic, size, size, device)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    const DEFAULT_HF_REPO: &str = "google/vit-base-patch16-224";

    fn get_test_video() -> PathBuf {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/sample.mp4");
        assert!(path.exists(), "test video missing: {path:?}");
        path
    }

        #[test]
    fn test_extract_frames_seeks_without_full_decode() {
        let path = std::env::temp_dir().join("route96_test_synthetic.mp4");
        let status = Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=duration=20:size=320x240:rate=25",
                "-c:v",
                "libx264",
                "-g",
                "25",
            ])
            .arg(&path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("ffmpeg must be installed");
        assert!(status.success());
        let frames = extract_video_frames(&path, &Normalization::default(), &Device::Cpu).unwrap();
        assert_eq!(frames.len(), MAX_VIDEO_FRAMES);
        let a = frames[0].flatten_all().unwrap().to_vec1::<f32>().unwrap();
        let b = frames[MAX_VIDEO_FRAMES - 1]
            .flatten_all()
            .unwrap()
            .to_vec1::<f32>()
            .unwrap();
        assert_ne!(a, b);
    }

    fn test_models_dir() -> PathBuf {
        std::env::temp_dir().join("route96_test_models")
    }

    #[test]
    fn test_llm_target_size_caps_pixels_and_height() {
        assert_eq!(
            llm_target_size(800, 600, DEFAULT_LLM_MAX_PIXELS),
            (800, 600)
        );
        assert_eq!(
            llm_target_size(1000, 2000, DEFAULT_LLM_MAX_PIXELS),
            (512, 1024)
        );
        for (w, h) in [
            (2163u32, 1024u32),
            (2527, 1024),
            (2688, 1024),
            (1500, 1500),
            (4000, 3000),
            (9000, 600),
        ] {
            let (ow, oh) = llm_target_size(w, h, DEFAULT_LLM_MAX_PIXELS);
            assert!(
                ow as u64 * oh as u64 <= DEFAULT_LLM_MAX_PIXELS,
                "{w}x{h} -> {ow}x{oh}"
            );
            assert!(oh <= LLM_MAX_HEIGHT, "{w}x{h} -> {ow}x{oh}");
            assert!(ow % 2 == 0 && oh % 2 == 0 && ow >= 2 && oh >= 2);
        }
    }

        #[test]
    fn test_extract_frames_from_real_video() {
        let video = get_test_video();
        let frames = extract_video_frames(&video, &Normalization::default(), &Device::Cpu).unwrap();

        // 5-second clip should yield up to 10 frames (one every 0.5s)
        // But the test video is very short (5s) with few frames, so we get at most 10
        assert!(
            !frames.is_empty() && frames.len() <= MAX_VIDEO_FRAMES,
            "expected >0 and <= {} frames, got {}",
            MAX_VIDEO_FRAMES,
            frames.len()
        );

        // Each frame should be a normalized 224x224 RGB tensor
        for (i, frame) in frames.iter().enumerate() {
            assert_eq!(frame.dims(), &[3, 224, 224], "frame {} has wrong shape", i);
            assert_eq!(frame.dtype(), DType::F32, "frame {} should be F32", i);
        }
    }

    #[test]
    fn test_tensor_values_normalized() {
        let video = get_test_video();
        let frames = extract_video_frames(&video, &Normalization::default(), &Device::Cpu).unwrap();
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
        let frames = extract_video_frames(&video, &Normalization::default(), &Device::Cpu).unwrap();
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
    fn test_normalization_from_preprocessor_config() {
        let dir = std::env::temp_dir().join(format!("route96_norm_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("preprocessor_config.json");

        assert_eq!(
            Normalization::from_preprocessor_config(&path),
            Normalization::default()
        );

        std::fs::write(
            &path,
            r#"{"image_mean":[0.485,0.456,0.406],"image_std":[0.229,0.224,0.225],"size":224}"#,
        )
        .unwrap();
        let n = Normalization::from_preprocessor_config(&path);
        assert_eq!(n.mean, [0.485, 0.456, 0.406]);
        assert_eq!(n.std, [0.229, 0.224, 0.225]);

        std::fs::write(&path, r#"{"do_resize":true}"#).unwrap();
        assert_eq!(
            Normalization::from_preprocessor_config(&path),
            Normalization::default()
        );

        let t = n.apply(vec![255, 0, 128], 1, 1, &Device::Cpu).unwrap();
        let v = t.flatten_all().unwrap().to_vec1::<f32>().unwrap();
        assert!((v[0] - (1.0 - 0.485) / 0.229).abs() < 1e-4);
        assert!((v[1] - (0.0 - 0.456) / 0.224).abs() < 1e-4);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_nsfw_model_does_not_flag_glass_sculpture() {
        let image =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/glass_sculpture.webp");
        let labels = label_file(
            &image,
            "image/webp",
            &test_models_dir(),
            "v0l/route96-nsfw-vit224",
        )
        .unwrap();
        assert!(!labels.contains_key("nsfw"), "{labels:?}");
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

    fn write_safetensors(
        path: &Path,
        tensors: &[(&str, Vec<usize>, Vec<f32>)],
        metadata: &[(&str, String)],
    ) {
        let mut header = serde_json::Map::new();
        let meta: serde_json::Map<String, serde_json::Value> = metadata
            .iter()
            .map(|(k, v)| (k.to_string(), serde_json::Value::String(v.clone())))
            .collect();
        header.insert("__metadata__".to_string(), serde_json::Value::Object(meta));
        let mut data = Vec::new();
        for (name, shape, values) in tensors {
            let start = data.len();
            data.extend(values.iter().flat_map(|v| v.to_le_bytes()));
            header.insert(
                name.to_string(),
                serde_json::json!({"dtype": "F32", "shape": shape, "data_offsets": [start, data.len()]}),
            );
        }
        let mut header = serde_json::to_vec(&header).unwrap();
        while header.len() % 8 != 0 {
            header.push(b' ');
        }
        let mut out = (header.len() as u64).to_le_bytes().to_vec();
        out.extend(header);
        out.extend(data);
        std::fs::write(path, out).unwrap();
    }

    const TINY_HIDDEN: usize = 8;

    fn tiny_taxonomy(dir: &Path, biases: &[(&str, f32, f32)]) -> (PathBuf, PathBuf) {
        let backbone = dir.join("backbone");
        std::fs::create_dir_all(&backbone).unwrap();
        let config = serde_json::json!({
            "vision_config": {
                "hidden_size": TINY_HIDDEN,
                "intermediate_size": 16,
                "num_hidden_layers": 1,
                "num_attention_heads": 2,
                "image_size": 224,
                "patch_size": 16
            }
        });
        std::fs::write(backbone.join("config.json"), config.to_string()).unwrap();
        let cfg: siglip::VisionConfig =
            serde_json::from_value(config["vision_config"].clone()).unwrap();
        let varmap = candle_nn::VarMap::new();
        let vb = VarBuilder::from_varmap(&varmap, DType::F32, &Device::Cpu);
        siglip::VisionModel::new(&cfg, true, vb.pp("vision_model")).unwrap();
        varmap.save(backbone.join("model.safetensors")).unwrap();

        let n = biases.len();
        let names: Vec<&str> = biases.iter().map(|(c, _, _)| *c).collect();
        let head = dir.join("head.safetensors");
        write_safetensors(
            &head,
            &[
                ("weight", vec![n, TINY_HIDDEN], vec![0.0; n * TINY_HIDDEN]),
                ("bias", vec![n], biases.iter().map(|(_, b, _)| *b).collect()),
                ("thresholds", vec![n], biases.iter().map(|(_, _, t)| *t).collect()),
            ],
            &[
                ("classes", serde_json::to_string(&names).unwrap()),
                ("base", TAXONOMY_BACKBONE.to_string()),
            ],
        );
        (backbone, head)
    }

    fn sigmoid(x: f32) -> f32 {
        1.0 / (1.0 + (-x).exp())
    }

    #[test]
    fn taxonomy_model_loads_and_labels_classes_over_their_thresholds() {
        let dir = tempfile::tempdir().unwrap();
        let (backbone, head) = tiny_taxonomy(
            dir.path(),
            &[("cat", 3.0, 0.5), ("dog", -3.0, 0.5), ("sky", 0.0, 0.4), ("nude", 0.0, 0.6)],
        );
        let model = TaxonomyModel::load(&backbone, &head, Device::Cpu).unwrap();
        assert_eq!(model.classes(), ["cat", "dog", "sky", "nude"]);
        let labeler = TaxonomyLabeler::new(model, "taxonomy-v1".to_string(), vec![], None);
        assert_eq!(labeler.name(), "taxonomy-v1");
        assert!(!labeler.moderates());

        let image =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/glass_sculpture.webp");
        let labels = labeler.label_file(&image, "image/webp").unwrap();
        let mut names: Vec<&String> = labels.keys().collect();
        names.sort();
        assert_eq!(names, ["cat", "sky"]);
        assert!((labels["cat"] - sigmoid(3.0)).abs() < 1e-5, "{labels:?}");
        assert!((labels["sky"] - 0.5).abs() < 1e-5, "{labels:?}");
    }

    #[test]
    fn taxonomy_min_confidence_raises_the_class_thresholds() {
        let dir = tempfile::tempdir().unwrap();
        let (backbone, head) =
            tiny_taxonomy(dir.path(), &[("cat", 3.0, 0.5), ("sky", 0.0, 0.4)]);
        let model = TaxonomyModel::load(&backbone, &head, Device::Cpu).unwrap();
        let labeler = TaxonomyLabeler::new(model, "t".to_string(), vec![], Some(0.6));
        let image =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/glass_sculpture.webp");
        let labels = labeler.label_file(&image, "image/webp").unwrap();
        assert_eq!(labels.keys().collect::<Vec<_>>(), ["cat"]);
        assert!(labeler.label_file(&image, "image/svg+xml").unwrap().is_empty());
    }

    #[test]
    fn taxonomy_head_that_disagrees_with_the_tower_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (backbone, _) = tiny_taxonomy(dir.path(), &[("cat", 0.0, 0.5)]);
        let bad = dir.path().join("bad.safetensors");
        write_safetensors(
            &bad,
            &[
                ("weight", vec![2, TINY_HIDDEN], vec![0.0; 2 * TINY_HIDDEN]),
                ("bias", vec![2], vec![0.0; 2]),
                ("thresholds", vec![2], vec![0.5; 2]),
            ],
            &[("classes", r#"["cat"]"#.to_string())],
        );
        let err = TaxonomyModel::load(&backbone, &bad, Device::Cpu)
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("disagree"), "{err}");
        let no_classes = dir.path().join("no_classes.safetensors");
        write_safetensors(&no_classes, &[("bias", vec![1], vec![0.0])], &[]);
        assert!(TaxonomyModel::load(&backbone, &no_classes, Device::Cpu).is_err());
        assert!(
            TaxonomyModel::resolve(dir.path(), None, None, None, Device::Cpu).is_err(),
            "a taxonomy entry without a head must not load"
        );
    }

    #[test]
    fn taxonomy_resolve_takes_a_local_backbone_and_head_path() {
        let dir = tempfile::tempdir().unwrap();
        let (backbone, head) = tiny_taxonomy(dir.path(), &[("cat", 1.0, 0.5)]);
        let model = TaxonomyModel::resolve(
            dir.path(),
            Some(&head),
            None,
            Some(backbone.to_str().unwrap()),
            Device::Cpu,
        )
        .unwrap();
        assert_eq!(model.classes(), ["cat"]);
    }

    #[test]
    fn taxonomy_video_averages_frame_probabilities() {
        let dir = tempfile::tempdir().unwrap();
        let (backbone, head) = tiny_taxonomy(dir.path(), &[("cat", 2.0, 0.5)]);
        let model = TaxonomyModel::load(&backbone, &head, Device::Cpu).unwrap();
        let labels = model.run(&get_test_video(), "video/mp4", 0.0).unwrap();
        assert!((labels["cat"] - sigmoid(2.0)).abs() < 1e-5, "{labels:?}");
    }

    #[test]
    #[ignore = "needs ROUTE96_TAXONOMY_HEAD (head.safetensors from training/export_head.py)"]
    fn taxonomy_v1_labels_the_glass_sculpture() {
        let head = PathBuf::from(std::env::var("ROUTE96_TAXONOMY_HEAD").unwrap());
        let backbone = std::env::var("ROUTE96_TAXONOMY_BACKBONE").ok();
        let model = TaxonomyModel::resolve(
            &test_models_dir(),
            Some(&head),
            None,
            backbone.as_deref(),
            Device::Cpu,
        )
        .unwrap();
        let labeler = TaxonomyLabeler::new(model, "taxonomy-v1".to_string(), vec![], None);
        let image =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-data/glass_sculpture.webp");
        let labels = labeler.label_file(&image, "image/webp").unwrap();
        println!("taxonomy-v1 labels: {labels:?}");
        for expected in ["statue", "abstract"] {
            assert!(labels.contains_key(expected), "{labels:?}");
        }
        for adult in ["nude", "breast", "lingerie", "sex"] {
            assert!(!labels.contains_key(adult), "{labels:?}");
        }
        assert!(labels.values().all(|p| (0.0..=1.0).contains(p)));
    }
}
