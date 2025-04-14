#![deny(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]

pub mod auth;
// pub mod handlers; // Commented out - missing file
pub mod background;
pub mod cors;
pub mod db;
pub mod filesystem;
pub mod nip29;
#[cfg(feature = "media-compression")]
pub mod processing;
pub mod routes;
pub mod settings;
#[cfg(feature = "s3-storage")]
pub mod spaces;
pub mod storage;
// pub mod types; // Removed
// pub mod utils; // Removed
