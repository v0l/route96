use crate::db::{Database, FileUpload};
use crate::filesystem::FileStore;
#[cfg(feature = "media-compression")]
use crate::processing::WebpProcessor;
pub use crate::routes::admin::admin_routes;
#[cfg(feature = "blossom")]
pub use crate::routes::blossom::blossom_routes;
#[cfg(feature = "nip96")]
pub use crate::routes::nip96::nip96_routes;
use crate::settings::Settings;
use crate::whitelist::Whitelist;
use anyhow::{Error, Result};
use axum::{
    extract::{Path, State as AxumState},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
};
use axum_extra::response::file_stream::FileStream;
use http_range_header::{parse_range_header, EndPosition, StartPosition};
use log::warn;
use nostr::Event;
use serde::Serialize;
#[cfg(feature = "media-compression")]
use std::env::temp_dir;
use std::io::SeekFrom;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;

mod admin;
#[cfg(feature = "blossom")]
mod blossom;
#[cfg(feature = "nip96")]
mod nip96;
#[cfg(feature = "payments")]
pub mod payment;

#[derive(Clone)]
pub struct AppState {
    pub fs: FileStore,
    pub db: Database,
    pub settings: Settings,
    pub wl: Whitelist,
    #[cfg(feature = "payments")]
    pub lnd: Option<fedimint_tonic_lnd::Client>,
}

pub struct FilePayload {
    pub file: File,
    pub info: FileUpload,
}

#[derive(Clone, Debug, Serialize, Default)]
struct Nip94Event {
    pub created_at: i64,
    pub content: Option<String>,
    pub tags: Vec<Vec<String>>,
}

#[derive(Serialize, Default)]
struct PagedResult<T> {
    pub count: u32,
    pub page: u32,
    pub total: u32,
    pub files: Vec<T>,
}

impl Nip94Event {
    pub fn from_upload(settings: &Settings, upload: &FileUpload) -> Self {
        let hex_id = hex::encode(&upload.id);
        let ext = if upload.mime_type != "application/octet-stream" {
            mime2ext::mime2ext(&upload.mime_type)
        } else {
            None
        };
        let mut tags = vec![
            vec![
                "url".to_string(),
                format!("{}/{}.{}", &settings.public_url, &hex_id, ext.unwrap_or("")),
            ],
            vec!["x".to_string(), hex_id.clone()],
            vec!["m".to_string(), upload.mime_type.clone()],
            vec!["size".to_string(), upload.size.to_string()],
        ];
        if upload.mime_type.starts_with("image/") || upload.mime_type.starts_with("video/") {
            tags.push(vec![
                "thumb".to_string(),
                format!("{}/thumb/{}.webp", &settings.public_url, &hex_id),
            ]);
        }

        if let Some(bh) = &upload.blur_hash {
            tags.push(vec!["blurhash".to_string(), bh.clone()]);
        }
        if let (Some(w), Some(h)) = (upload.width, upload.height) {
            tags.push(vec!["dim".to_string(), format!("{}x{}", w, h)])
        }
        if let Some(d) = &upload.duration {
            tags.push(vec!["duration".to_string(), d.to_string()]);
        }
        if let Some(b) = &upload.bitrate {
            tags.push(vec!["bitrate".to_string(), b.to_string()]);
        }

        #[cfg(feature = "labels")]
        for l in &upload.labels {
            let val = if l.label.contains(',') {
                let split_val: Vec<&str> = l.label.split(',').collect();
                split_val[0].to_string()
            } else {
                l.label.clone()
            };
            tags.push(vec!["t".to_string(), val])
        }

        Self {
            content: upload.name.clone(),
            created_at: upload.created.timestamp(),
            tags,
        }
    }
}

const MAX_UNBOUNDED_RANGE: u64 = 1024 * 1024;

/// Set common headers for file responses
fn set_file_headers(response: &mut Response, info: &FileUpload) {
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        info.mime_type.parse().unwrap_or_else(|_| "application/octet-stream".parse().unwrap()),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        "max-age=31536000, immutable".parse().unwrap(),
    );
    response.headers_mut().insert(
        header::ACCEPT_RANGES,
        "bytes".parse().unwrap(),
    );
    if let Some(name) = &info.name {
        if let Ok(disposition) = format!("inline; filename=\"{}\"", name).parse() {
            response.headers_mut().insert(header::CONTENT_DISPOSITION, disposition);
        }
    }
}

