use std::collections::HashMap;
use std::env::temp_dir;
use std::ops::Sub;
use std::path::PathBuf;
use std::time::Duration;

use crate::auth::nip98::Nip98Auth;
use crate::db::FileUpload;
use crate::filesystem::FileSystemResult;
use crate::routes::{AppState, Nip94Event, PagedResult, delete_file};
use crate::settings::Settings;
use axum::{
    Json, Router,
    extract::{Multipart, Path, Query, State as AxumState},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete as route_delete, get, post},
};
use futures_util::StreamExt;
use log::error;
use nostr::Timestamp;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Serialize, Default)]
struct Nip96InfoDoc {
    /// File upload and deletion are served from this url
    pub api_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegated_to_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_nips: Option<Vec<usize>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tos_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_types: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plans: Option<HashMap<String, Nip96Plan>>,
}

#[derive(Serialize, Default)]
struct Nip96Plan {
    pub name: String,
    pub is_nip98_required: bool,
    /// landing page for this plan
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub max_byte_size: u64,
    /// Range in days / 0 for no expiration
    /// [7, 0] means it may vary from 7 days to unlimited persistence,
    /// [0, 0] means it has no expiration
    /// early expiration may be due to low traffic or any other factor
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_expiration: Option<(usize, usize)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_transformations: Option<Nip96MediaTransformations>,
}

#[derive(Serialize, Default)]
struct Nip96MediaTransformations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video: Option<Vec<String>>,
}

enum Nip96Response {
    GenericError(Json<Nip96UploadResult>),
    UploadResult(Json<Nip96UploadResult>),
    FileList(Json<PagedResult<Nip94Event>>),
    Forbidden(Json<Nip96UploadResult>),
}

impl Nip96Response {
    pub(crate) fn error(msg: &str) -> Self {
        Nip96Response::GenericError(Json(Nip96UploadResult::error(msg)))
    }

    fn success(msg: &str) -> Self {
        Nip96Response::UploadResult(Json(Nip96UploadResult::success(msg)))
    }
}

impl IntoResponse for Nip96Response {
    fn into_response(self) -> Response {
        match self {
            Nip96Response::GenericError(json) => {
                (StatusCode::INTERNAL_SERVER_ERROR, json).into_response()
            }
            Nip96Response::UploadResult(json) => (StatusCode::OK, json).into_response(),
            Nip96Response::FileList(json) => (StatusCode::OK, json).into_response(),
            Nip96Response::Forbidden(json) => (StatusCode::FORBIDDEN, json).into_response(),
        }
    }
}

#[derive(Serialize, Default)]
struct Nip96UploadResult {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processing_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nip94_event: Option<Nip94Event>,
}

impl Nip96UploadResult {
    pub fn from_upload(settings: &Settings, upload: &FileUpload) -> Self {
        Self {
            status: "success".to_string(),
            nip94_event: Some(Nip94Event::from_upload(settings, upload)),
            ..Default::default()
        }
    }

    pub fn success(msg: &str) -> Self {
        Nip96UploadResult {
            status: "error".to_string(),
            message: Some(msg.to_string()),
            ..Default::default()
        }
    }

    pub fn error(msg: &str) -> Self {
        Nip96UploadResult {
            status: "error".to_string(),
            message: Some(msg.to_string()),
            ..Default::default()
        }
    }
}

struct Nip96Form {
    tmp_file: PathBuf,
    expiration: Option<usize>,
    size: u64,
    alt: Option<String>,
    caption: Option<String>,
    content_type: Option<String>,
    no_transform: Option<bool>,
}

