use crate::auth::blossom::BlossomAuth;
use crate::db::{Database, FileUpload};
use crate::nip29::Nip29Client;
use crate::routes::{delete_file, Nip94Event};
use crate::settings::Settings;
use crate::storage::{StorageBackend, StorageResult};
use anyhow::Result;
use log::error;
use nostr_sdk::prelude::hex;
use nostr_sdk::prelude::TagKind;
use rocket::data::{ByteUnit, ToByteUnit};
use rocket::futures::StreamExt;
use rocket::http::{Header, Status};
use rocket::response::Responder;
use rocket::serde::json::Json;
use rocket::{routes, Data, Request, Response, Route, State};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs::{self, File};
use tokio::io::AsyncRead as TokioAsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

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

pub fn blossom_routes() -> Vec<Route> {
    println!("DEBUG: Defining blossom routes...");
    // Start with base blossom routes
    let mut routes_vec = routes![
        delete,
        admin_delete,
        upload,
        list_files,
        upload_head,
        mirror
    ];

    // Conditionally add media routes
    #[cfg(feature = "media-compression")]
    {
        println!("Adding media compression routes to blossom...");
        routes_vec.extend(routes![upload_media, head_media,]);
    }

    routes_vec // Return the final Vec
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
    storage: &State<Arc<dyn StorageBackend>>,
) -> Result<BlossomResponse, (Status, String)> {
    match try_delete_blob(sha256, auth, db, nip29, storage).await {
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
    storage: &State<Arc<dyn StorageBackend>>,
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
            match delete_file(sha256, &auth.event, db, storage, is_admin).await {
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
    storage: &State<Arc<dyn StorageBackend>>,
    db: &State<Database>,
    settings: &State<Settings>,
    data: Data<'_>,
    nip29_client: &State<std::sync::Arc<crate::nip29::Nip29Client>>,
) -> BlossomResponse {
    process_upload("upload", auth, storage, db, settings, data, nip29_client).await
}

#[rocket::put("/mirror", data = "<req>", format = "json")]
async fn mirror(
    auth: BlossomAuth,
    storage: &State<Arc<dyn StorageBackend>>,
    db: &State<Database>,
    settings: &State<Settings>,
    req: Json<MirrorRequest>,
    nip29_client: &State<std::sync::Arc<crate::nip29::Nip29Client>>,
) -> BlossomResponse {
    if !check_method(&auth.event, "upload") {
        return BlossomResponse::forbidden(
            "Invalid request method tag (must be 'upload' for mirror)",
        );
    }

    if let Some(e) = check_whitelist(&auth, settings) {
        return e;
    }

    let h_tag = check_h_tag(&auth.event);
    if let Some(ref tag_val) = h_tag {
        if !nip29_client
            .is_group_member(tag_val, &auth.event.pubkey)
            .await
        {
            return BlossomResponse::forbidden("Not a member of the group specified in 'h' tag");
        }
        log::debug!("Group membership check passed for h_tag: {}", tag_val);
    }

    log::debug!("Mirroring file from URL: {}", req.url);
    let rsp = match reqwest::get(&req.url).await {
        Ok(r) => r,
        Err(e) => {
            log::error!("Error downloading file for mirroring: {}", e);
            return BlossomResponse::error(format!("Failed to download mirror file: {}", e));
        }
    };

    if !rsp.status().is_success() {
        log::error!(
            "Mirror download failed with status: {} for URL: {}",
            rsp.status(),
            req.url
        );
        return BlossomResponse::error(format!(
            "Failed to download mirror file (status {})",
            rsp.status()
        ));
    }

    let mime_type = rsp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or("application/octet-stream".to_string());

    log::debug!("Downloaded file MIME type: {}", mime_type);

    let temp_dir_path = env::temp_dir();
    let (temp_file_guard, downloaded_hash, downloaded_size) = {
        let uid = Uuid::new_v4();
        let temp_path = temp_dir_path.join(uid.to_string());

        if let Err(e) = fs::create_dir_all(&temp_dir_path).await {
            return BlossomResponse::error(format!("Failed to create temp directory: {}", e));
        }

        let mut file = match File::create(&temp_path).await {
            Ok(f) => f,
            Err(e) => {
                return BlossomResponse::error(format!(
                    "Failed to create temp file {:?}: {}",
                    temp_path, e
                ));
            }
        };

        let mut stream = rsp.bytes_stream();
        let mut hasher = Sha256::new();
        let mut total_bytes = 0u64;

        while let Some(chunk_result) = stream.next().await {
            match chunk_result {
                Ok(chunk) => {
                    if let Err(e) = file.write_all(&chunk).await {
                        let _ = fs::remove_file(&temp_path).await;
                        return BlossomResponse::error(format!(
                            "Failed to write to temp file {:?}: {}",
                            temp_path, e
                        ));
                    }
                    hasher.update(&chunk);
                    total_bytes += chunk.len() as u64;
                }
                Err(e) => {
                    let _ = fs::remove_file(&temp_path).await;
                    return BlossomResponse::error(format!("Error reading download stream: {}", e));
                }
            }
        }

        if let Err(e) = file.flush().await {
            let _ = fs::remove_file(&temp_path).await;
            return BlossomResponse::error(format!(
                "Failed to flush temp file {:?}: {}",
                temp_path, e
            ));
        }
        drop(file);

        let hash_vec = hasher.finalize().to_vec();
        (TempFileCleanup(temp_path), hash_vec, total_bytes)
    };
    let temp_file_path = temp_file_guard.0.clone();

    let x_tags: Vec<&str> = auth
        .event
        .tags
        .iter()
        .filter(|t| t.kind() == TagKind::x())
        .filter_map(|t| t.content())
        .collect();

    if x_tags.is_empty() {
        return BlossomResponse::forbidden("Missing hash ('x') tag in authorization for mirror");
    }

    let hex_downloaded_hash = hex::encode(&downloaded_hash);
    if !x_tags
        .iter()
        .any(|tag_hash| *tag_hash == hex_downloaded_hash)
    {
        log::warn!(
            "Mirror auth failed: Provided hash(es) {:?} do not match downloaded hash {}",
            x_tags,
            hex_downloaded_hash
        );
        return BlossomResponse::forbidden("Hash ('x') tag mismatch for mirrored file");
    }
    log::debug!(
        "Mirror auth check passed: Hash {} matched one of {:?}.",
        hex_downloaded_hash,
        x_tags
    );

    process_original_upload(
        storage,
        db,
        settings,
        temp_file_path,
        downloaded_hash,
        downloaded_size,
        &mime_type,
        &auth.event.pubkey.to_bytes().to_vec(),
        h_tag,
        None,
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
    storage: &State<Arc<dyn StorageBackend>>,
    db: &State<Database>,
    settings: &State<Settings>,
    data: Data<'_>,
    nip29_client: &State<std::sync::Arc<crate::nip29::Nip29Client>>,
) -> BlossomResponse {
    process_upload("media", auth, storage, db, settings, data, nip29_client).await
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

    if check_h_tag(&auth.event).is_none() {
        return BlossomHead {
            msg: Some("Missing h tag"),
        };
    }

    if let Some(wl) = &settings.whitelist {
        if !wl.contains(&auth.event.pubkey.to_hex()) {
            return BlossomHead {
                msg: Some("Not on whitelist"),
            };
        }
    }

    BlossomHead { msg: None }
}

struct TempFileCleanup(PathBuf);

impl Drop for TempFileCleanup {
    fn drop(&mut self) {
        let path = &self.0;
        if path.exists() {
            log::debug!("Cleaning up temporary upload file: {:?}", path);
            let path_clone = path.clone();
            tokio::spawn(async move {
                if let Err(e) = fs::remove_file(&path_clone).await {
                    log::error!("Failed to clean up temporary file {:?}: {}", path_clone, e);
                } else {
                    log::debug!("Successfully cleaned up temporary file: {:?}", path_clone);
                }
            });
        }
    }
}

async fn stream_to_temp_file_and_hash<'d>(
    data_stream: Data<'d>,
    limit: ByteUnit,
) -> Result<(TempFileCleanup, Vec<u8>, u64), BlossomResponse> {
    let uid = Uuid::new_v4();
    let temp_dir = env::temp_dir();
    let temp_path = temp_dir.join(uid.to_string());

    if let Err(e) = fs::create_dir_all(&temp_dir).await {
        return Err(BlossomResponse::error(format!(
            "Failed to create temp directory: {}",
            e
        )));
    }

    let mut file = File::create(&temp_path).await.map_err(|e| {
        BlossomResponse::error(format!("Failed to create temp file {:?}: {}", temp_path, e))
    })?;

    let mut stream = data_stream.open(limit);
    let mut hasher = Sha256::new();
    let mut total_bytes = 0u64;
    let mut buf = [0; 8192];

    loop {
        match stream.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                let bytes = &buf[..n];
                file.write_all(bytes).await.map_err(|e| {
                    let _cleanup = TempFileCleanup(temp_path.clone());
                    BlossomResponse::error(format!(
                        "Failed to write to temp file {:?}: {}",
                        temp_path, e
                    ))
                })?;
                hasher.update(bytes);
                total_bytes += n as u64;
            }
            Err(e) => {
                let _cleanup = TempFileCleanup(temp_path.clone());
                return Err(BlossomResponse::error(format!(
                    "Error reading chunk from stream: {}",
                    e
                )));
            }
        }
    }

    let hash = hasher.finalize().to_vec();
    file.flush().await.map_err(|e| {
        let _cleanup = TempFileCleanup(temp_path.clone());
        BlossomResponse::error(format!("Failed to flush temp file {:?}: {}", temp_path, e))
    })?;
    drop(file);

    Ok((TempFileCleanup(temp_path), hash, total_bytes))
}