impl IntoResponse for FilePayload {
    fn into_response(self) -> Response {
        let stream = ReaderStream::new(self.file);
        let file_stream = FileStream::new(stream).content_size(self.info.size);
        let mut response = file_stream.into_response();
        set_file_headers(&mut response, &self.info);
        response
    }
}

async fn delete_file(
    sha256: &str,
    auth: &Event,
    fs: &FileStore,
    db: &Database,
) -> Result<(), Error> {
    let sha256 = if sha256.contains(".") {
        sha256.split('.').next().unwrap()
    } else {
        sha256
    };
    let id = if let Ok(i) = hex::decode(sha256) {
        i
    } else {
        return Err(Error::msg("Invalid file id"));
    };

    if id.len() != 32 {
        return Err(Error::msg("Invalid file id"));
    }
    if let Ok(Some(_info)) = db.get_file(&id).await {
        let pubkey_vec = auth.pubkey.to_bytes().to_vec();
        let auth_user = db.get_user(&pubkey_vec).await?;
        let owners = db.get_file_owners(&id).await?;
        if auth_user.is_admin {
            if let Err(e) = db.delete_all_file_owner(&id).await {
                return Err(Error::msg(format!("Failed to delete (db): {}", e)));
            }
            if let Err(e) = db.delete_file(&id).await {
                return Err(Error::msg(format!("Failed to delete (fs): {}", e)));
            }
            if let Err(e) = tokio::fs::remove_file(fs.get(&id)).await {
                warn!("Failed to delete (fs): {}", e);
            }
        } else {
            let this_owner = match owners.iter().find(|o| o.pubkey.eq(&pubkey_vec)) {
                Some(o) => o,
                None => return Err(Error::msg("You dont own this file, you cannot delete it")),
            };
            if let Err(e) = db.delete_file_owner(&id, this_owner.id).await {
                return Err(Error::msg(format!("Failed to delete (db): {}", e)));
            }
            // only 1 owner was left, delete file completely
            if owners.len() == 1 {
                if let Err(e) = db.delete_file(&id).await {
                    return Err(Error::msg(format!("Failed to delete (fs): {}", e)));
                }
                if let Err(e) = tokio::fs::remove_file(fs.get(&id)).await {
                    warn!("Failed to delete (fs): {}", e);
                }
            }
        }
        Ok(())
    } else {
        Err(Error::msg("File not found"))
    }
}

