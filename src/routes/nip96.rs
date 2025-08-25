use std::collections::HashMap;
use std::ops::Sub;
use std::time::Duration;

use log::error;
use nostr::Timestamp;
use rocket::data::ToByteUnit;
use rocket::form::Form;
use rocket::fs::TempFile;
use rocket::serde::json::Json;
use rocket::serde::Serialize;
use rocket::{routes, FromForm, Responder, Route, State};

use crate::auth::nip98::Nip98Auth;
use crate::db::{Database, FileUpload};
use crate::filesystem::{FileStore, FileSystemResult};
use crate::routes::{delete_file, Nip94Event, PagedResult};
use crate::settings::Settings;
use crate::whitelist::Whitelist;

#[derive(Serialize, Default)]
#[serde(crate = "rocket::serde")]
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
#[serde(crate = "rocket::serde")]
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
#[serde(crate = "rocket::serde")]
struct Nip96MediaTransformations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video: Option<Vec<String>>,
}

#[derive(Responder)]
enum Nip96Response {
    #[response(status = 500)]
    GenericError(Json<Nip96UploadResult>),

    #[response(status = 200)]
    UploadResult(Json<Nip96UploadResult>),

    #[response(status = 200)]
    FileList(Json<PagedResult<Nip94Event>>),

    #[response(status = 403)]
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

#[derive(Serialize, Default)]
#[serde(crate = "rocket::serde")]
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

#[derive(FromForm)]
struct Nip96Form<'r> {
    file: TempFile<'r>,
    expiration: Option<usize>,
    size: u64,
    alt: Option<&'r str>,
    caption: Option<&'r str>,
    content_type: Option<&'r str>,
    no_transform: Option<bool>,
}

pub fn nip96_routes() -> Vec<Route> {
    routes![get_info_doc, upload, delete, list_files]
}

