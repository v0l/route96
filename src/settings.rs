#[cfg(feature = "payments")]
use crate::payments::{Currency, PaymentAmount, PaymentInterval, PaymentUnit};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Listen addr:port
    pub listen: Option<String>,

    /// Directory to store files
    pub storage_dir: String,

    /// Database connection string mysql://localhost
    pub database: String,

    /// Maximum support filesize for uploading
    pub max_upload_bytes: u64,

    /// Public facing url
    pub public_url: String,

    /// Whitelisted pubkeys
    pub whitelist: Option<Vec<String>>,

    /// Path to a file containing whitelisted pubkeys (one hex key per line).
    /// When set, the server will monitor this file and reload it if it changes.
    pub whitelist_file: Option<PathBuf>,

    /// Directory where HuggingFace models are cached / loaded from.
    /// Defaults to `<storage_dir>/models` when not set.
    #[cfg(feature = "labels")]
    pub models_dir: Option<PathBuf>,

    /// Label models to run on every uploaded file.
    /// Each entry describes one ViT model; all results are merged into the
    /// file's label set.  When this list is empty or absent, no labeling runs.
    #[cfg(feature = "labels")]
    pub label_models: Option<Vec<LabelModelConfig>>,

    /// Label terms that trigger automatic LabelFlagged review state.
    /// Any file whose AI labels contain a substring matching one of these
    /// terms (case-insensitive) will have its review_state set to LabelFlagged.
    #[cfg(feature = "labels")]
    pub label_flag_terms: Option<Vec<String>>,

    /// Webhook api endpoint
    pub webhook_url: Option<String>,

    /// Analytics tracking
    pub plausible_url: Option<String>,

    /// Path to void.cat uploads (files-v2)
    pub void_cat_files: Option<PathBuf>,

    /// Reject image uploads containing sensitive EXIF metadata (GPS, device info)
    #[cfg(feature = "blossom")]
    pub reject_sensitive_exif: Option<bool>,

    #[cfg(feature = "payments")]
    /// Payment options for paid storage
    pub payments: Option<PaymentConfig>,
}

/// Configuration for a single ViT labeling model.
#[cfg(feature = "labels")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelModelConfig {
    /// HuggingFace repo id (e.g. `"google/vit-base-patch16-224"`).
    /// The model files are downloaded into `models_dir` on first use and
    /// reused on subsequent runs.
    pub hf_repo: String,

    /// Human-readable name stored alongside each label this model produces
    /// (e.g. `"vit224"`, `"nsfw-detector"`).
    pub name: String,

    /// Labels to discard from this model's output (exact match, case-insensitive).
    /// Use this to suppress noise labels that are meaningless for a given model,
    /// e.g. `["normal"]` for the Falconsai NSFW detector.
    #[serde(default)]
    pub label_exclude: Vec<String>,

    /// Minimum confidence score for a label to be stored.
    /// Overrides the global default (0.25) for this model.
    pub min_confidence: Option<f32>,
}

#[cfg(feature = "payments")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentConfig {
    /// LND connection details
    pub lnd: LndConfig,

    /// Pricing per unit
    pub cost: PaymentAmount,

    /// What metric to bill payments on
    pub unit: PaymentUnit,

    /// Billing interval time per unit
    pub interval: PaymentInterval,

    /// Fiat base currency to store exchange rates along with invoice
    pub fiat: Option<Currency>,

    /// Free quota in bytes for users without payments (default: 100MB)
    pub free_quota_bytes: Option<u64>,
}

#[cfg(feature = "payments")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LndConfig {
    pub endpoint: String,
    pub tls: PathBuf,
    pub macaroon: PathBuf,
}
