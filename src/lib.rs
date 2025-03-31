#[cfg(feature = "analytics")]
pub mod analytics;
pub mod auth;
pub mod background;
pub mod cors;
pub mod db;
pub mod filesystem;
#[cfg(feature = "media-compression")]
pub mod processing;
pub mod routes;
pub mod settings;
#[cfg(any(feature = "void-cat-redirects", feature = "bin-void-cat-migrate"))]
pub mod void_db;
pub mod void_file;

pub fn can_compress(mime_type: &str) -> bool {
    mime_type.starts_with("image/")
}