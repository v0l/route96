use std::collections::HashMap;
use std::env::temp_dir;
use std::ops::Sub;
use std::path::PathBuf;
use std::time::Duration;

use crate::auth::nip98::Nip98Auth;
use crate::db::FileUpload;
use crate::filesystem::FileSystemResult;
use crate::routes::{AppState, Nip94Event, PagedResult, ban_check, delete_file};
use crate::settings::Settings;
use axum::{
    Json, Router,
    extract::{Multipart, Path, Query, State as AxumState},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete as route_delete, get, post},
};
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
            status: "success".to_string(),
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

/// Drop guard that removes a temp upload file when the request ends,
/// regardless of success or failure.
struct TempFileGuard(PathBuf);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let path = self.0.clone();
        tokio::spawn(async move {
            tokio::fs::remove_file(path).await.ok();
        });
    }
}

impl Nip96Form {
    async fn from_multipart(mut multipart: Multipart, max_bytes: u64) -> Result<Self, String> {
        use tokio::io::AsyncWriteExt;

        let mut file_stream = None;
        let mut expiration = None;
        let mut size = 0;
        let mut alt = None;
        let mut caption = None;
        let mut content_type = None;
        let mut no_transform = None;

        while let Some(mut field) = multipart
            .next_field()
            .await
            .map_err(|e| format!("Failed to get field: {}", e))?
        {
            let name = field.name().unwrap_or("").to_string();
            match name.as_str() {
                "file" => {
                    let temp_id = Uuid::new_v4();
                    let tmp_path = temp_dir().join(temp_id.to_string());
                    // Stream to disk with a hard byte cap so a chunked upload
                    // with no Content-Length can't exhaust memory or disk.
                    let mut file = match tokio::fs::File::create(&tmp_path).await {
                        Ok(f) => f,
                        Err(e) => {
                            error!("Failed to create temp file: {}", e);
                            return Err("Failed to write temp file".to_string());
                        }
                    };
                    let mut written: u64 = 0;
                    let write_result: Result<(), String> = async {
                        while let Some(chunk) = field
                            .chunk()
                            .await
                            .map_err(|e| format!("Failed to read field chunk: {}", e))?
                        {
                            written += chunk.len() as u64;
                            if written > max_bytes {
                                return Err("File too large".to_string());
                            }
                            file.write_all(&chunk)
                                .await
                                .map_err(|e| format!("Failed to write temp file: {}", e))?;
                        }
                        file.flush()
                            .await
                            .map_err(|e| format!("Failed to write temp file: {}", e))
                    }
                    .await;
                    drop(file);
                    if let Err(e) = write_result {
                        error!("Failed to write file: {}", e);
                        tokio::fs::remove_file(&tmp_path).await.ok();
                        return Err(e);
                    }
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
        .route(
            "/n96",
            post(upload)
                .get(list_files)
                .layer(axum::extract::DefaultBodyLimit::disable()),
        )
        .route("/n96/{sha256}", route_delete(delete))
}

async fn get_info_doc(AxumState(state): AxumState<Arc<AppState>>) -> Json<Nip96InfoDoc> {
    let settings = state.settings().await;
    let mut plans = HashMap::new();
    plans.insert(
        "free".to_string(),
        Nip96Plan {
            is_nip98_required: true,
            max_byte_size: settings.max_upload_bytes,
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

#[cfg_attr(not(feature = "payments"), allow(unused_assignments))]
async fn upload(
    auth: Nip98Auth,
    AxumState(state): AxumState<Arc<AppState>>,
    multipart: Multipart,
) -> Nip96Response {
    let settings = state.settings().await;
    let form = match Nip96Form::from_multipart(multipart, settings.max_upload_bytes).await {
        Ok(f) => f,
        Err(e) => return Nip96Response::error(&format!("Could not parse form: {}", e)),
    };
    // Always remove the temp file when we're done with it.
    let _tmp_guard = TempFileGuard(form.tmp_file.clone());

    let upload_size = auth.content_length.or(Some(form.size)).unwrap_or(0);
    if upload_size > 0 && upload_size > settings.max_upload_bytes {
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
    if !state
        .wl()
        .await
        .is_allowed(&auth.event.pubkey.to_hex())
        .await
    {
        return Nip96Response::Forbidden(Json(Nip96UploadResult::error("Not on whitelist")));
    }

    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();

    if let Some(msg) = ban_check(&state.db, &pubkey_vec).await {
        return Nip96Response::Forbidden(Json(Nip96UploadResult::error(&msg)));
    }

    // check quota (only if payments are configured)
    #[cfg(feature = "payments")]
    if let Some(payment_config) = &settings.payments {
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

    let Ok(temp_file) = tokio::fs::File::open(&form.tmp_file).await else {
        return Nip96Response::error("Failed to open temporary file");
    };
    #[cfg_attr(not(feature = "payments"), allow(unused_variables))]
    let mut is_new_file = false;
    let upload = match state
        .fs
        .put(
            &state.db,
            temp_file,
            content_type,
            !form.no_transform.unwrap_or(false),
        )
        .await
    {
        Ok(FileSystemResult::NewFile(blob)) => {
            is_new_file = true;
            let mut upload: FileUpload = (&blob).into();

            // Validate file size after upload if no pre-upload size was available
            if upload_size == 0 && upload.size > settings.max_upload_bytes {
                if let Err(e) = state.fs.delete(&upload.id).await {
                    log::warn!("Failed to cleanup oversized file: {}", e);
                }
                return Nip96Response::error("File too large");
            }

            upload.name = form.caption;
            upload.alt = form.alt;
            upload
        }
        Ok(FileSystemResult::Banned) => {
            return Nip96Response::Forbidden(Json(Nip96UploadResult::error(
                "File is not allowed on this server",
            )));
        }
        Ok(FileSystemResult::AlreadyExists(i)) => match state.db.get_file(&i).await {
            Ok(Some(f)) if !f.banned => f,
            Ok(Some(_)) => {
                return Nip96Response::Forbidden(Json(Nip96UploadResult::error(
                    "File is not allowed on this server",
                )));
            }
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

    // Post-upload quota check, using the actual stored size. The declared
    // size is only a hint and is never trusted for quota enforcement.
    #[cfg(feature = "payments")]
    {
        let _ = upload_size;
        if is_new_file && let Some(payment_config) = &settings.payments {
            let free_quota = payment_config.free_quota_bytes.unwrap_or(104857600); // Default to 100MB

            match state
                .db
                .check_user_quota(&pubkey_vec, upload.size, free_quota)
                .await
            {
                Ok(false) => {
                    if let Err(e) = state.fs.delete(&upload.id).await {
                        log::warn!("Failed to cleanup quota-exceeding file: {}", e);
                    }
                    return Nip96Response::error("Upload would exceed quota");
                }
                Err(_) => {
                    if let Err(e) = state.fs.delete(&upload.id).await {
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

    Nip96Response::UploadResult(Json(Nip96UploadResult::from_upload(&settings, &upload)))
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
    let server_count = query.count.clamp(1, 5_000);
    match state
        .db
        .list_files(&pubkey_vec, query.page * server_count, server_count)
        .await
    {
        Ok((files, total)) => {
            let settings = state.settings().await;
            Nip96Response::FileList(Json(PagedResult {
                count: server_count,
                page: query.page,
                total: total as u32,
                files: files
                    .iter()
                    .map(|f| {
                        Nip96UploadResult::from_upload(&settings, f)
                            .nip94_event
                            .unwrap()
                    })
                    .collect(),
            }))
        }
        Err(e) => Nip96Response::error(&format!("Could not list files: {}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::file_stats::FileStatsTracker;
    use crate::filesystem::FileStore;
    use crate::whitelist::Whitelist;
    use axum::body::Body;
    use base64::Engine;
    use base64::prelude::BASE64_STANDARD;
    use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, TagKind};
    use sqlx::mysql::MySqlPoolOptions;
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    fn test_state(max_upload_bytes: u64) -> (Arc<AppState>, tempfile::TempDir) {
        let temp = tempfile::tempdir().unwrap();
        let settings = Settings {
            listen: None,
            storage_dir: temp.path().to_string_lossy().into_owned(),
            database: "mysql://user:pass@127.0.0.1:1/route96".to_string(),
            max_upload_bytes,
            public_url: "https://example.com".to_string(),
            whitelist: None,
            #[cfg(feature = "labels")]
            models_dir: None,
            #[cfg(feature = "labels")]
            label_models: None,
            #[cfg(feature = "labels")]
            label_flag_terms: None,
            webhook_url: None,
            reject_sensitive_exif: None,
            reject_steganography: None,
            #[cfg(feature = "media-compression")]
            identical_media_dedup: None,
            #[cfg(feature = "media-compression")]
            identical_media_dedup_distance: None,
            #[cfg(feature = "media-compression")]
            identical_media_dedup_allow_override: None,
            #[cfg(feature = "payments")]
            payments: None,
            delete_unaccessed_days: None,
            delete_after_days: None,
            delete_zero_egress_days: None,
            cold_storage_dir: None,
            cold_storage_min_downloads: None,
            cold_storage_after_days: None,
        };
        let settings = Arc::new(RwLock::new(settings));
        let pool = MySqlPoolOptions::new()
            .acquire_timeout(Duration::from_millis(50))
            .connect_lazy("mysql://user:pass@127.0.0.1:1/route96")
            .unwrap();
        let state = Arc::new(AppState {
            fs: FileStore::new(settings.clone()),
            db: Database::from_pool(pool),
            config_path: String::new(),
            settings,
            wl: Arc::new(RwLock::new(Whitelist::default())),
            file_stats: FileStatsTracker::new(),
            #[cfg(feature = "payments")]
            lnd: None,
        });
        (state, temp)
    }

    fn auth_header() -> String {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags([
                Tag::custom(TagKind::u(), ["http://localhost/n96"]),
                Tag::custom(TagKind::Method, ["POST"]),
            ])
            .sign_with_keys(&keys)
            .unwrap();
        format!("Nostr {}", BASE64_STANDARD.encode(event.as_json()))
    }

    #[tokio::test]
    async fn upload_over_default_body_limit_is_parsed() {
        let (state, _temp) = test_state(10 * 1024 * 1024);
        let boundary = "XBOUNDARYX";
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.bin\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .into_bytes();
        body.extend(vec![7u8; 3 * 1024 * 1024]);
        body.extend(format!("\r\n--{boundary}--\r\n").into_bytes());

        let response = nip96_routes()
            .with_state(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/n96")
                    .header("authorization", auth_header())
                    .header(
                        "content-type",
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("Could not parse form"), "{text}");
    }
}
