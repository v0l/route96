#[cfg(feature = "analytics")]
pub mod analytics;
pub mod auth;
pub mod background;
pub mod comma_separated;
pub mod config_watcher;
pub mod cors;
pub mod db;
pub mod db_config;
#[cfg(feature = "blossom")]
pub mod exif_validator;
pub mod file_stats;
pub mod filesystem;
#[cfg(feature = "payments")]
pub mod payments;
#[cfg(feature = "media-compression")]
pub mod phash;
#[cfg(feature = "media-compression")]
pub mod processing;
pub mod routes;
pub mod settings;
pub mod whitelist;

pub fn can_compress(mime_type: &str) -> bool {
    mime_type.starts_with("image/")
}
