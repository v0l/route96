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

    /// Path for ViT image model
    pub vit_model: Option<VitModelConfig>,

    /// Webhook api endpoint
    pub webhook_url: Option<String>,

    /// Analytics tracking
    pub plausible_url: Option<String>,

    #[cfg(feature = "void-cat-redirects")]
    pub void_cat_database: Option<String>,

    /// Path to void.cat uploads (files-v2)
    pub void_cat_files: Option<PathBuf>,

    /// NIP-29 relay configuration
    pub nip29_relay: Nip29RelayConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VitModelConfig {
    pub model: PathBuf,
    pub config: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Nip29RelayConfig {
    /// NIP-29 relay URL
    pub url: String,
    /// Private key for the relay (hex format)
    pub private_key: String,
    /// Cache expiration time in seconds
    pub cache_expiration: Option<u64>,
}