impl Nip96Form {
    async fn from_multipart(mut multipart: Multipart) -> Result<Self, String> {
        let mut file_stream = None;
        let mut expiration = None;
        let mut size = 0;
        let mut alt = None;
        let mut caption = None;
        let mut content_type = None;
        let mut no_transform = None;

        while let Some(field) = multipart
            .next_field()
            .await
            .map_err(|e| format!("Failed to get field: {}", e))?
        {
            let name = field.name().unwrap_or("").to_string();
            match name.as_str() {
                "file" => {
                    let temp_id = Uuid::new_v4();
                    let tmp_path = temp_dir().join(temp_id.to_string());
                    tokio::fs::write(
                        &tmp_path,
                        field.bytes().await.map_err(|e| {
                            error!("Failed to write file: {}", e);
                            "Failed to write temp file".to_string()
                        })?,
                    )
                    .await
                    .map_err(|e| {
                        error!("Failed to write file: {}", e);
                        "Failed to write temp file".to_string()
                    })?;
                    file_stream = Some(tmp_path);
                }
                "expiration" => {
                    if let Ok(text) = field.text().await {
                        expiration = text.parse().ok();
                    }
                }
                "size" => {
                    if let Ok(text) = field.text().await {
                        size = text.parse().unwrap_or(0);
                    }
                }
                "alt" => {
                    if let Ok(text) = field.text().await {
                        alt = Some(text);
                    }
                }
                "caption" => {
                    if let Ok(text) = field.text().await {
                        caption = Some(text);
                    }
                }
                "content_type" => {
                    if let Ok(text) = field.text().await {
                        content_type = Some(text);
                    }
                }
                "no_transform" => {
                    if let Ok(text) = field.text().await {
                        no_transform = text.parse::<bool>().ok();
                    }
                }
                _ => {}
            }
        }

        Ok(Nip96Form {
            tmp_file: file_stream.ok_or_else(|| "Missing file field".to_string())?,
            expiration,
            size,
            alt,
            caption,
            content_type,
            no_transform,
        })
    }
}

pub fn nip96_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/.well-known/nostr/nip96.json", get(get_info_doc))
        .route("/n96", post(upload).get(list_files))
        .route("/n96/{sha256}", route_delete(delete))
}

async fn get_info_doc(AxumState(state): AxumState<Arc<AppState>>) -> Json<Nip96InfoDoc> {
    let mut plans = HashMap::new();
    plans.insert(
        "free".to_string(),
        Nip96Plan {
            is_nip98_required: true,
            max_byte_size: state.settings.max_upload_bytes,
            ..Default::default()
        },
    );
    Json(Nip96InfoDoc {
        api_url: "/n96".to_string(),
        download_url: Some("/".to_string()),
        content_types: Some(vec![
            "image/*".to_string(),
            "video/*".to_string(),
            "audio/*".to_string(),
        ]),
        plans: Some(plans),
        ..Default::default()
    })
}