pub async fn root() -> Result<Html<Vec<u8>>, StatusCode> {
    #[cfg(all(debug_assertions, feature = "react-ui"))]
    let index = "./ui_src/dist/index.html";
    #[cfg(all(not(debug_assertions), feature = "react-ui"))]
    let index = "./ui/index.html";
    #[cfg(not(feature = "react-ui"))]
    let index = "./index.html";
    
    match tokio::fs::read(index).await {
        Ok(contents) => Ok(Html(contents)),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Get the range from a parsed Range header
fn get_range_from_header(range_header: &str, file_size: u64) -> Option<(u64, u64)> {
    let ranges = match parse_range_header(range_header) {
        Ok(r) => r,
        Err(_) => return None,
    };

    // Only handle single range (no multipart)
    if ranges.ranges.len() != 1 {
        warn!("Multipart ranges are not supported, fallback to non-range request");
        return None;
    }

    let single_range = ranges.ranges.first().unwrap();

    let start = match single_range.start {
        StartPosition::Index(i) => i,
        StartPosition::FromLast(i) => file_size.saturating_sub(i),
    };

    let end = match single_range.end {
        EndPosition::Index(i) => i,
        EndPosition::LastByte => (file_size - 1).min(start + MAX_UNBOUNDED_RANGE),
    };

    // Validate the range
    if start > end || start >= file_size {
        return None;
    }

    // Clamp end to file size
    let end = end.min(file_size - 1);

    Some((start, end))
}

pub async fn get_blob(
    Path(sha256): Path<String>,
    headers: HeaderMap,
    AxumState(state): AxumState<Arc<AppState>>,
) -> Result<Response, StatusCode> {
    let sha256 = if sha256.contains(".") {
        sha256.split('.').next().unwrap()
    } else {
        &sha256
    };
    let id = if let Ok(i) = hex::decode(sha256) {
        i
    } else {
        return Err(StatusCode::NOT_FOUND);
    };

    if id.len() != 32 {
        return Err(StatusCode::NOT_FOUND);
    }

    let info = match state.db.get_file(&id).await {
        Ok(Some(info)) => info,
        _ => return Err(StatusCode::NOT_FOUND),
    };

    let file_path = state.fs.get(&id);

    // Check for Range header and handle range requests
    let range_header = headers
        .get(header::RANGE)
        .and_then(|h| h.to_str().ok());

    // Only use range response for files > 1MiB
    if info.size >= MAX_UNBOUNDED_RANGE {
        if let Some(range_str) = range_header {
            if let Some((start, end)) = get_range_from_header(range_str, info.size) {
                // Build the range response directly
                return Ok(build_range_response(file_path, info, start, end).await?);
            }
        }
    }

    // Full file response
    let file = File::open(&file_path).await.map_err(|_| StatusCode::NOT_FOUND)?;
    let payload = FilePayload { file, info };

    Ok(payload.into_response())
}

/// Build a range response by reading the specified range from file
async fn build_range_response(
    file_path: PathBuf,
    info: FileUpload,
    start: u64,
    end: u64,
) -> Result<Response, StatusCode> {
    let mut file = File::open(&file_path).await.map_err(|_| StatusCode::NOT_FOUND)?;
    file.seek(SeekFrom::Start(start)).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let range_len = end - start + 1;
    let limited_reader = file.take(range_len);
    let stream = ReaderStream::new(limited_reader);
    let file_stream = FileStream::new(stream);

    let mut response = file_stream.into_range_response(start, end, info.size);
    set_file_headers(&mut response, &info);

    Ok(response)
}

pub async fn head_blob(
    Path(sha256): Path<String>,
    AxumState(state): AxumState<Arc<AppState>>,
) -> StatusCode {
    let sha256 = if sha256.contains(".") {
        sha256.split('.').next().unwrap()
    } else {
        &sha256
    };
    let id = if let Ok(i) = hex::decode(sha256) {
        i
    } else {
        return StatusCode::NOT_FOUND;
    };

    if id.len() != 32 {
        return StatusCode::NOT_FOUND;
    }
    if state.fs.get(&id).exists() {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

/// Generate thumbnail for image / video
#[cfg(feature = "media-compression")]
pub async fn get_blob_thumb(
    Path(sha256): Path<String>,
    AxumState(state): AxumState<Arc<AppState>>,
) -> Result<FilePayload, StatusCode> {
    let sha256 = if sha256.contains(".") {
        sha256.split('.').next().unwrap()
    } else {
        &sha256
    };
    let id = if let Ok(i) = hex::decode(sha256) {
        i
    } else {
        return Err(StatusCode::NOT_FOUND);
    };

    if id.len() != 32 {
        return Err(StatusCode::NOT_FOUND);
    }
    let info = if let Ok(Some(info)) = state.db.get_file(&id).await {
        info
    } else {
        return Err(StatusCode::NOT_FOUND);
    };

    if !(info.mime_type.starts_with("image/") || info.mime_type.starts_with("video/")) {
        return Err(StatusCode::NOT_FOUND);
    }

    let file_path = state.fs.get(&id);

    let mut thumb_file = temp_dir().join(format!("thumb_{}", sha256));
    thumb_file.set_extension("webp");

    if !thumb_file.exists() {
        let mut p = WebpProcessor::new();
        if let Err(e) = p.thumbnail(&file_path, &thumb_file) {
            warn!("Failed to generate thumbnail: {}", e);
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    if let Ok(f) = File::open(&thumb_file).await {
        Ok(FilePayload {
            file: f,
            info: FileUpload {
                size: thumb_file.metadata().unwrap().len(),
                mime_type: "image/webp".to_string(),
                ..info
            },
        })
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}