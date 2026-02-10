use crate::auth::blossom::BlossomAuth;
use crate::db::FileUpload;
use crate::filesystem::FileSystemResult;
use crate::routes::{AppState, Nip94Event, delete_file};
use crate::settings::Settings;
use crate::whitelist::Whitelist;
use axum::{
    Json, Router,
    body::Body,
    extract::State as AxumState,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, head, put},
};
use futures_util::TryStreamExt;
use futures_util::stream::StreamExt;
use log::{error, info};
use nostr::{Alphabet, JsonUtil, SingleLetterTag, TagKind};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::AsyncRead;
use tokio_util::io::StreamReader;
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobDescriptor {
    pub url: String,
    pub sha256: String,
    pub size: u64,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub uploaded: u64,
    #[serde(rename = "nip94", skip_serializing_if = "Option::is_none")]
    pub nip94: Option<Vec<Vec<String>>>,
}

impl BlobDescriptor {
    pub fn from_upload(settings: &Settings, value: &FileUpload) -> Self {
        let id_hex = hex::encode(&value.id);
        Self {
            url: format!(
                "{}/{}{}",
                settings.public_url,
                &id_hex,
                mime2ext::mime2ext(&value.mime_type)
                    .map(|m| format!(".{m}"))
                    .unwrap_or("".to_string())
            ),
            sha256: id_hex,
            size: value.size,
            mime_type: Some(value.mime_type.clone()),
            uploaded: value.created.timestamp() as u64,
            nip94: Some(Nip94Event::from_upload(settings, value).tags),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MirrorRequest {
    pub url: String,
}

pub fn blossom_routes() -> Router<Arc<AppState>> {
    let router = Router::new()
        .route("/{sha256}", delete(delete_blob))
        .route("/list/{pubkey}", get(list_files))
        .route("/upload", head(upload_head).put(upload))
        .route("/mirror", put(mirror))
        .route("/report", put(report_file));

    #[cfg(feature = "media-compression")]
    let router = router.route("/media", head(head_media).put(upload_media));

    router
}

/// Generic holder response, mostly for errors
struct BlossomGenericResponse {
    pub message: Option<String>,
    pub status: StatusCode,
}

impl IntoResponse for BlossomGenericResponse {
    fn into_response(self) -> Response {
        let mut headers = HeaderMap::new();
        if let Some(message) = self.message
            && let Ok(value) = message.parse()
        {
            headers.insert("x-reason", value);
        }
        (self.status, headers).into_response()
    }
}

enum BlossomResponse {
    Generic(BlossomGenericResponse),
    BlobDescriptor(Json<BlobDescriptor>),
    BlobDescriptorList(Json<Vec<BlobDescriptor>>),
}

impl IntoResponse for BlossomResponse {
    fn into_response(self) -> Response {
        match self {
            BlossomResponse::Generic(g) => g.into_response(),
            BlossomResponse::BlobDescriptor(j) => (StatusCode::OK, j).into_response(),
            BlossomResponse::BlobDescriptorList(j) => (StatusCode::OK, j).into_response(),
        }
    }
}

impl BlossomResponse {
    pub fn error(msg: impl Into<String>) -> Self {
        Self::Generic(BlossomGenericResponse {
            message: Some(msg.into()),
            status: StatusCode::INTERNAL_SERVER_ERROR,
        })
    }
}

struct BlossomHead {
    pub msg: Option<&'static str>,
}

impl IntoResponse for BlossomHead {
    fn into_response(self) -> Response {
        match self.msg {
            Some(m) => {
                let mut headers = HeaderMap::new();
                headers.insert("x-upload-message", m.parse().unwrap());
                (StatusCode::INTERNAL_SERVER_ERROR, headers).into_response()
            }
            None => StatusCode::OK.into_response(),
        }
    }
}

fn check_method(event: &nostr::Event, method: &str) -> bool {
    if let Some(t) = event.tags.iter().find_map(|t| {
        if t.kind() == TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::T)) {
            t.content()
        } else {
            None
        }
    }) {
        return t.eq_ignore_ascii_case(method);
    }
    false
}

fn check_whitelist(auth: &BlossomAuth, whitelist: &Whitelist) -> Option<BlossomResponse> {
    if !whitelist.contains_hex(&auth.event.pubkey.to_hex()) {
        return Some(BlossomResponse::Generic(BlossomGenericResponse {
            status: StatusCode::FORBIDDEN,
            message: Some("Not on whitelist".to_string()),
        }));
    }
    None
}

async fn delete_blob(
    axum::extract::Path(sha256): axum::extract::Path<String>,
    auth: BlossomAuth,
    AxumState(state): AxumState<Arc<AppState>>,
) -> BlossomResponse {
    match delete_file(&sha256, &auth.event, &state.fs, &state.db).await {
        Ok(()) => BlossomResponse::Generic(BlossomGenericResponse {
            status: StatusCode::OK,
            message: None,
        }),
        Err(e) => BlossomResponse::error(format!("Failed to delete file: {}", e)),
    }
}

async fn list_files(
    axum::extract::Path(pubkey): axum::extract::Path<String>,
    AxumState(state): AxumState<Arc<AppState>>,
) -> BlossomResponse {
    let id = if let Ok(i) = hex::decode(&pubkey) {
        i
    } else {
        return BlossomResponse::error("invalid pubkey");
    };
    match state.db.list_files(&id, 0, 10_000).await {
        Ok((files, _count)) => BlossomResponse::BlobDescriptorList(Json(
            files
                .iter()
                .map(|f| BlobDescriptor::from_upload(&state.settings, f))
                .collect(),
        )),
        Err(e) => BlossomResponse::error(format!("Could not list files: {}", e)),
    }
}

async fn upload_head(auth: BlossomAuth, AxumState(state): AxumState<Arc<AppState>>) -> BlossomHead {
    check_head(auth, &state.wl, &state.settings)
}

async fn upload(
    auth: BlossomAuth,
    AxumState(state): AxumState<Arc<AppState>>,
    body: Body,
) -> BlossomResponse {
    process_upload("upload", false, auth, state, body).await
}

async fn mirror(
    auth: BlossomAuth,
    AxumState(state): AxumState<Arc<AppState>>,
    Json(req): Json<MirrorRequest>,
) -> BlossomResponse {
    if !check_method(&auth.event, "upload") {
        return BlossomResponse::error("Invalid request method tag");
    }
    if let Some(e) = check_whitelist(&auth, &state.wl) {
        return e;
    }

    let url = match Url::parse(&req.url) {
        Ok(u) => u,
        Err(e) => return BlossomResponse::error(format!("Invalid URL: {}", e)),
    };

    let hash = url
        .path_segments()
        .and_then(|mut c| c.next_back())
        .and_then(|s| s.split(".").next());

    let client = Client::builder().build().unwrap();

    let req_builder = client.get(url.clone()).header(
        "user-agent",
        format!("route96 ({})", state.settings.public_url),
    );
    info!("Requesting mirror: {}", url);
    info!("{:?}", req_builder);

    // download file
    let rsp = match req_builder.send().await {
        Err(e) => {
            error!("Error downloading file: {}", e);
            return BlossomResponse::error("Failed to mirror file");
        }
        Ok(rsp) if !rsp.status().is_success() => {
            let status = rsp.status();
            let body = rsp.bytes().await.unwrap_or(Default::default());
            error!(
                "Error downloading file, status is not OK({}): {}",
                status,
                String::from_utf8_lossy(&body)
            );
            return BlossomResponse::error("Failed to mirror file");
        }
        Ok(rsp) => rsp,
    };

    let mime_type = rsp
        .headers()
        .get("content-type")
        .map(|h| h.to_str().unwrap())
        .unwrap_or("application/octet-stream")
        .to_string();
    let pubkey = auth.event.pubkey.to_bytes().to_vec();

    process_stream(
        StreamReader::new(
            rsp.bytes_stream()
                .map(|result| result.map_err(|e| std::io::Error::other(e))),
        ),
        &mime_type,
        &None,
        &pubkey,
        false,
        0, // No size info for mirror
        state,
        hash.and_then(|h| hex::decode(h).ok()),
    )
    .await
}

#[cfg(feature = "media-compression")]
async fn head_media(auth: BlossomAuth, AxumState(state): AxumState<Arc<AppState>>) -> BlossomHead {
    check_head(auth, &state.wl, &state.settings)
}

#[cfg(feature = "media-compression")]
async fn upload_media(
    auth: BlossomAuth,
    AxumState(state): AxumState<Arc<AppState>>,
    body: Body,
) -> BlossomResponse {
    process_upload("media", true, auth, state, body).await
}

fn check_head(auth: BlossomAuth, whitelist: &Whitelist, settings: &Settings) -> BlossomHead {
    if !check_method(&auth.event, "upload") {
        return BlossomHead {
            msg: Some("Invalid auth method tag"),
        };
    }

    if let Some(z) = auth.x_content_length {
        if z > settings.max_upload_bytes {
            return BlossomHead {
                msg: Some("File too large"),
            };
        }
    } else {
        return BlossomHead {
            msg: Some("Missing x-content-length header"),
        };
    }

    if auth.x_sha_256.is_none() {
        return BlossomHead {
            msg: Some("Missing x-sha-256 header"),
        };
    }

    if auth.x_content_type.is_none() {
        return BlossomHead {
            msg: Some("Missing x-content-type header"),
        };
    }

    // check whitelist
    if !whitelist.contains_hex(&auth.event.pubkey.to_hex()) {
        return BlossomHead {
            msg: Some("Not on whitelist"),
        };
    }

    BlossomHead { msg: None }
}

async fn process_upload(
    method: &str,
    compress: bool,
    auth: BlossomAuth,
    state: Arc<AppState>,
    body: Body,
) -> BlossomResponse {
    if !check_method(&auth.event, method) {
        return BlossomResponse::error("Invalid request method tag");
    }

    let name = auth.event.tags.iter().find_map(|t| {
        if t.kind() == TagKind::Name {
            t.content()
        } else {
            None
        }
    });
    let size_tag = auth.event.tags.iter().find_map(|t| {
        if t.kind() == TagKind::Size {
            t.content().and_then(|v| v.parse::<u64>().ok())
        } else {
            None
        }
    });

    let size = size_tag.or(auth.x_content_length).unwrap_or(0);
    if size > 0 && size > state.settings.max_upload_bytes {
        return BlossomResponse::error("File too large");
    }

    // check whitelist
    if let Some(e) = check_whitelist(&auth, &state.wl) {
        return e;
    }

    // check quota (only if payments are configured)
    #[cfg(feature = "payments")]
    if let Some(payment_config) = &state.settings.payments {
        let free_quota = payment_config.free_quota_bytes.unwrap_or(104857600); // Default to 100MB
        let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();

        if size > 0 {
            match state
                .db
                .check_user_quota(&pubkey_vec, size, free_quota)
                .await
            {
                Ok(false) => return BlossomResponse::error("Upload would exceed quota"),
                Err(_) => return BlossomResponse::error("Failed to check quota"),
                Ok(true) => {} // Quota check passed
            }
        }
    }

    let data_stream = body.into_data_stream();
    let stream = TryStreamExt::map_err(data_stream, |e| std::io::Error::other(e));
    let reader = StreamReader::new(stream);

    process_stream(
        reader,
        &auth
            .content_type
            .unwrap_or("application/octet-stream".to_string()),
        &name,
        &auth.event.pubkey.to_bytes().to_vec(),
        compress,
        size,
        state,
        None,
    )
    .await
}

async fn process_stream<'p, S>(
    stream: S,
    mime_type: &str,
    name: &Option<&str>,
    pubkey: &Vec<u8>,
    compress: bool,
    #[cfg(feature = "payments")] size: u64,
    #[cfg(not(feature = "payments"))] _size: u64,
    state: Arc<AppState>,
    expect_hash: Option<Vec<u8>>,
) -> BlossomResponse
where
    S: AsyncRead + Unpin + 'p,
{
    let upload = match state.fs.put(stream, mime_type, compress).await {
        Ok(FileSystemResult::NewFile(blob)) => {
            let mut ret: FileUpload = (&blob).into();

            // check expected hash (mirroring)
            if let Some(h) = expect_hash
                && h != ret.id
            {
                if let Err(e) = tokio::fs::remove_file(state.fs.get(&ret.id)).await {
                    log::warn!("Failed to cleanup file: {}", e);
                }
                return BlossomResponse::error(
                    "Mirror request failed, server responses with invalid file content (hash mismatch)",
                );
            }

            // Check for sensitive EXIF metadata if enabled
            #[cfg(feature = "blossom")]
            if state.settings.reject_sensitive_exif.unwrap_or(false)
                && mime_type.starts_with("image/")
            {
                let file_path = state.fs.get(&ret.id);
                if let Err(e) = crate::exif_validator::check_for_sensitive_exif(&file_path) {
                    // Clean up the file
                    if let Err(cleanup_err) = tokio::fs::remove_file(&file_path).await {
                        log::warn!(
                            "Failed to cleanup file with sensitive EXIF: {}",
                            cleanup_err
                        );
                    }
                    return BlossomResponse::error(format!("Upload rejected: {}", e));
                }
            }

            // update file data before inserting
            ret.name = name.map(|s| s.to_string());

            ret
        }
        Ok(FileSystemResult::AlreadyExists(i)) => match state.db.get_file(&i).await {
            Ok(Some(f)) => f,
            _ => return BlossomResponse::error("File not found"),
        },
        Err(e) => {
            error!("{}", e);
            return BlossomResponse::error(format!("Error saving file (disk): {}", e));
        }
    };

    let user_id = match state.db.upsert_user(pubkey).await {
        Ok(u) => u,
        Err(e) => {
            return BlossomResponse::error(format!("Failed to save file (db): {}", e));
        }
    };

    // Post-upload quota check if we didn't have size information before upload (only if payments are configured)
    #[cfg(feature = "payments")]
    if size == 0 {
        if let Some(payment_config) = &state.settings.payments {
            let free_quota = payment_config.free_quota_bytes.unwrap_or(104857600); // Default to 100MB

            match state
                .db
                .check_user_quota(pubkey, upload.size, free_quota)
                .await
            {
                Ok(false) => {
                    // Clean up the uploaded file if quota exceeded
                    if let Err(e) = tokio::fs::remove_file(state.fs.get(&upload.id)).await {
                        log::warn!("Failed to cleanup quota-exceeding file: {}", e);
                    }
                    return BlossomResponse::error("Upload would exceed quota");
                }
                Err(_) => {
                    // Clean up on quota check error
                    if let Err(e) = tokio::fs::remove_file(state.fs.get(&upload.id)).await {
                        log::warn!("Failed to cleanup file after quota check error: {}", e);
                    }
                    return BlossomResponse::error("Failed to check quota");
                }
                Ok(true) => {} // Quota check passed
            }
        }
    }
    if let Err(e) = state.db.add_file(&upload, Some(user_id)).await {
        error!("{}", e);
        BlossomResponse::error(format!("Error saving file (db): {}", e))
    } else {
        BlossomResponse::BlobDescriptor(Json(BlobDescriptor::from_upload(&state.settings, &upload)))
    }
}