async fn upload(
    auth: Nip98Auth,
    AxumState(state): AxumState<Arc<AppState>>,
    multipart: Multipart,
) -> Nip96Response {
    let form = match Nip96Form::from_multipart(multipart).await {
        Ok(f) => f,
        Err(e) => return Nip96Response::error(&format!("Could not parse form: {}", e)),
    };

    let upload_size = auth.content_length.or(Some(form.size)).unwrap_or(0);
    if upload_size > 0 && upload_size > state.settings.max_upload_bytes {
        return Nip96Response::error("File too large");
    }

    let content_type = form
        .content_type
        .as_deref()
        .unwrap_or("application/octet-stream");

    if form.expiration.is_some() {
        return Nip96Response::error("Expiration not supported");
    }

    // account for upload speeds as slow as 1MB/s (8 Mbps)
    let size_for_timing = if upload_size > 0 {
        upload_size
    } else {
        form.size
    };
    let mbs = size_for_timing / (1024 * 1024); // 1 MB in bytes
    let max_time = 60.max(mbs);
    if auth.event.created_at < Timestamp::now().sub(Duration::from_secs(max_time)) {
        return Nip96Response::error("Auth event timestamp out of range");
    }

    // check whitelist
    if !state.wl.contains_hex(&auth.event.pubkey.to_hex()) {
        return Nip96Response::Forbidden(Json(Nip96UploadResult::error("Not on whitelist")));
    }

    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();

    // check quota (only if payments are configured)
    #[cfg(feature = "payments")]
    if let Some(payment_config) = &state.settings.payments {
        let free_quota = payment_config.free_quota_bytes.unwrap_or(104857600); // Default to 100MB

        if upload_size > 0 {
            match state
                .db
                .check_user_quota(&pubkey_vec, upload_size, free_quota)
                .await
            {
                Ok(false) => return Nip96Response::error("Upload would exceed quota"),
                Err(_) => return Nip96Response::error("Failed to check quota"),
                Ok(true) => {} // Quota check passed
            }
        }
    }

    let Ok(temp_file) = tokio::fs::File::open(form.tmp_file).await else {
        return Nip96Response::error("Failed to open temporary file");
    };
    let upload = match state
        .fs
        .put(temp_file, content_type, !form.no_transform.unwrap_or(false))
        .await
    {
        Ok(FileSystemResult::NewFile(blob)) => {
            let mut upload: FileUpload = (&blob).into();

            // Validate file size after upload if no pre-upload size was available
            if upload_size == 0 && upload.size > state.settings.max_upload_bytes {
                // Clean up the uploaded file
                if let Err(e) = tokio::fs::remove_file(state.fs.get(&upload.id)).await {
                    log::warn!("Failed to cleanup oversized file: {}", e);
                }
                return Nip96Response::error("File too large");
            }

            upload.name = form.caption;
            upload.alt = form.alt;
            upload
        }
        Ok(FileSystemResult::AlreadyExists(i)) => match state.db.get_file(&i).await {
            Ok(Some(f)) => f,
            _ => return Nip96Response::error("File not found"),
        },
        Err(e) => {
            error!("{}", e);
            return Nip96Response::error(&format!("Could not save file: {}", e));
        }
    };

    let user_id = match state.db.upsert_user(&pubkey_vec).await {
        Ok(u) => u,
        Err(e) => return Nip96Response::error(&format!("Could not save user: {}", e)),
    };

    // Post-upload quota check if we didn't have size information before upload (only if payments are configured)
    #[cfg(feature = "payments")]
    if upload_size == 0 {
        if let Some(payment_config) = &state.settings.payments {
            let free_quota = payment_config.free_quota_bytes.unwrap_or(104857600); // Default to 100MB

            match state
                .db
                .check_user_quota(&pubkey_vec, upload.size, free_quota)
                .await
            {
                Ok(false) => {
                    // Clean up the uploaded file if quota exceeded
                    if let Err(e) = tokio::fs::remove_file(state.fs.get(&upload.id)).await {
                        log::warn!("Failed to cleanup quota-exceeding file: {}", e);
                    }
                    return Nip96Response::error("Upload would exceed quota");
                }
                Err(_) => {
                    // Clean up on quota check error
                    if let Err(e) = tokio::fs::remove_file(state.fs.get(&upload.id)).await {
                        log::warn!("Failed to cleanup file after quota check error: {}", e);
                    }
                    return Nip96Response::error("Failed to check quota");
                }
                Ok(true) => {} // Quota check passed
            }
        }
    }

    if let Err(e) = state.db.add_file(&upload, Some(user_id)).await {
        error!("{}", e);
        return Nip96Response::error(&format!("Could not save file (db): {}", e));
    }
    Nip96Response::UploadResult(Json(Nip96UploadResult::from_upload(
        &state.settings,
        &upload,
    )))
}

async fn delete(
    Path(sha256): Path<String>,
    auth: Nip98Auth,
    AxumState(state): AxumState<Arc<AppState>>,
) -> Nip96Response {
    match delete_file(&sha256, &auth.event, &state.fs, &state.db).await {
        Ok(()) => Nip96Response::success("File deleted."),
        Err(e) => Nip96Response::error(&format!("Failed to delete file: {}", e)),
    }
}

#[derive(Deserialize)]
struct ListFilesQuery {
    page: u32,
    count: u32,
}

async fn list_files(
    auth: Nip98Auth,
    Query(query): Query<ListFilesQuery>,
    AxumState(state): AxumState<Arc<AppState>>,
) -> Nip96Response {
    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
    let server_count = query.count.min(5_000).max(1);
    match state
        .db
        .list_files(&pubkey_vec, query.page * server_count, server_count)
        .await
    {
        Ok((files, total)) => Nip96Response::FileList(Json(PagedResult {
            count: server_count,
            page: query.page,
            total: total as u32,
            files: files
                .iter()
                .map(|f| {
                    Nip96UploadResult::from_upload(&state.settings, f)
                        .nip94_event
                        .unwrap()
                })
                .collect(),
        })),
        Err(e) => Nip96Response::error(&format!("Could not list files: {}", e)),
    }
}