#[rocket::get("/.well-known/nostr/nip96.json")]
async fn get_info_doc(settings: &State<Settings>) -> Json<Nip96InfoDoc> {
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

#[rocket::post("/n96", data = "<form>")]
async fn upload(
    auth: Nip98Auth,
    fs: &State<FileStore>,
    db: &State<Database>,
    settings: &State<Settings>,
    whitelist: &State<Whitelist>,
    form: Form<Nip96Form<'_>>,
) -> Nip96Response {
    let upload_size = auth.content_length.or(Some(form.size)).unwrap_or(0);
    if upload_size > 0 && upload_size > settings.max_upload_bytes {
        return Nip96Response::error("File too large");
    }
    let file = match form.file.open().await {
        Ok(f) => f,
        Err(e) => return Nip96Response::error(&format!("Could not open file: {}", e)),
    };
    let content_type = form.content_type.unwrap_or("application/octet-stream");

    if form.expiration.is_some() {
        return Nip96Response::error("Expiration not supported");
    }

    // account for upload speeds as slow as 1MB/s (8 Mbps)
    let size_for_timing = if upload_size > 0 { upload_size } else { form.size };
    let mbs = size_for_timing / 1.megabytes().as_u64();
    let max_time = 60.max(mbs);
    if auth.event.created_at < Timestamp::now().sub(Duration::from_secs(max_time)) {
        return Nip96Response::error("Auth event timestamp out of range");
    }

    // check whitelist
    if !whitelist.contains_hex(&auth.event.pubkey.to_hex()) {
        return Nip96Response::Forbidden(Json(Nip96UploadResult::error("Not on whitelist")));
    }

    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();

    // check quota (only if payments are configured)
    #[cfg(feature = "payments")]
    if let Some(payment_config) = &settings.payments {
        let free_quota = payment_config.free_quota_bytes
            .unwrap_or(104857600); // Default to 100MB
        
        if upload_size > 0 {
            match db.check_user_quota(&pubkey_vec, upload_size, free_quota).await {
                Ok(false) => return Nip96Response::error("Upload would exceed quota"),
                Err(_) => return Nip96Response::error("Failed to check quota"),
                Ok(true) => {} // Quota check passed
            }
        }
    }
    let upload = match fs
        .put(file, content_type, !form.no_transform.unwrap_or(false))
        .await
    {
        Ok(FileSystemResult::NewFile(blob)) => {
            let mut upload: FileUpload = (&blob).into();
            
            // Validate file size after upload if no pre-upload size was available
            if upload_size == 0 && upload.size > settings.max_upload_bytes {
                // Clean up the uploaded file
                if let Err(e) = tokio::fs::remove_file(fs.get(&upload.id)).await {
                    log::warn!("Failed to cleanup oversized file: {}", e);
                }
                return Nip96Response::error("File too large");
            }
            
            upload.name = form.caption.map(|cap| cap.to_string());
            upload.alt = form.alt.as_ref().map(|s| s.to_string());
            upload
        }
        Ok(FileSystemResult::AlreadyExists(i)) => match db.get_file(&i).await {
            Ok(Some(f)) => f,
            _ => return Nip96Response::error("File not found"),
        },
        Err(e) => {
            error!("{}", e);
            return Nip96Response::error(&format!("Could not save file: {}", e));
        }
    };

    let user_id = match db.upsert_user(&pubkey_vec).await {
        Ok(u) => u,
        Err(e) => return Nip96Response::error(&format!("Could not save user: {}", e)),
    };

    // Post-upload quota check if we didn't have size information before upload (only if payments are configured)
    #[cfg(feature = "payments")]
    if upload_size == 0 {
        if let Some(payment_config) = &settings.payments {
            let free_quota = payment_config.free_quota_bytes
                .unwrap_or(104857600); // Default to 100MB
            
            match db.check_user_quota(&pubkey_vec, upload.size, free_quota).await {
                Ok(false) => {
                    // Clean up the uploaded file if quota exceeded
                    if let Err(e) = tokio::fs::remove_file(fs.get(&upload.id)).await {
                        log::warn!("Failed to cleanup quota-exceeding file: {}", e);
                    }
                    return Nip96Response::error("Upload would exceed quota");
                }
                Err(_) => {
                    // Clean up on quota check error
                    if let Err(e) = tokio::fs::remove_file(fs.get(&upload.id)).await {
                        log::warn!("Failed to cleanup file after quota check error: {}", e);
                    }
                    return Nip96Response::error("Failed to check quota");
                }
                Ok(true) => {} // Quota check passed
            }
        }
    }

    if let Err(e) = db.add_file(&upload, Some(user_id)).await {
        error!("{}", e);
        return Nip96Response::error(&format!("Could not save file (db): {}", e));
    }
    Nip96Response::UploadResult(Json(Nip96UploadResult::from_upload(settings, &upload)))
}

#[rocket::delete("/n96/<sha256>")]
async fn delete(
    sha256: &str,
    auth: Nip98Auth,
    fs: &State<FileStore>,
    db: &State<Database>,
) -> Nip96Response {
    match delete_file(sha256, &auth.event, fs, db).await {
        Ok(()) => Nip96Response::success("File deleted."),
        Err(e) => Nip96Response::error(&format!("Failed to delete file: {}", e)),
    }
}

#[rocket::get("/n96?<page>&<count>")]
async fn list_files(
    auth: Nip98Auth,
    page: u32,
    count: u32,
    db: &State<Database>,
    settings: &State<Settings>,
) -> Nip96Response {
    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
    let server_count = count.min(5_000).max(1);
    match db
        .list_files(&pubkey_vec, page * server_count, server_count)
        .await
    {
        Ok((files, total)) => Nip96Response::FileList(Json(PagedResult {
            count: server_count,
            page,
            total: total as u32,
            files: files
                .iter()
                .map(|f| {
                    Nip96UploadResult::from_upload(settings, f)
                        .nip94_event
                        .unwrap()
                })
                .collect(),
        })),
        Err(e) => Nip96Response::error(&format!("Could not list files: {}", e)),
    }
}
