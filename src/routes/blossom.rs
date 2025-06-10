use crate::auth::blossom::BlossomAuth;
use crate::db::{Database, FileUpload};
use crate::filesystem::{FileStore, FileSystemResult};
use crate::routes::{delete_file, Nip94Event};
use crate::settings::Settings;
use log::error;
use nostr::prelude::hex;
use nostr::{Alphabet, SingleLetterTag, TagKind, JsonUtil};
use rocket::data::ByteUnit;
use rocket::futures::StreamExt;
use rocket::http::{Header, Status};
use rocket::response::Responder;
use rocket::serde::json::Json;
use rocket::{routes, Data, Request, Response, Route, State};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncRead;
use tokio_util::io::StreamReader;

#[derive(Debug, Clone, Serialize)]
#[serde(crate = "rocket::serde")]
pub struct BlobDescriptor {
    pub url: String,
    pub sha256: String,
    pub size: u64,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub created: u64,
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
            created: value.created.timestamp() as u64,
            nip94: Some(Nip94Event::from_upload(settings, value).tags),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MirrorRequest {
    pub url: String,
}

#[cfg(feature = "media-compression")]
pub fn blossom_routes() -> Vec<Route> {
    routes![
        delete_blob,
        upload,
        list_files,
        upload_head,
        upload_media,
        head_media,
        mirror,
        report_file
    ]
}

#[cfg(not(feature = "media-compression"))]
pub fn blossom_routes() -> Vec<Route> {
    routes![delete_blob, upload, list_files, upload_head, mirror, report_file]
}

/// Generic holder response, mostly for errors
struct BlossomGenericResponse {
    pub message: Option<String>,
    pub status: Status,
}

impl<'r> Responder<'r, 'static> for BlossomGenericResponse {
    fn respond_to(self, _request: &'r Request<'_>) -> rocket::response::Result<'static> {
        let mut r = Response::new();
        r.set_status(self.status);
        if let Some(message) = self.message {
            r.set_raw_header("X-Reason", message);
        }
        Ok(r)
    }
}
#[derive(Responder)]
enum BlossomResponse {
    Generic(BlossomGenericResponse),

    #[response(status = 200)]
    BlobDescriptor(Json<BlobDescriptor>),

    #[response(status = 200)]
    BlobDescriptorList(Json<Vec<BlobDescriptor>>),
}

impl BlossomResponse {
    pub fn error(msg: impl Into<String>) -> Self {
        Self::Generic(BlossomGenericResponse {
            message: Some(msg.into()),
            status: Status::InternalServerError,
        })
    }
}

struct BlossomHead {
    pub msg: Option<&'static str>,
}

impl<'r> Responder<'r, 'static> for BlossomHead {
    fn respond_to(self, _request: &'r Request<'_>) -> rocket::response::Result<'static> {
        let mut response = Response::new();
        match self.msg {
            Some(m) => {
                response.set_status(Status::InternalServerError);
                response.set_header(Header::new("x-upload-message", m));
            }
            None => {
                response.set_status(Status::Ok);
            }
        }
        Ok(response)
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

fn check_whitelist(auth: &BlossomAuth, settings: &Settings) -> Option<BlossomResponse> {
    // check whitelist
    if let Some(wl) = &settings.whitelist {
        if !wl.contains(&auth.event.pubkey.to_hex()) {
            return Some(BlossomResponse::Generic(BlossomGenericResponse {
                status: Status::Forbidden,
                message: Some("Not on whitelist".to_string()),
            }));
        }
    }
    None
}

#[rocket::delete("/<sha256>")]
async fn delete_blob(
    sha256: &str,
    auth: BlossomAuth,
    fs: &State<FileStore>,
    db: &State<Database>,
) -> BlossomResponse {
    match delete_file(sha256, &auth.event, fs, db).await {
        Ok(()) => BlossomResponse::Generic(BlossomGenericResponse {
            status: Status::Ok,
            message: None,
        }),
        Err(e) => BlossomResponse::error(format!("Failed to delete file: {}", e)),
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
    match db.list_files(&id, 0, 10_000).await {
        Ok((files, _count)) => BlossomResponse::BlobDescriptorList(Json(
            files
                .iter()
                .map(|f| BlobDescriptor::from_upload(settings, f))
                .collect(),
        )),
        Err(e) => BlossomResponse::error(format!("Could not list files: {}", e)),
    }
}

#[rocket::head("/upload")]
fn upload_head(auth: BlossomAuth, settings: &State<Settings>) -> BlossomHead {
    check_head(auth, settings)
}

#[rocket::put("/upload", data = "<data>")]
async fn upload(
    auth: BlossomAuth,
    fs: &State<FileStore>,
    db: &State<Database>,
    settings: &State<Settings>,
    data: Data<'_>,
) -> BlossomResponse {
    process_upload("upload", false, auth, fs, db, settings, data).await
}

#[rocket::put("/mirror", data = "<req>", format = "json")]
async fn mirror(
    auth: BlossomAuth,
    fs: &State<FileStore>,
    db: &State<Database>,
    settings: &State<Settings>,
    req: Json<MirrorRequest>,
) -> BlossomResponse {
    if !check_method(&auth.event, "mirror") {
        return BlossomResponse::error("Invalid request method tag");
    }
    if let Some(e) = check_whitelist(&auth, settings) {
        return e;
    }

    // download file
    let rsp = match reqwest::get(&req.url).await {
        Err(e) => {
            error!("Error downloading file: {}", e);
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
        StreamReader::new(rsp.bytes_stream().map(|result| {
            result.map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))
        })),
        &mime_type,
        &None,
        &pubkey,
        false,
        fs,
        db,
        settings,
    )
    .await
}

