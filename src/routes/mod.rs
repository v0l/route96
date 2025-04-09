use crate::auth::blossom::BlossomAuth;
use crate::db::{Database, FileUpload};
use crate::nip29::Nip29Client;
#[cfg(feature = "media-compression")]
use crate::processing::WebpProcessor;
pub use crate::routes::admin::admin_routes;
#[cfg(feature = "blossom")]
pub use crate::routes::blossom::blossom_routes;
#[cfg(feature = "nip96")]
pub use crate::routes::nip96::nip96_routes;
use crate::settings::Settings;
use anyhow::{Error, Result};
use bs58;
use http_range_header::{
    parse_range_header, EndPosition, StartPosition, SyntacticallyCorrectRange,
};
use log::{debug, warn};
use nostr_sdk::prelude::*;
use rocket::fs::NamedFile;
use rocket::http::{Header, Status};
use rocket::request::{FromRequest, Outcome};
use rocket::response::{self, Responder};
use rocket::serde::Serialize;
use rocket::{async_trait, Request, Response, State};
use std::io::Error as IoError;
use std::io::SeekFrom;
use std::ops::Range;
use std::pin::{pin, Pin};
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncSeek, ReadBuf};

#[cfg(feature = "blossom")]
mod blossom;
#[cfg(feature = "nip96")]
mod nip96;

mod admin;

pub enum FilePayload {
    File(File),
    Range(RangeBody),
}

#[derive(Clone, Debug, Serialize, Default)]
#[serde(crate = "rocket::serde")]
struct Nip94Event {
    pub created_at: i64,
    pub content: Option<String>,
    pub tags: Vec<Vec<String>>,
}

#[derive(Serialize, Default)]
#[serde(crate = "rocket::serde")]
struct PagedResult<T> {
    pub count: u32,
    pub page: u32,
    pub total: u32,
    pub files: Vec<T>,
}

impl Nip94Event {
    pub fn from_upload(settings: &Settings, upload: &FileUpload) -> Self {
        let hex_id = hex::encode(&upload.id);
        let ext = if upload.mime_type != "application/octet-stream" {
            mime2ext::mime2ext(&upload.mime_type)
        } else {
            None
        };

        // Create tags using the new API
        let mut tags = Vec::new();

        // URL tag
        tags.push(vec![
            "url".to_string(),
            format!("{}/{}.{}", &settings.public_url, &hex_id, ext.unwrap_or("")),
        ]);

        // X tag (hex ID)
        tags.push(vec!["x".to_string(), hex_id.clone()]);

        // M tag (mime type)
        tags.push(vec!["m".to_string(), upload.mime_type.clone()]);

        // Size tag
        tags.push(vec!["size".to_string(), upload.size.to_string()]);

        // Thumb tag for images and videos
        if upload.mime_type.starts_with("image/") || upload.mime_type.starts_with("video/") {
            tags.push(vec![
                "thumb".to_string(),
                format!("{}/thumb/{}.webp", &settings.public_url, &hex_id),
            ]);
        }

        // Blurhash tag
        if let Some(bh) = &upload.blur_hash {
            tags.push(vec!["blurhash".to_string(), bh.clone()]);
        }

        // Dimensions tag
        if let (Some(w), Some(h)) = (upload.width, upload.height) {
            tags.push(vec!["dim".to_string(), format!("{}x{}", w, h)])
        }

        // Duration tag
        if let Some(d) = &upload.duration {
            tags.push(vec!["duration".to_string(), d.to_string()]);
        }

        // Bitrate tag
        if let Some(b) = &upload.bitrate {
            tags.push(vec!["bitrate".to_string(), b.to_string()]);
        }

        // Labels tag (if feature enabled)
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
            content: upload.alt.clone(),
            created_at: upload.created.timestamp(),
            tags,
        }
    }
}

/// Range request handler over file handle
pub struct RangeBody {
    file: File,
    range_start: i64,
    range_end: i64,
    current_offset: i64,
    poll_complete: bool,
    #[allow(dead_code)]
    file_size: i64,
}

