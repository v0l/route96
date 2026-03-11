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
#[derive(Debug, Clone, Serialize)]
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

    /// Analytics tracking
    pub plausible_url: Option<String>,

    /// Reject image uploads containing sensitive EXIF metadata (GPS, device info)
    #[cfg(feature = "blossom")]
    pub reject_sensitive_exif: Option<bool>,

    #[cfg(feature = "payments")]
    /// Payment options for paid storage
    pub payments: Option<PaymentConfig>,
}

/// Configuration for a single labeling model / API endpoint.
#[cfg(feature = "labels")]
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LabelerType {
    /// Local ViT model downloaded from HuggingFace.
    Vit {
        /// HuggingFace repo id (e.g. `"google/vit-base-patch16-224"`).
        hf_repo: String,
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
}