#[cfg(feature = "media-compression")]
#[rocket::head("/media")]
fn head_media(auth: BlossomAuth, settings: &State<Settings>) -> BlossomHead {
    check_head(auth, settings)
}

#[cfg(feature = "media-compression")]
#[rocket::put("/media", data = "<data>")]
async fn upload_media(
    auth: BlossomAuth,
    fs: &State<FileStore>,
    db: &State<Database>,
    settings: &State<Settings>,
    data: Data<'_>,
) -> BlossomResponse {
    process_upload("media", true, auth, fs, db, settings, data).await
}

fn check_head(auth: BlossomAuth, settings: &State<Settings>) -> BlossomHead {
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
    if let Some(wl) = &settings.whitelist {
        if !wl.contains(&auth.event.pubkey.to_hex()) {
            return BlossomHead {
                msg: Some("Not on whitelist"),
            };
        }
    }

    BlossomHead { msg: None }
}

async fn process_upload(
    method: &str,
    compress: bool,
    auth: BlossomAuth,
    fs: &State<FileStore>,
    db: &State<Database>,
    settings: &State<Settings>,
    data: Data<'_>,
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
    let size = auth.event.tags.iter().find_map(|t| {
        if t.kind() == TagKind::Size {
            t.content().and_then(|v| v.parse::<u64>().ok())
        } else {
            None
        }
    });
    if let Some(z) = size {
        if z > settings.max_upload_bytes {
            return BlossomResponse::error("File too large");
        }
    }

    // check whitelist
    if let Some(e) = check_whitelist(&auth, settings) {
        return e;
    }

    // check quota
    #[cfg(feature = "payments")]
    if let Some(upload_size) = size {
        let free_quota = settings.payments.as_ref()
            .and_then(|p| p.free_quota_bytes)
            .unwrap_or(104857600); // Default to 100MB
        let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
        
        match db.check_user_quota(&pubkey_vec, upload_size, free_quota).await {
            Ok(false) => return BlossomResponse::error("Upload would exceed quota"),
            Err(_) => return BlossomResponse::error("Failed to check quota"),
            Ok(true) => {} // Quota check passed
        }
    }

    process_stream(
        data.open(ByteUnit::Byte(settings.max_upload_bytes)),
        &auth
            .content_type
            .unwrap_or("application/octet-stream".to_string()),
        &name,
        &auth.event.pubkey.to_bytes().to_vec(),
        compress,
        fs,
        db,
        settings,
    )
    .await
}

async fn process_stream<'p, S>(
    stream: S,
    mime_type: &str,
    name: &Option<&str>,
    pubkey: &Vec<u8>,
    compress: bool,
    fs: &State<FileStore>,
    db: &State<Database>,
    settings: &State<Settings>,
) -> BlossomResponse
where
    S: AsyncRead + Unpin + 'p,
{
    let upload = match fs.put(stream, mime_type, compress).await {
        Ok(FileSystemResult::NewFile(blob)) => {
            let mut ret: FileUpload = (&blob).into();

            // update file data before inserting
            ret.name = name.map(|s| s.to_string());

            ret
        }
        Ok(FileSystemResult::AlreadyExists(i)) => match db.get_file(&i).await {
            Ok(Some(f)) => f,
            _ => return BlossomResponse::error("File not found"),
        },
        Err(e) => {
            error!("{}", e.to_string());
            return BlossomResponse::error(format!("Error saving file (disk): {}", e));
        }
    };

    let user_id = match db.upsert_user(pubkey).await {
        Ok(u) => u,
        Err(e) => {
            return BlossomResponse::error(format!("Failed to save file (db): {}", e));
        }
    };
    if let Err(e) = db.add_file(&upload, user_id).await {
        error!("{}", e.to_string());
        BlossomResponse::error(format!("Error saving file (db): {}", e))
    } else {
        BlossomResponse::BlobDescriptor(Json(BlobDescriptor::from_upload(settings, &upload)))
    }
}

#[rocket::put("/report", data = "<data>", format = "json")]
async fn report_file(
    auth: BlossomAuth,
    db: &State<Database>,
    settings: &State<Settings>,
    data: Json<nostr::Event>,
) -> BlossomResponse {
    // Check if the request has the correct method tag
    if !check_method(&auth.event, "report") {
        return BlossomResponse::error("Invalid request method tag");
    }

    // Check whitelist
    if let Some(e) = check_whitelist(&auth, settings) {
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
    match db.get_file(&file_sha256).await {
        Ok(Some(_)) => {}, // File exists, continue
        Ok(None) => return BlossomResponse::error("File not found"),
        Err(e) => return BlossomResponse::error(format!("Failed to check file: {}", e)),
    }

    // Get or create the reporter user
    let reporter_id = match db.upsert_user(&auth.event.pubkey.to_bytes().to_vec()).await {
        Ok(user_id) => user_id,
        Err(e) => return BlossomResponse::error(format!("Failed to get user: {}", e)),
    };

    // Store the report (the database will handle duplicate prevention via unique index)
    match db.add_report(&file_sha256, reporter_id, &data.as_json()).await {
        Ok(()) => BlossomResponse::Generic(BlossomGenericResponse {
            status: Status::Ok,
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