const MAX_UNBOUNDED_RANGE: i64 = 1024 * 1024;

impl RangeBody {
    pub fn new(file: File, file_size: i64, range: Range<i64>) -> Self {
        Self {
            file,
            file_size,
            range_start: range.start,
            range_end: range.end,
            current_offset: 0,
            poll_complete: false,
        }
    }

    pub fn get_range(file_size: i64, header: &SyntacticallyCorrectRange) -> Range<i64> {
        let range_start = match header.start {
            StartPosition::Index(i) => i as i64,
            StartPosition::FromLast(i) => file_size.saturating_sub(i as i64),
        };
        let range_end = match header.end {
            EndPosition::Index(i) => i as i64,
            EndPosition::LastByte => (file_size - 1).min(range_start + MAX_UNBOUNDED_RANGE),
        };
        range_start..range_end
    }

    #[allow(dead_code)]
    pub fn get_headers(&self) -> Vec<Header<'static>> {
        let r_len = (self.range_end - self.range_start) + 1;
        vec![
            Header::new("content-length", r_len.to_string()),
            Header::new(
                "content-range",
                format!(
                    "bytes {}-{}/{}",
                    self.range_start, self.range_end, self.file_size
                ),
            ),
        ]
    }
}

impl AsyncRead for RangeBody {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let range_start = self.range_start + self.current_offset;
        let range_len = self.range_end.saturating_sub(range_start) + 1;
        let bytes_to_read = buf.remaining().min(range_len as usize) as i64;

        if bytes_to_read == 0 {
            return Poll::Ready(Ok(()));
        }

        // when no pending poll, seek to starting position
        if !self.poll_complete {
            let pinned = pin!(&mut self.file);
            pinned.start_seek(SeekFrom::Start(range_start as u64))?;
            self.poll_complete = true;
        }

        // check poll completion
        if self.poll_complete {
            let pinned = pin!(&mut self.file);
            match pinned.poll_complete(cx) {
                Poll::Ready(Ok(_)) => {
                    self.poll_complete = false;
                }
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }

        // Read data from the file
        let pinned = pin!(&mut self.file);
        match pinned.poll_read(cx, buf) {
            Poll::Ready(Ok(_)) => {
                self.current_offset += bytes_to_read;
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => {
                self.poll_complete = true;
                Poll::Pending
            }
        }
    }
}

impl<'r> Responder<'r, 'static> for FilePayload {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        match self {
            FilePayload::File(file) => Response::build().streamed_body(file).ok(),
            FilePayload::Range(range) => Response::build().streamed_body(range).ok(),
        }
    }
}

