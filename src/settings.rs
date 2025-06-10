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

    /// Path for ViT image model
    pub vit_model: Option<VitModelConfig>,

    /// Webhook api endpoint
    pub webhook_url: Option<String>,

    /// Analytics tracking
    pub plausible_url: Option<String>,

    /// Path to void.cat uploads (files-v2)
    pub void_cat_files: Option<PathBuf>,

    #[cfg(feature = "payments")]
    /// Payment options for paid storage
    pub payments: Option<PaymentConfig>,

    #[cfg(feature = "payments")]
    /// Free quota in bytes for users without payments (default: 100MB)
    pub free_quota_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VitModelConfig {
    pub model: PathBuf,
    pub config: PathBuf,
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
}

#[cfg(feature = "payments")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LndConfig {
    pub endpoint: String,
    pub tls: PathBuf,
    pub macaroon: PathBuf,
}
