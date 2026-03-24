#[cfg(feature = "payments")]
use crate::payments::{Currency, PaymentAmount, PaymentInterval, PaymentUnit};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use std::path::PathBuf;

/// How the server determines which pubkeys are allowed to upload.
///
/// Configured via the `whitelist` key in `config.yaml`.
/// When the key is absent the server is open to everyone.
///
/// ```yaml
/// # Database-backed list managed via the admin UI
/// whitelist: true
///
/// # Static inline list of hex pubkeys
/// whitelist:
///   - "aabbcc..."
///   - "ddeeff..."
///
/// # Hot-reloaded file (one hex pubkey per line, # comments ignored)
/// whitelist: "/etc/route96/whitelist.txt"
/// ```
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum WhitelistMode {
    /// Pubkeys are managed at runtime via the admin UI and stored in the database.
    Database,
    /// A static list of hex pubkeys embedded directly in the config.
    Static(Vec<String>),
    /// Path to a plain-text file of hex pubkeys (one per line).
    /// The server watches this file and hot-reloads it on change.
    File(PathBuf),
}

impl<'de> Deserialize<'de> for WhitelistMode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct WhitelistModeVisitor;

        impl<'de> Visitor<'de> for WhitelistModeVisitor {
            type Value = WhitelistMode;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    "true (database mode), a string path, or a list of hex pubkeys"
                )
            }

            /// `whitelist: true` → Database mode
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<WhitelistMode, E> {
                if v {
                    Ok(WhitelistMode::Database)
                } else {
                    Err(E::custom(
                        "whitelist: false is not valid; omit the key to disable",
                    ))
                }
            }

            /// `whitelist: "/path/to/file"` → File mode
            fn visit_str<E: de::Error>(self, v: &str) -> Result<WhitelistMode, E> {
                Ok(WhitelistMode::File(PathBuf::from(v)))
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<WhitelistMode, E> {
                Ok(WhitelistMode::File(PathBuf::from(v)))
            }

            /// `whitelist: ["aabb...", ...]` → Static mode
            fn visit_seq<A: de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<WhitelistMode, A::Error> {
                let mut pubkeys = Vec::new();
                while let Some(s) = seq.next_element::<String>()? {
                    pubkeys.push(s);
                }
                Ok(WhitelistMode::Static(pubkeys))
            }
        }

        deserializer.deserialize_any(WhitelistModeVisitor)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

    /// Whitelist mode. Omit to allow all users to upload.
    pub whitelist: Option<WhitelistMode>,

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

    /// Reject image uploads containing sensitive EXIF metadata (GPS, device info)
    #[cfg(feature = "blossom")]
    pub reject_sensitive_exif: Option<bool>,

    /// Enable BUD-12 identical media deduplication.
    ///
    /// When set to `true`, image uploads are checked against existing blobs using
    /// perceptual hashing (pHash). If a visually identical image is already stored,
    /// the server returns `409 Conflict` with an `X-Identical-Media` header pointing
    /// to the existing blob's SHA-256.
    ///
    /// Requires the `media-compression` feature. Has no effect without it.
    #[cfg(feature = "media-compression")]
    pub identical_media_dedup: Option<bool>,

    /// Maximum pHash Hamming distance at which two images are considered identical
    /// for BUD-12 deduplication. Only used when `identical_media_dedup` is `true`.
    ///
    /// Lower values are stricter:
    ///   - `0` — bit-exact perceptual hash (only finds near-lossless re-encodes)
    ///   - `1`–`2` — very tight; catches trivial EXIF-strip or minor compression changes
    ///   - `3`–`5` — moderate; catches slight crops or quality changes
    ///
    /// Defaults to `0` when absent.
    #[cfg(feature = "media-compression")]
    pub identical_media_dedup_distance: Option<u32>,

    /// Whether to allow clients to bypass identical-media deduplication by
    /// echoing back the `X-Identical-Media` header from a prior 409 response.
    ///
    /// When `true` (default), a client that sends `X-Identical-Media: <sha256>`
    /// can force the server to store a distinct copy of the blob.
    /// When `false`, the server ignores the acknowledgement and always enforces
    /// deduplication regardless of what the client sends.
    #[cfg(feature = "media-compression")]
    pub identical_media_dedup_allow_override: Option<bool>,

    #[cfg(feature = "payments")]
    /// Payment options for paid storage
    pub payments: Option<PaymentConfig>,

    /// Automatically delete files that have had no downloads in this many days.
    ///
    /// Files uploaded within the same window are given a grace period and are
    /// not deleted even if they have never been downloaded.
    ///
    /// Set to `0` or omit to disable.
    pub delete_unaccessed_days: Option<u64>,

    /// Hard retention limit: delete all files older than this many days,
    /// regardless of whether they have been downloaded.
    ///
    /// Set to `0` or omit to disable.
    pub delete_after_days: Option<u64>,
}