pub async fn delete_file(
    sha256: &str,
    auth: &Event,
    db: &Database,
    is_group_admin: bool,
) -> Result<(), Error> {
    log::debug!("Starting delete_file for sha256: {}", sha256);
    log::debug!("User pubkey: {}", auth.pubkey.to_hex());
    log::debug!("Is group admin flag: {}", is_group_admin);

    let sha256 = if sha256.contains(".") {
        sha256.split('.').next().unwrap()
    } else {
        sha256
    };

    let id = match hex::decode(sha256) {
        Ok(i) => i,
        Err(_) => {
            log::error!("Invalid file id format: {}", sha256);
            return Err(Error::msg("Invalid file id"));
        }
    };

    if id.len() != 32 {
        log::error!("Invalid file id length: {}", id.len());
        return Err(Error::msg("Invalid file id"));
    }

    match db.get_file(&id).await {
        Ok(Some(file)) => {
            log::debug!("File found in database with h_tag: {:?}", file.h_tag);
            let pubkey_vec = auth.pubkey.to_bytes().to_vec();

            let auth_user = match db.get_user(&pubkey_vec).await {
                Ok(user) => {
                    log::debug!("User found in database. Is admin: {}", user.is_admin);
                    user
                }
                Err(e) => {
                    log::error!("Failed to get user: {}", e);
                    return Err(Error::msg(format!("Failed to get user: {}", e)));
                }
            };

            let owners = match db.get_file_owners(&id).await {
                Ok(o) => {
                    log::debug!("Found {} owner(s) for the file", o.len());
                    for owner in &o {
                        log::debug!(
                            "File owner: {} (id: {})",
                            hex::encode(&owner.pubkey),
                            owner.id
                        );
                    }
                    o
                }
                Err(e) => {
                    log::error!("Failed to get file owners: {}", e);
                    return Err(Error::msg(format!("Failed to get file owners: {}", e)));
                }
            };

            // Admin (either database admin or group admin)
            if auth_user.is_admin || is_group_admin {
                log::debug!("User is admin or group admin, proceeding with full deletion");
                if let Err(e) = db.delete_all_file_owner(&id).await {
                    log::error!("Failed to delete file owners: {}", e);
                    return Err(Error::msg(format!("Failed to delete file owners: {}", e)));
                }

                if let Err(e) = db.delete_file(&id).await {
                    log::error!("Failed to delete file record: {}", e);
                    return Err(Error::msg(format!("Failed to delete file record: {}", e)));
                }

                // Use Database.get_file_path for consistent file path handling
                match db.get_file_path(&id).await {
                    Ok(file_path) => {
                        if let Err(e) = tokio::fs::remove_file(&file_path).await {
                            log::warn!("Failed to delete file from disk: {}", e);
                        } else {
                            log::debug!("File successfully deleted from disk: {:?}", file_path);
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to get file path for deletion: {}", e);
                    }
                }
            } else {
                // Regular user must own the file
                log::debug!("User is not admin, checking if they own the file");
                let this_owner = match owners.iter().find(|o| o.pubkey.eq(&pubkey_vec)) {
                    Some(o) => {
                        log::debug!("User owns the file (owner id: {})", o.id);
                        o
                    }
                    None => {
                        log::error!("User does not own this file, cannot delete it");
                        return Err(Error::msg("You dont own this file, you cannot delete it"));
                    }
                };

                if let Err(e) = db.delete_file_owner(&id, this_owner.id).await {
                    log::error!("Failed to delete file owner: {}", e);
                    return Err(Error::msg(format!("Failed to delete file owner: {}", e)));
                }

                // only 1 owner was left, delete file completely
                if owners.len() == 1 {
                    log::debug!("Only one owner was left, deleting file completely");
                    if let Err(e) = db.delete_file(&id).await {
                        log::error!("Failed to delete file record: {}", e);
                        return Err(Error::msg(format!("Failed to delete file record: {}", e)));
                    }

                    // Use Database.get_file_path for consistent file path handling
                    match db.get_file_path(&id).await {
                        Ok(file_path) => {
                            if let Err(e) = tokio::fs::remove_file(&file_path).await {
                                log::warn!("Failed to delete file from disk: {}", e);
                            } else {
                                log::debug!("File successfully deleted from disk: {:?}", file_path);
                            }
                        }
                        Err(e) => {
                            log::warn!("Failed to get file path for deletion: {}", e);
                        }
                    }
                } else {
                    log::debug!(
                        "Multiple owners exist ({} owners), only removing this user's ownership",
                        owners.len()
                    );
                }
            }
            log::debug!("File deletion process completed successfully");
            Ok(())
        }
        Ok(None) => {
            log::error!("File not found in database: {}", sha256);
            Err(Error::msg("File not found"))
        }
        Err(e) => {
            log::error!("Error retrieving file from database: {}", e);
            Err(Error::msg(format!("Error retrieving file: {}", e)))
        }
    }
}

#[cfg(feature = "blossom")]
async fn authorize_file_access(
    file_h_tag: &Option<String>,
    auth: Option<BlossomAuth>,
    nip29_client: &State<Arc<Nip29Client>>,
    requested_hash: &str,
) -> Result<(), Status> {
    // --- Auth Header Presence ---
    let auth = match auth {
        Some(a) => a,
        None => {
            warn!(
                "Auth required but not provided for file fetch: {}",
                requested_hash
            );
            return Err(Status::Unauthorized);
        }
    };
    let auth_event = &auth.event;

    // --- Auth Event Tag Validation ---

    // 1. Check t=get tag using string comparison
    let t_tag_valid = auth_event.tags.iter().any(|tag| {
        let slice = tag.as_slice();
        slice.first().map(|s| s == "t").unwrap_or(false)
            && slice.get(1).map(|v| v == "get").unwrap_or(false)
    });
    if !t_tag_valid {
        warn!(
            "Auth event missing or incorrect 't=get' tag for hash {}",
            requested_hash
        );
        return Err(Status::Unauthorized); // Or Forbidden?
    }

    // 2. Check x=<hash> tag using string comparison
    let x_tag_matches = auth_event.tags.iter().any(|tag| {
        let slice = tag.as_slice();
        slice.first().map(|s| s == "x").unwrap_or(false)
            && slice.get(1).map(|v| v == requested_hash).unwrap_or(false)
    });
    if !x_tag_matches {
        warn!(
            "Auth event missing or incorrect 'x' tag for hash {}",
            requested_hash
        );
        return Err(Status::Unauthorized); // Or Forbidden?
    }

    // 3. Check for h=<group_id> tag in Auth Event using string comparison
    let auth_h_tag_value = auth_event.tags.iter().find_map(|tag| {
        let slice = tag.as_slice();
        if slice.first().map(|s| s == "h").unwrap_or(false) {
            slice.get(1).map(|s| s.as_str())
        } else {
            None
        }
    });

    let auth_h_tag = match auth_h_tag_value {
        Some(h) => h,
        None => {
            warn!(
                "Auth event missing required 'h' tag for hash {}",
                requested_hash
            );
            return Err(Status::Unauthorized); // Or Forbidden?
        }
    };

    // --- File and Group Matching ---

    // 4. Check if file has an h_tag and if it matches the auth h_tag
    match file_h_tag {
        Some(file_h) if file_h == auth_h_tag => {
            // Tags match, proceed to membership check
            debug!(
                "Auth h_tag ({}) matches file h_tag for file {}",
                auth_h_tag, requested_hash
            );
        }
        Some(file_h) => {
            // File has an h_tag, but it doesn't match the one in the auth event
            warn!(
                "NIP-29 check failed: Auth h_tag ({}) does not match file h_tag ({}) for file {}",
                auth_h_tag, file_h, requested_hash
            );
            return Err(Status::Forbidden);
        }
        None => {
            // File does NOT have an h_tag, but auth is required
            warn!(
                "NIP-29 check failed: File {} does not have an associated h_tag, access denied.",
                requested_hash
            );
            return Err(Status::Forbidden); // File not associated with the required group context
        }
    }

    // --- NIP-29 Membership Check ---

    // 5. Check group membership (h_tags already confirmed to match)
    match nip29_client
        .is_group_member(auth_h_tag, &auth_event.pubkey)
        .await
    {
        Ok(true) => {
            debug!(
                "NIP-29 check passed: User {} is a member of group {}",
                auth_event.pubkey.to_hex(),
                auth_h_tag
            );
            Ok(()) // User is a member, access granted
        }
        Ok(false) => {
            warn!(
                "NIP-29 check failed: User {} not a member of group {}",
                auth_event.pubkey.to_hex(),
                auth_h_tag
            );
            Err(Status::Forbidden) // User not a member
        }
        Err(e) => {
            warn!("NIP-29 check error for group {}: {}", auth_h_tag, e);
            Err(Status::InternalServerError) // Error checking membership
        }
    }
}

#[rocket::get("/")]
pub async fn root() -> Result<NamedFile, Status> {
    #[cfg(all(debug_assertions, feature = "react-ui"))]
    let index = "./ui_src/dist/index.html";
    #[cfg(all(not(debug_assertions), feature = "react-ui"))]
    let index = "./ui/index.html";
    #[cfg(not(feature = "react-ui"))]
    let index = "./index.html";
    if let Ok(f) = NamedFile::open(index).await {
        Ok(f)
    } else {
        Err(Status::InternalServerError)
    }
}

pub async fn get_blob(
    db: &Database,
    file: &[u8],
    range: Option<&SyntacticallyCorrectRange>,
) -> Result<FilePayload, IoError> {
    log::debug!("get_blob called with file hash: {}", hex::encode(file));

    let file_path = match db.get_file_path(file).await {
        Ok(path) => {
            log::debug!("File path found: {:?}", path);
            path
        }
        Err(e) => {
            log::error!("Failed to get file path: {}", e);
            return Err(e);
        }
    };

    let file_size = match file_path.metadata() {
        Ok(metadata) => {
            let size = metadata.len() as i64;
            log::debug!("File size: {}", size);
            size
        }
        Err(e) => {
            log::error!("Failed to get file metadata: {}", e);
            return Err(e);
        }
    };

    if let Some(range) = range {
        let range = RangeBody::get_range(file_size, range);
        if range.start > range.end || range.end >= file_size {
            return Err(IoError::new(
                std::io::ErrorKind::InvalidInput,
                "Range not satisfiable",
            ));
        }
        let file = File::open(&file_path).await?;
        Ok(FilePayload::Range(RangeBody::new(file, file_size, range)))
    } else {
        log::debug!("Opening file: {:?}", file_path);
        match File::open(&file_path).await {
            Ok(file) => {
                log::debug!("File opened successfully");
                Ok(FilePayload::File(file))
            }
            Err(e) => {
                log::error!("Failed to open file: {}", e);
                Err(e)
            }
        }
    }
}

// Request guard for Range header
pub struct RangeHeader(pub Option<String>);

#[async_trait]
impl<'r> FromRequest<'r> for RangeHeader {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let range = request.headers().get_one("Range").map(|v| v.to_string());
        Outcome::Success(RangeHeader(range))
    }
}

