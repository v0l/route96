use crate::auth::BlossomAuth;
use crate::blob::BlobDescriptor;
use crate::db::Database;
use crate::filesystem::FileStore;
use nostr::prelude::hex;
use nostr::Tag;
use rocket::data::ToByteUnit;
use rocket::fs::NamedFile;
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome};
use rocket::response::status::NotFound;
use rocket::serde::json::Json;
use rocket::{async_trait, routes, Data, Request, Route, State};

pub fn all() -> Vec<Route> {
    routes![root, get_blob, get_blob_check, upload, list_files]
}

fn check_method(event: &nostr::Event, method: &str) -> bool {
    if let Some(t) = event.tags.iter().find_map(|t| match t {
        Tag::Hashtag(tag) => Some(tag),
        _ => None,
    }) {
        if t == method {
            return false;
        }
    }
    false
}

#[rocket::get("/")]
async fn root() -> &'static str {
    "Hello welcome to void_cat_rs"
}

#[rocket::get("/<sha256>")]
async fn get_blob(sha256: &str, fs: &State<FileStore>) -> Result<NamedFile, Status> {
    let id = if let Ok(i) = hex::decode(sha256) {
        i
    } else {
        return Err(Status::NotFound);
    };

    if id.len() != 32 {
        return Err(Status::NotFound);
    }
    if let Ok(f) = NamedFile::open(fs.get(&id)).await {
        Ok(f)
    } else {
        Err(Status::NotFound)
    }
}

#[rocket::head("/<sha256>")]
async fn get_blob_check(sha256: &str, fs: &State<FileStore>) -> Status {
    let id = if let Ok(i) = hex::decode(sha256) {
        i
    } else {
        return Status::NotFound;
    };

    if id.len() != 32 {
        return Status::NotFound;
    }
    if fs.get(&id).exists() {
        Status::Ok
    } else {
        Status::NotFound
    }
}

#[rocket::put("/upload", data = "<data>")]
async fn upload(auth: BlossomAuth, fs: &State<FileStore>, data: Data<'_>) -> Status {
    if !check_method(&auth.event, "upload") {
        return Status::NotFound;
    }

    match fs.put(data.open(8.gigabytes())).await {
        Ok(blob) => Status::Ok,
        Err(e) => Status::InternalServerError,
    }
}

#[rocket::get("/list/<pubkey>")]
async fn list_files(
    auth: BlossomAuth,
    db: &State<Database>,
    pubkey: String,
) -> Result<Json<Vec<BlobDescriptor>>, Status> {
    if !check_method(&auth.event, "list") {
        return Err(Status::NotFound);
    }
    let id = if let Ok(i) = hex::decode(pubkey) {
        i
    } else {
        return Err(Status::NotFound);
    };
    if let Ok(files) = db.list_files(&id).await {
        Ok(Json(
            files
                .iter()
                .map(|f| BlobDescriptor {
                    url: "".to_string(),
                    sha256: hex::encode(&f.id),
                    size: f.size,
                    mime_type: None,
                    created: f.created,
                })
                .collect(),
        ))
    } else {
        Err(Status::InternalServerError)
    }
}
