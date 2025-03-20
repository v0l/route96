use crate::auth::blossom::BlossomAuth;
use crate::db::{Database, FileUpload};
use crate::filesystem::{FileStore, FileSystemResult};
use crate::routes::{delete_file, Nip94Event};
use crate::settings::Settings;
use log::error;
use nostr_sdk::nostr::prelude::hex;
use nostr_sdk::nostr::TagKind;
use rocket::data::ByteUnit;
use rocket::futures::StreamExt;
use rocket::http::{Header, Status};
use rocket::response::Responder;
use rocket::serde::json::Json;
use rocket::{routes, Data, Request, Response, Route, State};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    pub uploaded: u64,
    #[serde(rename = "nip94", skip_serializing_if = "Option::is_none")]
    pub nip94: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub h_tag: Option<String>,
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
            nip94: Some(
                Nip94Event::from_upload(settings, value)
                    .tags
                    .iter()
                    .map(|r| (r[0].clone(), r[1].clone()))
                    .collect(),
            ),
            h_tag: value.h_tag.clone(),
        }
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
        mirror
    ]
}

#[cfg(not(feature = "media-compression"))]
pub fn blossom_routes() -> Vec<Route> {
    routes![delete_blob, upload, list_files, upload_head, mirror]
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

    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::Generic(BlossomGenericResponse {
            message: Some(msg.into()),
            status: Status::Forbidden,
        })
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::Generic(BlossomGenericResponse {
            message: Some(msg.into()),
            status: Status::NotFound,
        })
    }
}

impl std::fmt::Debug for BlossomResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlossomResponse::Generic(response) => {
                write!(
                    f,
                    "BlossomResponse Generic {:?} {:?}",
                    response.status, response.message
                )
            }
            _ => write!(f, "BlossomResponse"),
        }
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

fn check_method(event: &nostr_sdk::nostr::Event, method: &str) -> bool {
    // Check for t tag with the correct method
    event
        .tags
        .find(TagKind::t())
        .and_then(|t| t.content())
        .map_or(false, |content| content == method)
}

fn check_h_tag(event: &nostr_sdk::nostr::Event) -> Option<String> {
    // Check for h tag (required for all operations)
    event
        .tags
        .find(TagKind::h())
        .and_then(|t| t.content())
        .map(|s| s.to_string())
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
    nip29_client: &State<std::sync::Arc<crate::nip29::Nip29Client>>,
) -> BlossomResponse {
    match try_delete_blob(sha256, auth, fs, db, nip29_client).await {
        Ok(response) => response,
        Err(e) => {
            log::error!("Error in delete_blob handler: {}", e);
            BlossomResponse::error(format!("Internal server error: {}", e))
        }
    }
}