// Helper function to preprocess the SHA256 hex string
fn preprocess_sha256(sha256: &str) -> Result<Vec<u8>, Status> {
    let sha256_cleaned = if sha256.contains('.') {
        sha256.split('.').next().unwrap_or(sha256)
    } else {
        sha256
    };

    let id = match hex::decode(sha256_cleaned) {
        Ok(i) => i,
        Err(_) => {
            log::error!("Invalid file id format: {}", sha256);
            // Use BadRequest for invalid format/length
            return Err(Status::BadRequest);
        }
    };

    if id.len() != 32 {
        log::error!("Invalid file id length: {}", id.len());
        return Err(Status::BadRequest);
    }
    Ok(id)
}

// Helper function to parse and validate the Range header
fn parse_and_validate_range(
    range_header_val: Option<String>,
    file_size: i64,
) -> Option<SyntacticallyCorrectRange> {
    if let Some(range_str) = range_header_val {
        match parse_range_header(&range_str) {
            Ok(parsed_ranges) => {
                // Validate the parsed ranges against the file size
                match parsed_ranges.validate(file_size as u64) {
                    // validate expects u64
                    Ok(validated_byte_ranges) if !validated_byte_ranges.is_empty() => {
                        if let Some(first_syntactic_range) = parsed_ranges.ranges.first() {
                            debug!(
                                "Valid Range header found and validated: {:?}",
                                first_syntactic_range
                            );
                            Some(*first_syntactic_range)
                        } else {
                            // Should be unreachable, but handle defensively
                            warn!("Range validation succeeded but no ranges found in original parse? Header: {}", range_str);
                            None
                        }
                    }
                    Ok(_) => {
                        // Range(s) were syntactically correct but unsatisfiable for this file size.
                        warn!(
                            "Range header syntactically correct but unsatisfiable: {}",
                            range_str
                        );
                        // Note: A 416 Range Not Satisfiable might be more appropriate here,
                        // but returning None lets the caller decide or fall back to full file.
                        None
                    }
                    Err(e) => {
                        // Validation failed (e.g., invalid range format after parsing)
                        warn!("Failed to validate Range header '{}': {}", range_str, e);
                        None // Treat as invalid range
                    }
                }
            }
            Err(e) => {
                // Parsing failed
                warn!("Failed to parse Range header '{}': {}", range_str, e);
                None // Treat as invalid range
            }
        }
    } else {
        None // No range header provided
    }
}

