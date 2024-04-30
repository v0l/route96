use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Utc;
use log::{error, info};
use nostr::{JsonUtil, Tag, TagKind};
use nostr::prelude::hex;
use rocket::{async_trait, Data, Request, Route, routes, State, uri};
use rocket::data::ToByteUnit;
use rocket::fs::NamedFile;
use rocket::http::{ContentType, Header, Status};
use rocket::http::hyper::header::CONTENT_DISPOSITION;
use rocket::request::{FromRequest, Outcome};
use rocket::response::Responder;
use rocket::response::status::NotFound;
use rocket::serde::json::Json;
use serde::{Deserialize, Serialize};

use crate::auth::BlossomAuth;
use crate::blob::BlobDescriptor;
use crate::db::{Database, FileUpload};
use crate::filesystem::FileStore;
use crate::routes::BlossomResponse::BlobDescriptorList;

pub fn all() -> Vec<Route> {
    routes![root, get_blob, head_blob, delete_blob, upload, list_files]
}

#[derive(Serialize, Deserialize)]
struct BlossomError {
    pub message: String,
}

impl BlossomError {
    pub fn new(msg: String) -> Self {
        Self { message: msg }
    }
}

struct BlossomFile {
    pub file: File,
    pub info: FileUpload,
}

impl<'r> Responder<'r, 'static> for BlossomFile {
    fn respond_to(self, request: &'r Request<'_>) -> rocket::response::Result<'static> {
        let mut response = self.file.respond_to(request)?;
        if let Ok(ct) = ContentType::from_str(&self.info.mime_type) {
            response.set_header(ct);
        }
        response.set_header(Header::new("content-disposition", format!("inline; filename=\"{}\"", self.info.name)));
        Ok(response)
    }
}

#[derive(Responder)]
enum BlossomResponse {
    #[response(status = 403)]
    Unauthorized(Json<BlossomError>),

    #[response(status = 500)]
    GenericError(Json<BlossomError>),

    #[response(status = 200)]
    File(BlossomFile),

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

#[rocket::get("/")]
async fn root() -> &'static str {
    "Hello welcome to void_cat_rs"
}

#[rocket::get("/<sha256>")]
async fn get_blob(sha256: &str, fs: &State<FileStore>, db: &State<Database>) -> BlossomResponse {
    let sha256 = if sha256.contains(".") {
        sha256.split('.').next().unwrap()
    } else {
        sha256
    };
    let id = if let Ok(i) = hex::decode(sha256) {
        i
    } else {
        return BlossomResponse::error("Invalid file id");
    };

    if id.len() != 32 {
        return BlossomResponse::error("Invalid file id");
    }
    if let Ok(Some(info)) = db.get_file(&id).await {
        if let Ok(f) = File::open(fs.get(&id)) {
            return BlossomResponse::File(BlossomFile {
                file: f,
                info,
            });
        }
    }
    BlossomResponse::StatusOnly(Status::NotFound)
}

#[rocket::head("/<sha256>")]
async fn head_blob(sha256: &str, fs: &State<FileStore>) -> BlossomResponse {
    let sha256 = if sha256.contains(".") {
        sha256.split('.').next().unwrap()
    } else {
        sha256
    };
    let id = if let Ok(i) = hex::decode(sha256) {
        i
    } else {
        return BlossomResponse::error("Invalid file id");
    };

    if id.len() != 32 {
        return BlossomResponse::error("Invalid file id");
    }
    if fs.get(&id).exists() {
        BlossomResponse::StatusOnly(Status::Ok)
    } else {
        BlossomResponse::StatusOnly(Status::NotFound)
    }
}

#[rocket::delete("/<sha256>")]
async fn delete_blob(sha256: &str, fs: &State<FileStore>, db: &State<Database>) -> BlossomResponse {
    let sha256 = if sha256.contains(".") {
        sha256.split('.').next().unwrap()
    } else {
        sha256
    };
    let id = if let Ok(i) = hex::decode(sha256) {
        i
    } else {
        return BlossomResponse::error("Invalid file id");
    };

    if id.len() != 32 {
        return BlossomResponse::error("Invalid file id");
    }
    if let Ok(Some(_info)) = db.get_file(&id).await {
        db.delete_file(&id).await?;
        fs::remove_file(fs.get(&id))?;
        BlossomResponse::StatusOnly(Status::Ok)
    } else {
        BlossomResponse::StatusOnly(Status::NotFound)
    }
}

#[rocket::put("/upload", data = "<data>")]
async fn upload(auth: BlossomAuth, fs: &State<FileStore>, db: &State<Database>, data: Data<'_>)
                -> BlossomResponse {
    if !check_method(&auth.event, "upload") {
        return BlossomResponse::error("Invalid request method tag");
    }

    let name = auth.event.tags.iter().find_map(|t| match t {
        Tag::Name(s) => Some(s.clone()),
        _ => None
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
    match fs.put(data.open(8.gigabytes())).await {
        Ok(blob) => {
            let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
            let user_id = match db.upsert_user(&pubkey_vec).await {
                Ok(u) => u,
                Err(e) => return BlossomResponse::error(format!("Failed to save file (db): {}", e))
            };
            let f = FileUpload {
                id: blob.sha256,
                user_id: user_id as u64,
                name: name.unwrap_or("".to_string()),
                size: blob.size,
                mime_type: auth.content_type.unwrap_or("application/octet-stream".to_string()),
                created: Utc::now(),
            };
            if let Err(e) = db.add_file(&f).await {
                error!("{:?}", e);
                BlossomResponse::error(format!("Error saving file (db): {}", e))
            } else {
                BlossomResponse::BlobDescriptor(Json(BlobDescriptor::from(&f)))
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
    pubkey: &str,
) -> BlossomResponse {
    let id = if let Ok(i) = hex::decode(pubkey) {
        i
    } else {
        return BlossomResponse::error("invalid pubkey");
    };
    match db.list_files(&id).await {
        Ok(files) => BlobDescriptorList(Json(files.iter()
            .map(|f| BlobDescriptor::from(f))
            .collect())
        ),
        Err(e) => BlossomResponse::error(format!("Could not list files: {}", e))
    }
}