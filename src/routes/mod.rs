use std::fs;
use std::fs::File;
use std::str::FromStr;

use anyhow::Error;
use nostr::Event;
use rocket::fs::NamedFile;
use rocket::http::{ContentType, Header, Status};
use rocket::response::Responder;
use rocket::{Request, State};

use crate::db::{Database, FileUpload};
use crate::filesystem::FileStore;
pub use crate::routes::blossom::blossom_routes;
pub use crate::routes::nip96::nip96_routes;

mod blossom;
mod nip96;

pub struct FilePayload {
    pub file: File,
    pub info: FileUpload,
}

impl<'r> Responder<'r, 'static> for FilePayload {
    fn respond_to(self, request: &'r Request<'_>) -> rocket::response::Result<'static> {
        let mut response = self.file.respond_to(request)?;
        if let Ok(ct) = ContentType::from_str(&self.info.mime_type) {
            response.set_header(ct);
        }
        response.set_header(Header::new(
            "content-disposition",
            format!("inline; filename=\"{}\"", self.info.name),
        ));
        Ok(response)
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
    if let Ok(Some(info)) = db.get_file(&id).await {
        let pubkey_vec = auth.pubkey.to_bytes().to_vec();
        let user = match db.get_user_id(&pubkey_vec).await {
            Ok(u) => u,
            Err(_e) => return Err(Error::msg("User not found")),
        };
        if user != info.user_id {
            return Err(Error::msg("You dont own this file, you cannot delete it"));
        }
        if let Err(e) = db.delete_file(&id).await {
            return Err(Error::msg(format!("Failed to delete (db): {}", e)));
        }
        if let Err(e) = fs::remove_file(fs.get(&id)) {
            return Err(Error::msg(format!("Failed to delete (fs): {}", e)));
        }
        Ok(())
    } else {
        Err(Error::msg("File not found"))
    }
}

#[rocket::get("/")]
pub async fn root() -> Result<NamedFile, Status> {
    if let Ok(f) = NamedFile::open("./ui/index.html").await {
        Ok(f)
    } else {
        Err(Status::InternalServerError)
    }
}

#[rocket::get("/<sha256>")]
pub async fn get_blob(
    sha256: &str,
    fs: &State<FileStore>,
    db: &State<Database>,
) -> Result<FilePayload, Status> {
    let sha256 = if sha256.contains(".") {
        sha256.split('.').next().unwrap()
    } else {
        sha256
    };
    let id = if let Ok(i) = hex::decode(sha256) {
        i
    } else {
        return Err(Status::NotFound);
    };

    if id.len() != 32 {
        return Err(Status::NotFound);
    }
    if let Ok(Some(info)) = db.get_file(&id).await {
        if let Ok(f) = File::open(fs.get(&id)) {
            return Ok(FilePayload { file: f, info });
        }
    }
    return Err(Status::NotFound);
}

#[rocket::head("/<sha256>")]
pub async fn head_blob(sha256: &str, fs: &State<FileStore>) -> Status {
    let sha256 = if sha256.contains(".") {
        sha256.split('.').next().unwrap()
    } else {
        sha256
    };
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
