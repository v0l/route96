use crate::auth::blossom::BlossomAuth;
use crate::db::{Database, FileUpload};
use crate::filesystem::{FileStore, FileSystemResult};
use crate::nip29::Nip29Client;
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
use std::sync::Arc;
use tokio::io::AsyncRead;
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
        .iter()
        .find(|t| t.kind() == TagKind::t())
        .and_then(|t| t.content())
        .map_or(false, |content| content == method)
}

fn check_h_tag(event: &nostr_sdk::nostr::Event) -> Option<String> {
    // Check for h tag (required for all operations)
    event
        .tags
        .iter()
        .find(|t| t.kind() == TagKind::h())
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
    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
    log::debug!(
        "Checking delete permission for file: {}",
        hex::encode(&file_info.id)
    );
    log::debug!("User pubkey: {}", auth.event.pubkey.to_hex());
    log::debug!("File h_tag: {:?}", file_info.h_tag);

    // Check if user is a server admin
    match db.get_user(&pubkey_vec).await {
        Ok(user) if user.is_admin => {
            log::debug!("User is a server admin, permission granted");
            log::debug!("is_admin flag raw value: {:?}", user.is_admin);
            return Ok(true);
        }
        Ok(user) => {
            log::debug!(
                "User found but not an admin (is_admin = {:?})",
                user.is_admin
            );
            log::debug!(
                "is_admin type: {}",
                std::any::type_name_of_val(&user.is_admin)
            );
        }
        Err(e) => {
            log::error!("Error looking up user: {}", e);
            return Err(BlossomResponse::forbidden("Not authorized to delete files"));
        }
    }

    // Check if user is the file creator
    match db.get_file_owners(&file_info.id).await {
        Ok(owners) => {
            log::debug!("Found {} owner(s) for the file", owners.len());
            for owner in &owners {
                log::debug!(
                    "File owner: {} (id: {})",
                    hex::encode(&owner.pubkey),
                    owner.id
                );
                log::debug!(
                    "Owner pubkey length: {}, User pubkey length: {}",
                    owner.pubkey.len(),
                    pubkey_vec.len()
                );
                if owner.pubkey.len() == pubkey_vec.len() {
                    let matching = owner
                        .pubkey
                        .iter()
                        .zip(pubkey_vec.iter())
                        .filter(|&(a, b)| a == b)
                        .count();
                    log::debug!(
                        "Matching bytes: {}/{} when comparing owner pubkey to user pubkey",
                        matching,
                        owner.pubkey.len()
                    );
                }
            }

            if owners.iter().any(|owner| owner.pubkey == pubkey_vec) {
                log::debug!("User is a file owner, permission granted");
                return Ok(true);
            } else {
                log::debug!("User is not a file owner");
                log::debug!("User pubkey: {}", hex::encode(&pubkey_vec));
            }
        }
        Err(e) => {
            log::error!("Database error when checking file owners: {}", e);
            return Err(BlossomResponse::error(format!("Database error: {}", e)));
        }
    }

    // Check if user is a group admin (for files with h_tag)
    if let Some(file_h_tag) = &file_info.h_tag {
        log::debug!("File has h_tag: {}, checking group permissions", file_h_tag);

        // Check for h tag in the auth event
        let h_tag = check_h_tag(&auth.event);
        if h_tag.is_none() {
            log::error!("Missing h tag in authentication event");
            return Err(BlossomResponse::error("Missing h tag for group file"));
        }

        let auth_h_tag = h_tag.as_deref().unwrap();
        log::debug!("Auth event h_tag: {}", auth_h_tag);

        // Verify h_tag in auth matches file's h_tag
        if auth_h_tag != file_h_tag {
            log::error!(
                "Auth h_tag '{}' doesn't match file h_tag '{}'",
                auth_h_tag,
                file_h_tag
            );
            return Err(BlossomResponse::error(
                "Auth h_tag doesn't match file h_tag",
            ));
        }

        // Check if user is a group admin
        match nip29.is_group_admin(file_h_tag, &auth.event.pubkey).await {
            Ok(true) => {
                log::debug!("User is a group admin, permission granted");
                return Ok(true);
            }
            Ok(false) => {
                log::debug!("User is not a group admin, checking if they're a member");
                // Not a group admin, check if they're a member
                match nip29.is_group_member(file_h_tag, &auth.event.pubkey).await {
                    Ok(true) => {
                        log::debug!("User is a group member but not an admin");
                    }
                    Ok(false) => {
                        log::error!("User is not a member of the group");
                        return Err(BlossomResponse::forbidden("Not a member of the group"));
                    }
                    Err(e) => {
                        log::error!("Error checking group membership: {}", e);
                        return Err(BlossomResponse::error(format!(
                            "Error checking group membership: {}",
                            e
                        )));
                    }
                }
            }
            Err(e) => {
                log::error!("Error checking group admin status: {}", e);
                return Err(BlossomResponse::error(format!(
                    "Error checking admin status: {e}"
                )));
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
    if !check_method(&auth.event, method) {
        error!(
            "Invalid request method tag: expected '{}', found '{:?}'",
            method,
            auth.event
                .tags
                .iter()
                .find(|t| t.kind() == TagKind::t())
                .map(|t| t.content())
                .flatten()
        );
        return BlossomResponse::error("Invalid request method tag");
    }

    // Debug log the tags to help troubleshoot
    error!("Auth event tags: {:?}", auth.event.tags);

    // Check for h tag (required for security)
    let h_tag = check_h_tag(&auth.event);
    if h_tag.is_none() {
        error!("Missing h tag in request");
        return BlossomResponse::error("Missing h tag");
    }

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

    // Check group membership (h_tag is guaranteed to exist here)
    let h_tag_str = h_tag.as_deref().unwrap();
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
    S: AsyncRead + Unpin + 'p,
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

    // Try to find the user - if not found, they don't have permission
    let is_admin = match db.get_user(&pubkey_vec).await {
        Ok(user) => user.is_admin,
        Err(_) => {
            // If the user isn't in the database, they don't have permission to delete anything
            return Ok(BlossomResponse::forbidden("Not authorized to delete files"));
        }
    };

    // For admin route, require the user to be a database admin
    if !is_admin {
        return Ok(BlossomResponse::forbidden("Admin privileges required"));
    }

    // Check if this is a group file and verify group authorization
    match db.get_file(&id).await {
        Ok(Some(file_info)) => {
            // If the file has an h_tag, we still need to verify group access
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

                // Verify group membership - admins should still be members
                match nip29.is_group_member(file_h_tag, &auth.event.pubkey).await {
                    Ok(true) => {
                        // Member of the group, proceed with admin privileges
                    }
                    Ok(false) => {
                        return Ok(BlossomResponse::forbidden("Not a member of the group"));
                    }
                    Err(e) => {
                        return Ok(BlossomResponse::error(format!(
                            "Error checking group membership: {}",
                            e
                        )));
                    }
                }
            }
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