async fn hash_file(file_path: &Path) -> anyhow::Result<Vec<u8>> {
    let mut file = File::open(file_path).await?;
    let mut hasher = Sha256::new();
    let mut buf = [0; 8192];

    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(hasher.finalize().to_vec())
}

async fn process_upload(
    method: &str,
    auth: BlossomAuth,
    storage: &State<Arc<dyn StorageBackend>>,
    db: &State<Database>,
    settings: &State<Settings>,
    data: Data<'_>,
    nip29_client: &State<std::sync::Arc<crate::nip29::Nip29Client>>,
) -> BlossomResponse {
    if let Some(e) = check_whitelist(&auth, settings) {
        return e;
    }

    let (temp_file_guard, original_hash, original_size) =
        match stream_to_temp_file_and_hash(data, settings.max_upload_bytes.bytes()).await {
            Ok(result) => result,
            Err(resp) => return resp,
        };

    let original_temp_path = temp_file_guard.0.clone();

    if !check_method(&auth.event, method) {
        return BlossomResponse::forbidden("Invalid request method tag");
    }

    let x_tags: Vec<&str> = auth
        .event
        .tags
        .iter()
        .filter(|t| t.kind() == TagKind::x())
        .filter_map(|t| t.content())
        .collect();

    if x_tags.is_empty() {
        return BlossomResponse::forbidden("Missing hash ('x') tag in authorization");
    }

    let hex_original_hash = hex::encode(&original_hash);
    if !x_tags.iter().any(|tag_hash| *tag_hash == hex_original_hash) {
        log::warn!(
            "Auth check failed: Provided hash(es) {:?} do not match calculated hash {}",
            x_tags,
            hex_original_hash
        );
        return BlossomResponse::forbidden("Hash ('x') tag mismatch");
    }
    log::debug!(
        "Auth check passed: Hash {} matched one of {:?}.",
        hex_original_hash,
        x_tags
    );

    let h_tag = check_h_tag(&auth.event);
    if let Some(ref tag_val) = h_tag {
        if !nip29_client
            .is_group_member(tag_val, &auth.event.pubkey)
            .await
        {
            return BlossomResponse::forbidden("Not a member of the group specified in 'h' tag");
        }
        log::debug!("Group membership check passed for h_tag: {}", tag_val);
    }

    let mime_type = auth
        .content_type
        .unwrap_or("application/octet-stream".to_string());

    // Always process the original upload directly
    log::debug!("Processing original upload...");
    process_original_upload(
        storage,
        db,
        settings,
        original_temp_path,
        original_hash,
        original_size,
        &mime_type,
        &auth.event.pubkey.to_bytes().to_vec(),
        h_tag,
        None,
    )
    .await
}