#[rocket::get("/<sha256>")]
#[cfg(feature = "blossom")]
pub async fn get_blob_route(
    sha256: &str,
    db: &State<Database>,
    auth: Option<BlossomAuth>,
    nip29_client: &State<Arc<Nip29Client>>,
    range_header: RangeHeader,
) -> Result<FilePayload, Status> {
    let id = preprocess_sha256(sha256)?; // Use helper

    // --- Get File Metadata (needed for auth check and range validation) ---
    let file_meta = match db.get_file(&id).await {
        Ok(Some(meta)) => meta,
        Ok(None) => {
            warn!("File not found in DB: {}", sha256); // Log original input
            return Err(Status::NotFound);
        }
        Err(e) => {
            warn!("Error getting file metadata for {}: {}", sha256, e);
            return Err(Status::InternalServerError);
        }
    };

    let file_size = file_meta.size;
    let file_h_tag = file_meta.h_tag.clone(); // Clone needed for authorize_file_access

    // --- Authentication/Authorization ---
    // Pass the cleaned sha256 hex for accurate 'x' tag check
    let sha256_cleaned = hex::encode(&id);
    authorize_file_access(&file_h_tag, auth, nip29_client, &sha256_cleaned).await?;

    // --- Range Header Processing ---
    let valid_range_to_use = parse_and_validate_range(range_header.0, file_size); // Use helper

    // --- Blob Retrieval ---
    match get_blob(db, &id, valid_range_to_use.as_ref()).await {
        Ok(f) => Ok(f),
        Err(e) => {
            log::error!("Failed to get blob for {}: {}", sha256, e);
            // Consider mapping IoError kinds to different Status codes if needed
            Err(Status::NotFound) // Default to NotFound, could be InternalServerError too
        }
    }
}

