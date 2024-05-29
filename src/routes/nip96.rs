use std::collections::HashMap;
use std::fs;

use log::error;
use rocket::{FromForm, Responder, Route, routes, State};
use rocket::form::Form;
use rocket::fs::TempFile;
use rocket::serde::json::Json;
use rocket::serde::Serialize;

use crate::auth::nip98::Nip98Auth;
use crate::db::{Database, FileUpload};
use crate::filesystem::FileStore;
use crate::routes::delete_file;
use crate::settings::Settings;
use crate::webhook::Webhook;

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
    pub max_byte_size: usize,
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

#[derive(Serialize, Default)]
#[serde(crate = "rocket::serde")]
struct Nip96FileListResults {
    pub count: u32,
    pub page: u32,
    pub total: u32,
    pub files: Vec<Nip94Event>,
}

#[derive(Responder)]
enum Nip96Response {
    #[response(status = 500)]
    GenericError(Json<Nip96UploadResult>),

    #[response(status = 200)]
    UploadResult(Json<Nip96UploadResult>),

    #[response(status = 200)]
    FileList(Json<Nip96FileListResults>),
}

impl Nip96Response {
    fn error(msg: &str) -> Self {
        Nip96Response::GenericError(Json(Nip96UploadResult {
            status: "error".to_string(),
            message: Some(msg.to_string()),
            ..Default::default()
        }))
    }

    fn success(msg: &str) -> Self {
        Nip96Response::UploadResult(Json(Nip96UploadResult {
            status: "success".to_string(),
            message: Some(msg.to_string()),
            ..Default::default()
        }))
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
        let hex_id = hex::encode(&upload.id);
        let mut tags = vec![
            vec![
                "url".to_string(),
                format!("{}/{}", &settings.public_url, &hex_id),
            ],
            vec!["x".to_string(), hex_id],
            vec!["m".to_string(), upload.mime_type.clone()],
        ];
        if let Some(bh) = &upload.blur_hash {
            tags.push(vec!["blurhash".to_string(), bh.clone()]);
        }
        if let (Some(w), Some(h)) = (upload.width, upload.height) {
            tags.push(vec!["dim".to_string(), format!("{}x{}", w, h)])
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
            status: "success".to_string(),
            nip94_event: Some(Nip94Event {
                content: upload.name.clone(),
                created_at: upload.created.timestamp(),
                tags,
            }),
            ..Default::default()
        }
    }
}

#[derive(Serialize, Default)]
#[serde(crate = "rocket::serde")]
struct Nip94Event {
    pub created_at: i64,
    pub content: String,
    pub tags: Vec<Vec<String>>,
}

#[derive(FromForm)]
struct Nip96Form<'r> {
    file: TempFile<'r>,
    expiration: Option<usize>,
    size: usize,
    alt: Option<&'r str>,
    caption: Option<&'r str>,
    media_type: Option<&'r str>,
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
    webhook: &State<Option<Webhook>>,
    form: Form<Nip96Form<'_>>,
) -> Nip96Response {
    if let Some(size) = auth.content_length {
        if size > settings.max_upload_bytes {
            return Nip96Response::error("File too large");
        }
    }
    if form.size > settings.max_upload_bytes {
        return Nip96Response::error("File too large");
    }
    let file = match form.file.open().await {
        Ok(f) => f,
        Err(e) => return Nip96Response::error(&format!("Could not open file: {}", e)),
    };
    let mime_type = form.media_type
        .unwrap_or("application/octet-stream");

    if form.expiration.is_some() {
        return Nip96Response::error("Expiration not supported");
    }
    if form.alt.is_some() {
        return Nip96Response::error("\"alt\" is not supported");
    }
    
    // check whitelist
    if let Some(wl) = &settings.whitelist {
        if !wl.contains(&auth.event.pubkey.to_hex()) {
            return Nip96Response::error("Not on whitelist");
        }
    }
    match fs
        .put(file, mime_type, !form.no_transform.unwrap_or(false))
        .await
    {
        Ok(mut blob) => {
            blob.upload.name = match &form.caption {
                Some(c) => c.to_string(),
                None => "".to_string(),
            };
            let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
            if let Some(wh) = webhook.as_ref() {
                match wh.store_file(&pubkey_vec, blob.clone()) {
                    Ok(store) => if !store {
                        let _ = fs::remove_file(blob.path);
                        return Nip96Response::error("Upload rejected");
                    }
                    Err(e) => {
                        let _ = fs::remove_file(blob.path);
                        return Nip96Response::error(&format!("Internal error, failed to call webhook: {}", e));
                    }
                }
            }
            let user_id = match db.upsert_user(&pubkey_vec).await {
                Ok(u) => u,
                Err(e) => return Nip96Response::error(&format!("Could not save user: {}", e)),
            };
            let tmp_file = blob.path.clone();
            if let Err(e) = db.add_file(&blob.upload, user_id).await {
                error!("{}", e.to_string());
                let _ = fs::remove_file(tmp_file);
                if let Some(dbe) = e.as_database_error() {
                    if let Some(c) = dbe.code() {
                        if c == "23000" {
                            return Nip96Response::error("File already exists");
                        }
                    }
                }
                return Nip96Response::error(&format!("Could not save file (db): {}", e));
            }

            Nip96Response::UploadResult(Json(Nip96UploadResult::from_upload(settings, &blob.upload)))
        }
        Err(e) => {
            error!("{}", e.to_string());
            Nip96Response::error(&format!("Could not save file: {}", e))
        }
    }
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
    match db.list_files(&pubkey_vec, page * server_count, server_count).await {
        Ok((files, total)) => Nip96Response::FileList(Json(Nip96FileListResults {
            count: server_count,
            page,
            total: total as u32,
            files: files
                .iter()
                .map(|f| Nip96UploadResult::from_upload(settings, f).nip94_event.unwrap())
                .collect(),
        }
        )),
        Err(e) => Nip96Response::error(&format!("Could not list files: {}", e)),
    }
}
