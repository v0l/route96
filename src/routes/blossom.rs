use std::sync::{Mutex, RwLock};
use chrono::Utc;
use log::{error};
use nostr::prelude::hex;
use nostr::{Tag};
use rocket::data::{ByteUnit};

use rocket::http::{Status};
use rocket::response::Responder;
use rocket::serde::json::Json;
use rocket::{routes, Data, Route, State};
use serde::{Deserialize, Serialize};

use crate::auth::blossom::BlossomAuth;
use crate::blob::BlobDescriptor;
use crate::db::{Database, FileUpload};
use crate::filesystem::FileStore;
use crate::routes::{delete_file};
use crate::settings::Settings;

#[derive(Serialize, Deserialize)]
struct BlossomError {
    pub message: String,
}

pub fn blossom_routes() -> Vec<Route> {
    routes![delete_blob, upload, list_files]
}

impl BlossomError {
    pub fn new(msg: String) -> Self {
        Self { message: msg }
    }
}

#[derive(Responder)]
enum BlossomResponse {
    #[response(status = 500)]
    GenericError(Json<BlossomError>),

    #[response(status = 200)]
    BlobDescriptor(Json<BlobDescriptor>),

    #[response(status = 200)]
    BlobDescriptorList(Json<Vec<BlobDescriptor>>),

    StatusOnly(Status),
}

impl BlossomResponse {
    pub fn error(msg: impl Into<String>) -> Self {
        Self::GenericError(Json(BlossomError::new(msg.into())))
    }
}

fn check_method(event: &nostr::Event, method: &str) -> bool {
    if let Some(t) = event.tags.iter().find_map(|t| match t {
        Tag::Hashtag(tag) => Some(tag),
        _ => None,
    }) {
        return t == method;
    }
    false
}

#[rocket::delete("/<sha256>")]
async fn delete_blob(
    sha256: &str,
    auth: BlossomAuth,
    fs: &State<FileStore>,
    db: &State<Database>,
) -> BlossomResponse {
    match delete_file(sha256, &auth.event, fs, db).await {
        Ok(()) => BlossomResponse::StatusOnly(Status::Ok),
        Err(e) => BlossomResponse::error(format!("Failed to delete file: {}", e)),
    }
}

#[rocket::put("/upload", data = "<data>")]
async fn upload(
    auth: BlossomAuth,
    fs: &State<FileStore>,
    db: &State<Database>,
    settings: &State<Settings>,
    data: Data<'_>,
) -> BlossomResponse {
    if !check_method(&auth.event, "upload") {
        return BlossomResponse::error("Invalid request method tag");
    }

    let name = auth.event.tags.iter().find_map(|t| match t {
        Tag::Name(s) => Some(s.clone()),
        _ => None,
    });
    let size = auth.event.tags.iter().find_map(|t| {
        let values = t.as_vec();
        if values.len() == 2 && values[0] == "size" {
            Some(values[1].parse::<usize>().unwrap())
        } else {
            None
        }
    });
    if size.is_none() {
        return BlossomResponse::error("Invalid request, no size tag");
    }
    let mime_type = auth
        .content_type
        .unwrap_or("application/octet-stream".to_string());
    
    match fs
        .put(data.open(ByteUnit::from(settings.max_upload_bytes)), &mime_type)
        .await
    {
        Ok(blob) => {
            let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
            let user_id = match db.upsert_user(&pubkey_vec).await {
                Ok(u) => u,
                Err(e) => {
                    return BlossomResponse::error(format!("Failed to save file (db): {}", e))
                }
            };
            let f = FileUpload {
                id: blob.sha256,
                user_id,
                name: name.unwrap_or("".to_string()),
                size: blob.size,
                mime_type,
                created: Utc::now(),
            };
            if let Err(e) = db.add_file(&f).await {
                error!("{:?}", e);
                BlossomResponse::error(format!("Error saving file (db): {}", e))
            } else {
                BlossomResponse::BlobDescriptor(Json(BlobDescriptor::from_upload(
                    &f,
                    &settings.public_url,
                )))
            }
        }
        Err(e) => {
            error!("{:?}", e);
            BlossomResponse::error(format!("Error saving file (disk): {}", e))
        }
    }
}

#[rocket::get("/list/<pubkey>")]
async fn list_files(
    db: &State<Database>,
    settings: &State<Settings>,
    pubkey: &str,
) -> BlossomResponse {
    let id = if let Ok(i) = hex::decode(pubkey) {
        i
    } else {
        return BlossomResponse::error("invalid pubkey");
    };
    match db.list_files(&id).await {
        Ok(files) => BlossomResponse::BlobDescriptorList(Json(
            files
                .iter()
                .map(|f| BlobDescriptor::from_upload(&f, &settings.public_url))
                .collect(),
        )),
        Err(e) => BlossomResponse::error(format!("Could not list files: {}", e)),
    }
}