// Separate function to handle the actual deletion logic
async fn try_delete_blob(
    sha256: &str,
    auth: BlossomAuth,
    fs: &State<FileStore>,
    db: &State<Database>,
    nip29_client: &State<std::sync::Arc<crate::nip29::Nip29Client>>,
) -> Result<BlossomResponse, anyhow::Error> {
    // Check for method tag
    if !check_method(&auth.event, "delete") {
        return Ok(BlossomResponse::error("Invalid request method tag"));
    }

    // Extract the hex ID and get the file info
    let id = match hex::decode(sha256.split('.').next().unwrap_or(sha256)) {
        Ok(i) => {
            if i.len() != 32 {
                return Ok(BlossomResponse::error("Invalid file id"));
            }
            i
        }
        Err(_) => {
            return Ok(BlossomResponse::error("Invalid file id format"));
        }
    };

    // First check if the user is a database admin
    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();

    // Try to find the user - if not found, they don't have permission
    let is_admin = match db.get_user(&pubkey_vec).await {
        Ok(user) => user.is_admin,
        Err(_) => {
            // If the user isn't in the database, they don't have permission to delete anything
            return Ok(BlossomResponse::forbidden("Not authorized to delete files"));
        }
    };

    // If user is a database admin, they can always delete files
    if is_admin {
        match delete_file(sha256, &auth.event, fs, db, false).await {
            Ok(()) => {
                return Ok(BlossomResponse::Generic(BlossomGenericResponse {
                    status: Status::Ok,
                    message: None,
                }));
            }
            Err(e) => {
                return Ok(BlossomResponse::error(format!(
                    "Failed to delete file: {}",
                    e
                )));
            }
        }
    }

    // Process the file access permissions and check group admin status
    let is_group_admin = {
        match db.get_file(&id).await {
            Ok(Some(file_info)) => {
                // If the file has an h_tag, we need to verify group access
                if let Some(file_h_tag) = &file_info.h_tag {
                    // Check for h tag in the auth event
                    let h_tag = check_h_tag(&auth.event);
                    if h_tag.is_none() {
                        return Ok(BlossomResponse::error("Missing h tag for group file"));
                    }

                    let auth_h_tag = h_tag.as_deref().unwrap();

                    // Verify h_tag in auth matches file's h_tag
                    if auth_h_tag != file_h_tag {
                        return Ok(BlossomResponse::error(
                            "Auth h_tag doesn't match file h_tag",
                        ));
                    }

                    // First check if the user is a group admin
                    match nip29_client
                        .is_group_admin(file_h_tag, &auth.event.pubkey)
                        .await
                    {
                        Ok(true) => {
                            true // User is a group admin
                        }
                        Ok(false) => {
                            // Not an admin, check if they're a member
                            match nip29_client
                                .is_group_member(file_h_tag, &auth.event.pubkey)
                                .await
                            {
                                Ok(true) => {
                                    false // Not a group admin, ownership check needed
                                }
                                Ok(false) => {
                                    return Ok(BlossomResponse::forbidden(
                                        "Not a member of the group",
                                    ));
                                }
                                Err(e) => {
                                    return Ok(BlossomResponse::error(format!(
                                        "Error checking group membership: {}",
                                        e
                                    )));
                                }
                            }
                        }
                        Err(e) => {
                            return Ok(BlossomResponse::error(format!(
                                "Error checking admin status: {e}"
                            )));
                        }
                    }
                } else {
                    false // Not a group admin
                }
            }
            Ok(None) => {
                return Ok(BlossomResponse::not_found("File not found"));
            }
            Err(e) => {
                return Ok(BlossomResponse::error(format!("Database error: {}", e)));
            }
        }
    };

    // Now proceed with deletion
    match delete_file(sha256, &auth.event, fs, db, is_group_admin).await {
        Ok(()) => Ok(BlossomResponse::Generic(BlossomGenericResponse {
            status: Status::Ok,
            message: None,
        })),
        Err(e) => Ok(BlossomResponse::error(format!(
            "Failed to delete file: {}",
            e
        ))),
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
    nip29_client: &State<std::sync::Arc<crate::nip29::Nip29Client>>,
) -> BlossomResponse {
    process_upload("upload", false, auth, fs, db, settings, data, nip29_client).await
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

    // Check for h tag
    let h_tag = check_h_tag(&auth.event);
    if h_tag.is_none() {
        return BlossomResponse::error("Missing h tag");
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
        h_tag,
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
    nip29_client: &State<std::sync::Arc<crate::nip29::Nip29Client>>,
) -> BlossomResponse {
    process_upload("media", true, auth, fs, db, settings, data, nip29_client).await
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

    // Check for h tag
    if check_h_tag(&auth.event).is_none() {
        return BlossomHead {
            msg: Some("Missing h tag"),
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
    nip29_client: &State<std::sync::Arc<crate::nip29::Nip29Client>>,
) -> BlossomResponse {
    if !check_method(&auth.event, method) {
        return BlossomResponse::error("Invalid request method tag");
    }

    // Check for h tag
    let h_tag = check_h_tag(&auth.event);
    if h_tag.is_none() {
        return BlossomResponse::error("Missing h tag");
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

    if let Some(h_tag_str) = h_tag.as_deref() {
        match nip29_client
            .is_group_member(h_tag_str, &auth.event.pubkey)
            .await
        {
            Ok(true) => {}
            Ok(false) => return BlossomResponse::forbidden("Not a member of the group"),
            Err(e) => {
                error!("Error checking group membership: {}", e);
                return BlossomResponse::error("Error checking group membership");
            }
        };
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
        h_tag,
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
    h_tag: Option<String>,
) -> BlossomResponse
where
    S: AsyncRead + Unpin + 'p,
{
    let upload = match fs.put(stream, mime_type, compress).await {
        Ok(FileSystemResult::NewFile(blob)) => {
            let mut ret: FileUpload = (&blob).into();

            // update file data before inserting
            ret.name = name.map(|s| s.to_string());
            ret.h_tag = h_tag;

            ret
        }
        Ok(FileSystemResult::AlreadyExists(i)) => match db.get_file(&i).await {
            Ok(Some(f)) => f,
            _ => return BlossomResponse::not_found("File not found"),
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
    if let Err(e) = db.add_file(&upload, Some(user_id)).await {
        error!("{}", e.to_string());
        BlossomResponse::error(format!("Error saving file (db): {}", e))
    } else {
        BlossomResponse::BlobDescriptor(Json(BlobDescriptor::from_upload(settings, &upload)))
    }
}