/// Configuration for a single labeling model / API endpoint.
#[cfg(feature = "labels")]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelModelConfig {
    /// Human-readable name stored alongside each label this model produces
    /// (e.g. `"vit224"`, `"nsfw-detector"`).
    pub name: String,

    /// Which labeling backend to use for this model.
    #[serde(flatten)]
    pub labeler_type: LabelerType,

    /// Labels to discard from this model's output (exact match, case-insensitive).
    /// Use this to suppress noise labels that are meaningless for a given model,
    /// e.g. `["normal"]` for the Falconsai NSFW detector.
    #[serde(default)]
    pub label_exclude: Vec<String>,

    /// Minimum confidence score for a label to be stored.
    /// Overrides the global default (0.25) for this model.
    pub min_confidence: Option<f32>,
}

/// The labeling backend type. Uses `#[serde(tag = "type")]` so each variant
/// is selected by a `"type"` key in the YAML config.
#[cfg(feature = "labels")]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LabelerType {
    /// Local ViT model downloaded from HuggingFace.
    Vit {
        /// HuggingFace repo id (e.g. `"google/vit-base-patch16-224"`).
        hf_repo: String,
    },
    /// Generic LLM API endpoint for image classification.
    GenericLlm {
        /// API endpoint URL (e.g. `"https://api.example.com/v1/chat/completions"`).
        api_url: String,
        /// Model name (required, e.g. `"gpt-4o-mini"`).
        model: String,
        /// API key for authentication (optional).
        api_key: Option<String>,
        /// Custom prompt template for the LLM (optional, `{mime_type}` placeholder).
        prompt_template: Option<String>,
    },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelist_mode_deserialize_database() {
        let mode: WhitelistMode = serde_json::from_str("true").unwrap();
        assert!(matches!(mode, WhitelistMode::Database));
    }

    #[test]
    fn whitelist_mode_deserialize_false_is_error() {
        assert!(serde_json::from_str::<WhitelistMode>("false").is_err());
    }

    #[test]
    fn whitelist_mode_deserialize_file() {
        let mode: WhitelistMode = serde_json::from_str(r#""/etc/whitelist.txt""#).unwrap();
        assert!(matches!(mode, WhitelistMode::File(p) if p == PathBuf::from("/etc/whitelist.txt")));
    }

    #[test]
    fn whitelist_mode_deserialize_static_list() {
        let mode: WhitelistMode = serde_json::from_str(r#"["aabbcc","ddeeff"]"#).unwrap();
        assert!(matches!(mode, WhitelistMode::Static(ref v) if v == &["aabbcc", "ddeeff"]));
    }

    #[test]
    fn whitelist_mode_deserialize_empty_list() {
        let mode: WhitelistMode = serde_json::from_str("[]").unwrap();
        assert!(matches!(mode, WhitelistMode::Static(ref v) if v.is_empty()));
    }

    #[test]
    fn delete_unaccessed_days_deserializes_some() {
        let json = r#"{"storage_dir":"/data","database":"mysql://localhost","max_upload_bytes":1048576,"public_url":"https://example.com","delete_unaccessed_days":30}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.delete_unaccessed_days, Some(30));
    }

    #[test]
    fn delete_unaccessed_days_deserializes_absent_as_none() {
        let json = r#"{"storage_dir":"/data","database":"mysql://localhost","max_upload_bytes":1048576,"public_url":"https://example.com"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.delete_unaccessed_days, None);
    }

    #[test]
    fn delete_unaccessed_days_zero_is_valid() {
        let json = r#"{"storage_dir":"/data","database":"mysql://localhost","max_upload_bytes":1048576,"public_url":"https://example.com","delete_unaccessed_days":0}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.delete_unaccessed_days, Some(0));
    }

    #[test]
    fn delete_after_days_deserializes_some() {
        let json = r#"{"storage_dir":"/data","database":"mysql://localhost","max_upload_bytes":1048576,"public_url":"https://example.com","delete_after_days":30}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.delete_after_days, Some(30));
    }

    #[test]
    fn delete_after_days_deserializes_absent_as_none() {
        let json = r#"{"storage_dir":"/data","database":"mysql://localhost","max_upload_bytes":1048576,"public_url":"https://example.com"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.delete_after_days, None);
    }

    #[test]
    fn both_deletion_policies_can_coexist() {
        let json = r#"{"storage_dir":"/data","database":"mysql://localhost","max_upload_bytes":1048576,"public_url":"https://example.com","delete_unaccessed_days":14,"delete_after_days":30}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.delete_unaccessed_days, Some(14));
        assert_eq!(s.delete_after_days, Some(30));
    }
}