#[rocket::get("/<sha256>")]
#[cfg(not(feature = "blossom"))]
pub async fn get_blob_route(
    sha256: &str,
    db: &State<Database>,
    range_header: RangeHeader,
) -> Result<FilePayload, Status> {
    let id = preprocess_sha256(sha256)?; // Use helper

    // --- Get File Metadata (needed for range validation) ---
    // Fetch metadata *before* range processing, similar to the blossom version
    let file_meta = match db.get_file(&id).await {
        Ok(Some(meta)) => meta,
        Ok(None) => {
            warn!("File not found in DB: {}", sha256);
            return Err(Status::NotFound);
        }
        Err(e) => {
            warn!("Error getting file metadata for {}: {}", sha256, e);
            return Err(Status::InternalServerError);
        }
    };
    let file_size = file_meta.size;

    // --- Range Header Processing ---
    let valid_range_to_use = parse_and_validate_range(range_header.0, file_size); // Use helper

    // --- Blob Retrieval ---
    match get_blob(db, &id, valid_range_to_use.as_ref()).await {
        Ok(f) => Ok(f),
        Err(e) => {
            log::error!("Failed to get blob for {}: {}", sha256, e);
            Err(Status::NotFound) // Default to NotFound
        }
    }
}

#[rocket::head("/<sha256>")]
pub async fn head_blob(sha256: &str, db: &State<Database>) -> Status {
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

    // Use the database's get_file_path method for consistency with the flat storage
    match db.get_file_path(&id).await {
        Ok(_) => Status::Ok,
        Err(_) => Status::NotFound,
    }
}

