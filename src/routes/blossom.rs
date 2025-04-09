use crate::auth::blossom::BlossomAuth;
use crate::db::{Database, FileUpload};
use crate::filesystem::{FileStore, FileSystemResult};
use crate::nip29::Nip29Client;
use crate::routes::{delete_file, Nip94Event};
use crate::settings::Settings;
use log::error;
use nostr_sdk::prelude::hex;
use nostr_sdk::prelude::TagKind;
use rocket::data::ByteUnit;
use rocket::futures::StreamExt;
use rocket::http::{Header, Status};
use rocket::response::Responder;
use rocket::serde::json::Json;
use rocket::{routes, Data, Request, Response, Route, State};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::AsyncRead as TokioAsyncRead;
use tokio_util::io::StreamReader;

#[derive(Debug, Clone, Serialize)]
#[serde(crate = "rocket::serde")]
pub struct BlobDescriptor {
    pub url: String,
    pub sha256: String,
    pub size: i64,
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
        delete,
        admin_delete,
        upload,
        upload_media,
        head_media,
        list_files,
        upload_head,
        mirror
    ]
}

#[cfg(not(feature = "media-compression"))]
pub fn blossom_routes() -> Vec<Route> {
    routes![
        delete,
        admin_delete,
        upload,
        list_files,
        upload_head,
        mirror
    ]
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

// Check for h tag (group id)
pub fn check_h_tag(event: &nostr_sdk::nostr::Event) -> Option<String> {
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
async fn delete(
    sha256: &str,
    auth: BlossomAuth,
    db: &State<Database>,
    nip29: &State<Arc<Nip29Client>>,
) -> Result<BlossomResponse, (Status, String)> {
    match try_delete_blob(sha256, auth, db, nip29).await {
        Ok(response) => Ok(response),
        Err((status, e)) => {
            log::error!("Error in delete handler: {}", e);
            Err((status, format!("Internal server error: {}", e)))
        }
    }
}

// Helper function for checking if user can delete a file
async fn check_delete_permission(
    file_info: &FileUpload,
    auth: &BlossomAuth,
    db: &State<Database>,
    nip29: &State<Arc<Nip29Client>>,
) -> Result<bool, BlossomResponse> {
    // Check if user is the file owner
    match db.get_file_owners(&file_info.id).await {
        Ok(owners) => {
            let auth_pubkey_bytes = auth.event.pubkey.to_bytes().to_vec();
            if owners.iter().any(|owner| owner.pubkey == auth_pubkey_bytes) {
                log::debug!("User is a file owner, permission granted");
                return Ok(true);
            } else {
                log::debug!("User is not a file owner");
                // Continue to check group permissions if applicable
            }
        }
        Err(e) => {
            log::error!("Database error when checking file owners: {}", e);
            return Err(BlossomResponse::error(format!("Database error: {}", e)));
        }
    }

    // Check if the file belongs to a group (has h_tag)
    if let Some(file_h_tag) = &file_info.h_tag {
        log::debug!("File belongs to group: {}", file_h_tag);
        // File is a group file, check group admin status
        let auth_h_tag = match check_h_tag(&auth.event) {
            Some(tag) => tag,
            None => {
                log::error!("Auth event missing h_tag for group file deletion");
                return Err(BlossomResponse::error("Missing h tag for group file"));
            }
        };

        if file_h_tag != &auth_h_tag {
            log::error!("Auth h_tag mismatch ({} != {})", auth_h_tag, file_h_tag);
            return Err(BlossomResponse::error(
                "Auth h_tag doesn't match file h_tag",
            ));
        }

        // Check if user is a group admin
        if nip29.is_group_admin(file_h_tag, &auth.event.pubkey).await {
            log::debug!("User is a group admin, permission granted");
            return Ok(true);
        } else {
            log::debug!("User is not a group admin, checking if they're a member");
            // Not a group admin, check if they're a member (maybe for listing but not deleting?)
            if nip29.is_group_member(file_h_tag, &auth.event.pubkey).await {
                log::debug!("User is a group member but not an admin");
                // Currently, members cannot delete, only admins. Might change later.
                // Fall through to deny permission.
            } else {
                log::error!("User is not a member of the group");
                return Err(BlossomResponse::forbidden("Not a member of the group"));
            }
        }
    } else {
        log::debug!("File has no h_tag, skipping group permission checks");
    }

    // If none of the checks passed
    log::debug!("All permission checks failed, returning false");
    Ok(false)
}

// Separate function to handle the actual deletion logic
async fn try_delete_blob(
    sha256: &str,
    auth: BlossomAuth,
    db: &State<Database>,
    nip29: &State<Arc<Nip29Client>>,
) -> Result<BlossomResponse, (Status, String)> {
    log::debug!("Attempting to delete blob with sha256: {}", sha256);
    log::debug!("Auth event pubkey: {}", auth.event.pubkey.to_hex());

    // Check for method tag
    if !check_method(&auth.event, "delete") {
        log::error!("Invalid method tag in auth event");
        return Ok(BlossomResponse::error("Invalid request method tag"));
    }

    // Extract the hex ID and get the file info
    let id = match hex::decode(sha256.split('.').next().unwrap_or(sha256)) {
        Ok(i) => {
            if i.len() != 32 {
                log::error!("Invalid file id length: {} (expected 32)", i.len());
                return Ok(BlossomResponse::error("Invalid file id"));
            }
            log::debug!("Successfully decoded file id");
            i
        }
        Err(e) => {
            log::error!("Failed to decode file id: {}", e);
            return Ok(BlossomResponse::error("Invalid file id format"));
        }
    };

    // Get the file info
    let file_info = match db.get_file(&id).await {
        Ok(Some(info)) => {
            log::debug!("Found file in database with id: {}", hex::encode(&id));
            log::debug!("File has h_tag: {:?}", info.h_tag);
            info
        }
        Ok(None) => {
            log::error!("File not found in database: {}", sha256);
            return Ok(BlossomResponse::not_found("File not found"));
        }
        Err(e) => {
            log::error!("Database error when looking up file: {}", e);
            return Ok(BlossomResponse::error(format!("Database error: {}", e)));
        }
    };

    // Check if the user has permission to delete the file
    log::debug!("Checking user permission to delete the file");
    match check_delete_permission(&file_info, &auth, db, nip29).await {
        Ok(true) => {
            log::debug!("User has permission to delete the file");
            // User has permission, proceed with deletion
            let is_admin = match db.get_user(&auth.event.pubkey.to_bytes().to_vec()).await {
                Ok(user) => {
                    log::debug!("User is_admin flag: {}", user.is_admin);
                    user.is_admin
                }
                Err(e) => {
                    log::error!("Failed to get user for admin check: {}", e);
                    false
                }
            };

            log::debug!("Calling delete_file with is_admin={}", is_admin);
            match delete_file(sha256, &auth.event, db, is_admin).await {
                Ok(()) => {
                    log::debug!("File successfully deleted");
                    Ok(BlossomResponse::Generic(BlossomGenericResponse {
                        status: Status::Ok,
                        message: None,
                    }))
                }
                Err(e) => {
                    log::error!("Failed to delete file: {}", e);
                    Ok(BlossomResponse::error(format!(
                        "Failed to delete file: {}",
                        e
                    )))
                }
            }
        }
        Ok(false) => {
            log::error!("User does not have permission to delete the file");
            Ok(BlossomResponse::forbidden("Not authorized to delete files"))
        }
        Err(response) => {
            log::error!("Error checking delete permission: {:?}", response);
            Ok(response)
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
    if let Some(e) = check_whitelist(&auth, settings) {
        return e;
    }
    if !check_method(&auth.event, method) {
        return BlossomResponse::error("Invalid request method tag");
    }

    // Check for h tag
    let h_tag = check_h_tag(&auth.event);
    if let Some(ref tag_val) = h_tag {
        // Ensure user is a member of the group if h_tag is present
        if !nip29_client
            .is_group_member(tag_val, &auth.event.pubkey)
            .await
        {
            return BlossomResponse::forbidden("Not a member of the group");
        }
    }

    process_stream(
        data.open(ByteUnit::Byte(settings.max_upload_bytes)),
        &auth
            .content_type
            .unwrap_or("application/octet-stream".to_string()),
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
    pubkey: &Vec<u8>,
    compress: bool,
    fs: &State<FileStore>,
    db: &State<Database>,
    settings: &State<Settings>,
    h_tag: Option<String>,
) -> BlossomResponse
where
    S: TokioAsyncRead + Unpin + 'p,
{
    let upload = match fs.put(stream, mime_type, compress).await {
        Ok(FileSystemResult::NewFile(blob)) => {
            let mut ret: FileUpload = (&blob).into();

            // update file data before inserting
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

#[rocket::delete("/admin/<sha256>")]
async fn admin_delete(
    sha256: &str,
    auth: BlossomAuth,
    db: &State<Database>,
    nip29: &State<Arc<Nip29Client>>,
) -> Result<BlossomResponse, (Status, String)> {
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
    let is_db_admin = match db.get_user(&pubkey_vec).await {
        Ok(user) => user.is_admin,
        Err(_) => false, // If not found or error, not admin
    };
    if !is_db_admin {
        return Ok(BlossomResponse::forbidden("Admin privileges required"));
    }

    // Check if this is a group file and verify group authorization (even for DB admins)
    match db.get_file(&id).await {
        Ok(Some(file_info)) => {
            if let Some(file_h_tag) = &file_info.h_tag {
                let h_tag = check_h_tag(&auth.event);
                if h_tag.is_none() {
                    return Ok(BlossomResponse::error("Missing h tag for group file"));
                }
                let auth_h_tag = h_tag.as_deref().unwrap();
                if auth_h_tag != file_h_tag {
                    return Ok(BlossomResponse::error(
                        "Auth h_tag doesn't match file h_tag",
                    ));
                }
                // Verify group membership for DB admins too, just to be safe?
                // Or assume DB admin overrides group membership?
                // For now, let's enforce membership for consistency.
                if !nip29.is_group_member(file_h_tag, &auth.event.pubkey).await {
                    return Ok(BlossomResponse::forbidden(
                        "Admin not a member of the group (required for group file deletion)",
                    ));
                }
            }
            // If no h_tag, DB admin can delete.
        }
        Ok(None) => {
            return Ok(BlossomResponse::not_found("File not found"));
        }
        Err(e) => {
            return Ok(BlossomResponse::error(format!("Database error: {}", e)));
        }
    };

    // Delete the file using the admin privileges
    match delete_file(sha256, &auth.event, db, true).await {
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