async fn report_file(
    auth: BlossomAuth,
    AxumState(state): AxumState<Arc<AppState>>,
    Json(data): Json<nostr::Event>,
) -> BlossomResponse {
    // Check if the request has the correct method tag
    if !check_method(&auth.event, "report") {
        return BlossomResponse::error("Invalid request method tag");
    }

    // Check whitelist
    if let Some(e) = check_whitelist(&auth, &state.wl) {
        return e;
    }

    // Extract file SHA256 from the "x" tag in the report event
    let file_sha256 = if let Some(x_tag) = data.tags.iter().find_map(|t| {
        if t.kind() == TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::X)) {
            t.content()
        } else {
            None
        }
    }) {
        match hex::decode(x_tag) {
            Ok(hash) => hash,
            Err(_) => return BlossomResponse::error("Invalid file hash in x tag"),
        }
    } else {
        return BlossomResponse::error("Missing file hash in x tag");
    };

    // Verify the reported file exists
    match state.db.get_file(&file_sha256).await {
        Ok(Some(_)) => {} // File exists, continue
        Ok(None) => return BlossomResponse::error("File not found"),
        Err(e) => return BlossomResponse::error(format!("Failed to check file: {}", e)),
    }

    // Get or create the reporter user
    let reporter_id = match state
        .db
        .upsert_user(&auth.event.pubkey.to_bytes().to_vec())
        .await
    {
        Ok(user_id) => user_id,
        Err(e) => return BlossomResponse::error(format!("Failed to get user: {}", e)),
    };

    // Store the report (the database will handle duplicate prevention via unique index)
    match state
        .db
        .add_report(&file_sha256, reporter_id, &data.as_json())
        .await
    {
        Ok(()) => BlossomResponse::Generic(BlossomGenericResponse {
            status: StatusCode::OK,
            message: Some("Report submitted successfully".to_string()),
        }),
        Err(e) => {
            if e.to_string().contains("Duplicate entry") {
                BlossomResponse::error("You have already reported this file")
            } else {
                BlossomResponse::error(format!("Failed to submit report: {}", e))
            }
        }
    }
}