/// Generate thumbnail for image / video
#[cfg(feature = "media-compression")]
#[rocket::get("/thumb/<sha256>")]
pub async fn get_blob_thumb(
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
    let info = if let Ok(Some(info)) = db.get_file(&id).await {
        info
    } else {
        return Err(Status::NotFound);
    };

    if !(info.mime_type.starts_with("image/") || info.mime_type.starts_with("video/")) {
        return Err(Status::NotFound);
    }

    let file_path = fs.get(&id);

    let mut thumb_file = std::env::temp_dir().join(format!("thumb_{}", sha256));
    thumb_file.set_extension("webp");

    if !thumb_file.exists() {
        let mut p = WebpProcessor::new();
        if p.thumbnail(&file_path, &thumb_file).is_err() {
            return Err(Status::InternalServerError);
        }
    };

    if let Ok(f) = File::open(&thumb_file).await {
        Ok(FilePayload {
            file: f,
            info: FileUpload {
                size: thumb_file.metadata().unwrap().len(),
                mime_type: "image/webp".to_string(),
                ..info
            },
        })
    } else {
        Err(Status::NotFound)
    }
}

/// Legacy URL redirect for void.cat uploads
#[rocket::get("/d/<id>")]
pub async fn void_cat_redirect(id: &str, settings: &State<Settings>) -> Option<NamedFile> {
    let id = if id.contains(".") {
        id.split('.').next().unwrap()
    } else {
        id
    };
    if let Some(base) = &settings.void_cat_files {
        let uuid = if let Ok(b58) = bs58::decode(id).into_vec() {
            uuid::Uuid::from_slice_le(&b58).unwrap()
        } else {
            uuid::Uuid::parse_str(id).unwrap()
        };
        let f = base.join(uuid.to_string());
        debug!("Legacy file map: {} => {}", id, f.display());
        if let Ok(f) = NamedFile::open(f).await {
            Some(f)
        } else {
            None
        }
    } else {
        None
    }
}

#[rocket::head("/d/<id>")]
pub async fn void_cat_redirect_head(id: &str) -> VoidCatFile {
    let id = if id.contains(".") {
        id.split('.').next().unwrap()
    } else {
        id
    };

    // Handle both Result types properly
    let uuid = match bs58::decode(id).into_vec() {
        Ok(bytes) => match uuid::Uuid::from_slice_le(&bytes) {
            Ok(uuid) => uuid,
            Err(_) => {
                // If UUID conversion fails, try parsing as string
                match uuid::Uuid::parse_str(id) {
                    Ok(uuid) => uuid,
                    Err(_) => {
                        // If both methods fail, return a default UUID
                        uuid::Uuid::nil()
                    }
                }
            }
        },
        Err(_) => {
            // If base58 decoding fails, try parsing as string
            match uuid::Uuid::parse_str(id) {
                Ok(uuid) => uuid,
                Err(_) => {
                    // If both methods fail, return a default UUID
                    uuid::Uuid::nil()
                }
            }
        }
    };

    VoidCatFile {
        status: Status::Ok,
        uuid: Header::new("X-UUID", uuid.to_string()),
    }
}

#[derive(Responder)]
pub struct VoidCatFile {
    pub status: Status,
    pub uuid: Header<'static>,
}

#[cfg(test)]
mod tests {
    use super::MAX_UNBOUNDED_RANGE;
    use crate::routes::RangeBody;
    use http_range_header::parse_range_header;

    #[test]
    fn test_ranges() -> std::result::Result<(), anyhow::Error> {
        let size = 16482469;

        let req = parse_range_header("bytes=0-1023")?;
        let r = RangeBody::get_range(size, req.ranges.first().unwrap());
        assert_eq!(r.start, 0);
        assert_eq!(r.end, 1023);

        let req = parse_range_header("bytes=16482467-")?;
        let r = RangeBody::get_range(size, req.ranges.first().unwrap());
        assert_eq!(r.start, 16482467);
        assert_eq!(r.end, 16482468);

        let req = parse_range_header("bytes=-10")?;
        let r = RangeBody::get_range(size, req.ranges.first().unwrap());
        assert_eq!(r.start, 16482459);
        assert_eq!(r.end, 16482468);

        let req = parse_range_header("bytes=-16482470")?;
        let r = RangeBody::get_range(size, req.ranges.first().unwrap());
        assert_eq!(r.start, 0);
        assert_eq!(r.end, MAX_UNBOUNDED_RANGE);
        Ok(())
    }
}