async fn process_original_upload(
    storage: &State<Arc<dyn StorageBackend>>,
    db: &State<Database>,
    settings: &State<Settings>,
    temp_file_path: PathBuf,
    original_hash: Vec<u8>,
    original_size: u64,
    mime_type: &str,
    pubkey: &Vec<u8>,
    h_tag: Option<String>,
    _optimized_metadata: Option<FileUpload>,
) -> BlossomResponse {
    let file_stream = match File::open(&temp_file_path).await {
        Ok(file) => Box::new(file) as Box<dyn TokioAsyncRead + Send + Unpin>,
        Err(e) => {
            log::error!(
                "Failed to re-open temp file {:?} for storage backend: {}",
                temp_file_path,
                e
            );
            return BlossomResponse::error(format!("Internal error reading temp file: {}", e));
        }
    };

    let storage_result = match storage.put(file_stream, mime_type).await {
        Ok(res) => res,
        Err(e) => {
            error!("Storage backend put error: {}", e.to_string());
            return BlossomResponse::error(format!("Error saving file (storage): {}", e));
        }
    };

    let upload_metadata = match storage_result {
        StorageResult::NewFile {
            id,
            size,
            mime_type,
        } => {
            if id != original_hash {
                error!(
                    "CRITICAL: Hash mismatch! Calculated={}, StoragePutReturned={}",
                    hex::encode(&original_hash),
                    hex::encode(&id)
                );
                return BlossomResponse::error(
                    "Internal Server Error: Hash mismatch during storage.",
                );
            }
            if size != original_size {
                log::warn!(
                    "Size mismatch: Calculated={}, StoragePutReturned={}. Using storage value.",
                    original_size,
                    size
                );
            }

            FileUpload {
                id,
                size: size as i64,
                mime_type,
                created: chrono::Utc::now(),
                width: None,
                height: None,
                blur_hash: None,
                alt: None,
                duration: None,
                bitrate: None,
                h_tag,
                #[cfg(feature = "labels")]
                labels: Vec::new(),
            }
        }
        StorageResult::AlreadyExists(id) => {
            if id != original_hash {
                error!("CRITICAL: Hash mismatch on AlreadyExists! Calculated={}, StorageReportedExisting={}", hex::encode(&original_hash), hex::encode(&id));
                return BlossomResponse::error(
                    "Internal Server Error: Hash mismatch for existing file.",
                );
            }
            log::debug!(
                "File already exists in storage backend: {}",
                hex::encode(&id)
            );
            match db.get_file(&id).await {
                Ok(Some(f)) => f,
                Ok(None) => {
                    log::warn!(
                        "File {} exists in storage but not DB. Creating DB record.",
                        hex::encode(&id)
                    );
                    FileUpload {
                        id,
                        size: original_size as i64,
                        mime_type: mime_type.to_string(),
                        created: chrono::Utc::now(),
                        width: None,
                        height: None,
                        blur_hash: None,
                        alt: None,
                        duration: None,
                        bitrate: None,
                        h_tag,
                        #[cfg(feature = "labels")]
                        labels: Vec::new(),
                    }
                }
                Err(e) => {
                    return BlossomResponse::error(format!(
                        "DB error fetching existing file: {}",
                        e
                    ));
                }
            }
        }
    };

    let user_id = match db.upsert_user(pubkey).await {
        Ok(u) => u,
        Err(e) => {
            return BlossomResponse::error(format!("Failed to save user (db): {}", e));
        }
    };

    if let Err(e) = db.add_file(&upload_metadata, Some(user_id)).await {
        error!("Failed to add file to DB: {}", e.to_string());
        BlossomResponse::error(format!("Error saving file metadata (db): {}", e))
    } else {
        log::info!(
            "Successfully processed upload for hash: {}",
            hex::encode(&upload_metadata.id)
        );
        BlossomResponse::BlobDescriptor(Json(BlobDescriptor::from_upload(
            settings,
            &upload_metadata,
        )))
    }
}

#[rocket::delete("/admin/<sha256>")]
async fn admin_delete(
    sha256: &str,
    auth: BlossomAuth,
    db: &State<Database>,
    nip29: &State<Arc<Nip29Client>>,
    storage: &State<Arc<dyn StorageBackend>>,
) -> Result<BlossomResponse, (Status, String)> {
    if !check_method(&auth.event, "delete") {
        return Ok(BlossomResponse::error("Invalid request method tag"));
    }

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

    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
    let is_db_admin = match db.get_user(&pubkey_vec).await {
        Ok(user) => user.is_admin,
        Err(_) => false,
    };
    if !is_db_admin {
        return Ok(BlossomResponse::forbidden("Admin privileges required"));
    }

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
                if !nip29.is_group_member(file_h_tag, &auth.event.pubkey).await {
                    return Ok(BlossomResponse::forbidden(
                        "Admin not a member of the group (required for group file deletion)",
                    ));
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

    match delete_file(sha256, &auth.event, db, storage, true).await {
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
