use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum StorageBackendType {
    FileSystem,
    #[cfg(feature = "s3-storage")]
    S3,
}

fn default_storage_type() -> StorageBackendType {
    StorageBackendType::FileSystem
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSystemStorageSettings {
    /// Directory to store files permanently
    pub storage_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Listen addr:port
    pub listen: Option<String>,

    /// Specifies which storage backend to use ("FileSystem" or "S3")
    #[serde(default = "default_storage_type")]
    pub storage_type: StorageBackendType,

    /// Filesystem backend specific settings
    #[serde(default)]
    pub filesystem: Option<FileSystemStorageSettings>,

    /// S3 backend specific settings (only available if 's3-storage' feature is enabled)
    #[cfg(feature = "s3-storage")]
    #[serde(default)]
    pub s3: Option<S3StorageSettings>,

    /// Database connection string postgres://user:pass@host:5432/db
    pub database: String,

    /// Maximum support filesize for uploading
    pub max_upload_bytes: u64,

    /// Public facing url
    pub public_url: String,

    /// Whitelisted pubkeys
    pub whitelist: Option<Vec<String>>,

    /// Path for ViT image model
    pub vit_model: Option<VitModelConfig>,

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

#[cfg(feature = "s3-storage")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3StorageSettings {
    pub region: Option<String>,
    pub endpoint_url: Option<String>,
    pub bucket_name: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    #[serde(default)]
    pub force_path_style: bool,
}
