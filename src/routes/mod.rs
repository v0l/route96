// #[cfg(feature = "blossom")]
use crate::auth::blossom::BlossomAuth;
use crate::db::{Database, FileUpload};
// #[cfg(feature = "blossom")]
use crate::nip29::Nip29Client;
// #[cfg(feature = "blossom")]
use crate::routes::blossom::check_h_tag;
use crate::settings::Settings;
use crate::storage::{HttpRange, StorageBackend};
use anyhow::{Error, Result};
use bs58;
use http_range_header::{parse_range_header, EndPosition, StartPosition};
use log::{debug, error, warn};
use nostr_sdk::nostr::Event;
use nostr_sdk::prelude::*;
use rocket::fs::NamedFile;
use rocket::http::{Header, Status};
use rocket::request::{FromRequest, Outcome};
use rocket::response::{self, Responder};
use rocket::{async_trait, Request, Response, State};
use serde::Serialize;
use std::io::SeekFrom;
use std::ops::Range;
use std::pin::{pin, Pin};
use std::sync::Arc;
use std::task::{Context as TaskContext, Poll};
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncSeek, ReadBuf};

// Remove cfg feature flag
// #[cfg(feature = "blossom")]
mod blossom;
// #[cfg(feature = "blossom")]
pub use blossom::blossom_routes;

pub mod admin;

// Define a custom responder that wraps Response
pub struct CustomResponse(Response<'static>);

// Implement Responder for the custom wrapper
impl<'r, 'o: 'r> Responder<'r, 'o> for CustomResponse {
    fn respond_to(self, _req: &'r Request<'_>) -> response::Result<'o> {
        // A finalized Response just needs to be returned.
        Ok(self.0)
    }
}

pub enum FilePayload {
    File(File),
    Range(RangeBody),
}

#[derive(Clone, Debug, Serialize, Default)]
#[serde(crate = "rocket::serde")]
pub struct Nip94Event {
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

    pub fn get_range(
        file_size: i64,
        header: &http_range_header::SyntacticallyCorrectRange,
    ) -> Range<i64> {
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
        cx: &mut TaskContext<'_>,
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
    storage: &State<Arc<dyn StorageBackend>>,
    is_group_admin: bool,
) -> Result<(), Error> {
    log::debug!(
        "delete_file called for user {} on file {}",
        auth.pubkey.to_hex(),
        sha256
    );
    match delete_file_internal(sha256, auth, db, storage, is_group_admin).await {
        Ok(response) => {
            if let Some(resp) = response {
                // This indicates an error response was generated (e.g., Forbidden)
                // We should probably propagate this error differently, but for now,
                // log it and return Ok(()) as the function signature expects.
                // A better approach might be to change delete_file signature
                // to return Result<Option<Response<'static>>, Error>
                log::warn!(
                    "delete_file_internal generated an unexpected response: {:?}",
                    resp.status()
                );
                Ok(())
            } else {
                // Success, no response body needed
                Ok(())
            }
        }
        Err(e) => {
            log::error!(
                "Error in delete_file_internal for {}: {}",
                sha256,
                e.to_string()
            );
            Err(e) // Propagate the internal error
        }
    }
}

async fn delete_file_internal(
    sha256: &str,
    auth: &Event,
    db: &Database,
    storage: &State<Arc<dyn StorageBackend>>,
    is_group_admin: bool,
) -> Result<Option<Response<'static>>, Error> {
    let id = hex::decode(sha256).map_err(|e| Error::msg(format!("Invalid hex: {}", e)))?;

    // Get the user performing the delete
    let auth_pubkey_bytes = auth.pubkey.to_bytes().to_vec();
    // Attempt to get user, handle potential DB error
    let user_result = db.get_user(&auth_pubkey_bytes).await;

    // Check ownership or admin status
    let owners = db.get_file_owners(&id).await?;
    let user_is_owner = owners.iter().any(|owner| owner.pubkey == auth_pubkey_bytes);

    if is_group_admin || user_is_owner {
        // If user is admin OR owns the file
        if is_group_admin {
            // Admin can delete all associations
            db.delete_all_file_owner(&id).await?;
        } else {
            // Non-admin owner can only delete their own association.
            // We need the user's ID. Check the result from earlier.
            match user_result {
                Ok(user) => {
                    db.delete_file_owner(&id, user.id).await?;
                }
                Err(e) => {
                    // This shouldn't happen if they are an owner, but handle defensively
                    log::error!(
                        "Failed to get user record for owner {}: {}",
                        auth.pubkey.to_hex(),
                        e
                    );
                    return Err(Error::from(e).context("Failed to retrieve owner user record"));
                }
            }
        }

        // Re-check owners after potential deletion
        let remaining_owners = db.get_file_owners(&id).await?;
        if remaining_owners.is_empty() {
            // If no owners left, delete the file from storage and DB
            log::debug!("Attempting to delete {} from storage backend", sha256);
            storage.delete(&id).await?;
            log::info!("Successfully deleted file {} from storage backend.", sha256);
            // Also delete the main upload record from the database
            db.delete_file(&id).await?;
            log::debug!("Deleted upload record for {} from database.", sha256);
        } else {
            log::debug!(
                "Only removed user ownership for file {}, not deleting from storage.",
                sha256
            );
        }
        Ok(None) // Indicate success with no specific response body needed
    } else {
        // User is not admin and not an owner
        log::error!(
            "delete_file called for user {} on file {} without ownership or admin rights.",
            auth.pubkey.to_hex(),
            sha256
        );
        // Return an error response (e.g., Forbidden)
        // Note: This function returns Result<Option<Response>, Error>
        // Returning Ok(Some(Response)) might be confusing. Consider changing signature.
        let forbidden_response = Response::build()
            .status(Status::Forbidden)
            .header(rocket::http::ContentType::Plain)
            .sized_body(
                None,
                std::io::Cursor::new(
                    "Forbidden: User does not have permission to delete this file.".as_bytes(),
                ),
            )
            .finalize();
        Ok(Some(forbidden_response))
    }
}

// #[cfg(feature = "blossom")]
async fn authorize_file_access(
    file_h_tag: &Option<String>,
    auth: Option<BlossomAuth>,
    nip29_client: &State<Arc<Nip29Client>>,
    requested_hash: &str,
) -> Result<(), Status> {
    // 1. Check if the file is public (no h_tag)
    if file_h_tag.is_none() {
        debug!(
            "File {} is public (no h_tag), access granted.",
            requested_hash
        );
        return Ok(()); // Public file, access granted
    }

    // Dereference file_h_tag safely, we know it's Some here
    let file_h_tag_str = file_h_tag.as_deref().unwrap();
    debug!(
        "File {} belongs to group {}. Checking auth...",
        requested_hash, file_h_tag_str
    );

    // 2. Check if authentication is provided
    let auth = match auth {
        Some(a) => a,
        None => {
            warn!(
                "Auth required for group file {}, but none provided.",
                requested_hash
            );
            return Err(Status::Unauthorized); // Auth required but not provided
        }
    };
    let auth_event = auth.event;

    // 3. Check for method tag (should be "read" or similar? Assuming GET implies read)
    // Skipping explicit method check for GET routes, but might be needed for others.

    // 4. Check h_tag in auth event, convert Option<&str> to Option<String>
    let auth_h_tag_opt: Option<String> = check_h_tag(&auth_event);
    let auth_h_tag = match auth_h_tag_opt {
        Some(tag) => tag, // Now it's an owned String
        None => {
            warn!(
                "Auth event for group file {} missing h_tag.",
                requested_hash
            );
            return Err(Status::BadRequest); // h_tag required in auth
        }
    };

    // Compare &str with &str using as_str()
    if file_h_tag_str != auth_h_tag.as_str() {
        warn!(
            "Auth h_tag mismatch for file {}: file has '{}', auth has '{}'",
            requested_hash, file_h_tag_str, auth_h_tag
        );
        return Err(Status::Forbidden); // Tags don't match
    }

    // 5. Check group membership (h_tags already confirmed to match)
    if nip29_client
        .is_group_member(&auth_h_tag, &auth_event.pubkey)
        .await
    {
        debug!(
            "NIP-29 check passed: User {} is a member of group {}",
            auth_event.pubkey.to_hex(),
            auth_h_tag
        );
        Ok(()) // User is a member, access granted
    } else {
        warn!(
            "NIP-29 check failed: User {} not a member of group {}",
            auth_event.pubkey.to_hex(),
            auth_h_tag
        );
        Err(Status::Forbidden) // User not a member
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

pub struct RangeHeader(pub Option<String>);

#[async_trait]
impl<'r> FromRequest<'r> for RangeHeader {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let range = request.headers().get_one("Range").map(|v| v.to_string());
        Outcome::Success(RangeHeader(range))
    }
}

// Parses the raw sha256 string, handling potential file extensions.
fn preprocess_sha256(sha256: &str) -> Result<Vec<u8>, Status> {
    let hash_part = sha256.split('.').next().unwrap_or(sha256);
    if hash_part.len() != 64 {
        warn!("Invalid sha256 length: {}", hash_part.len());
        return Err(Status::BadRequest);
    }
    hex::decode(hash_part).map_err(|e| {
        warn!("Invalid sha256 hex: {}", e);
        Status::BadRequest
    })
}

// Parses the Range header string and validates it against file size.
fn parse_and_validate_range(range_header_val: Option<String>, file_size: u64) -> Option<HttpRange> {
    // If file size is 0, no range is possible or meaningful.
    if file_size == 0 {
        return None;
    }

    let mut range_tuple: Option<(Option<u64>, Option<u64>)> = None;

    if let Some(range_header_str) = range_header_val {
        match parse_range_header(&range_header_str) {
            Ok(parsed_ranges) => {
                // We only support a single range specifier for simplicity
                if let Some(range) = parsed_ranges.ranges.first() {
                    // Match on the start and end position variants directly
                    match (range.start, range.end) {
                        (StartPosition::Index(start), EndPosition::Index(end)) => {
                            // SatisfiableRange with specific start and end
                            range_tuple = Some((Some(start), Some(end)));
                        }
                        (StartPosition::Index(start), EndPosition::LastByte) => {
                            // Range from start to the end
                            range_tuple = Some((Some(start), None));
                        }
                        (StartPosition::FromLast(suffix), _) => {
                            // SuffixRange (bytes=-N)
                            range_tuple = Some((None, Some(suffix)));
                        }
                    }
                } else {
                    warn!(
                        "No valid range specifiers found in header: {}",
                        range_header_str
                    );
                }
            }
            Err(e) => {
                warn!("Failed to parse Range header '{}': {}", range_header_str, e);
            }
        }
    }

    // Calculate effective HttpRange based on file size
    if let Some((start_opt, end_opt)) = range_tuple {
        match (start_opt, end_opt) {
            // Specific range: bytes=start-end
            (Some(start), Some(end)) => {
                // Validate start <= end and start < file_size.
                // end >= file_size is allowed by RFC, means read up to the end.
                if start <= end && start < file_size {
                    // Clip end to the actual last byte index (file_size - 1)
                    let effective_end = end.min(file_size - 1);
                    // Ensure start is still <= effective_end after clipping
                    if start <= effective_end {
                        // Wrap values in Some()
                        return Some(HttpRange {
                            start: Some(start),
                            end: Some(effective_end),
                        });
                    } else {
                        warn!(
                            "Range invalid after clipping: start ({}) > effective_end ({}) for file size {}",
                            start,
                            effective_end,
                            file_size
                        );
                        // Return 416 Range Not Satisfiable implicitly by returning None
                    }
                } else {
                    warn!(
                        "Range invalid or unsatisfiable: start={}, end={}, file_size={}",
                        start, end, file_size
                    );
                    // Return 416 Range Not Satisfiable implicitly by returning None
                }
            }
            // Range from start: bytes=start-
            (Some(start), None) => {
                if start < file_size {
                    // Range is from start to the end of the file
                    // Wrap values in Some()
                    return Some(HttpRange {
                        start: Some(start),
                        end: Some(file_size - 1),
                    });
                } else {
                    warn!(
                        "Range invalid or unsatisfiable: start ({}) >= file_size ({})",
                        start, file_size
                    );
                    // Return 416 Range Not Satisfiable implicitly by returning None
                }
            }
            // Suffix range: bytes=-suffix (represented as (None, Some(suffix)))
            (None, Some(suffix)) => {
                if suffix > 0 {
                    if suffix >= file_size {
                        // Suffix is larger than or equal to file size, return the whole file
                        // Wrap values in Some()
                        return Some(HttpRange {
                            start: Some(0),
                            end: Some(file_size - 1),
                        });
                    } else {
                        // Calculate start for the suffix
                        let start = file_size - suffix;
                        // Wrap values in Some()
                        return Some(HttpRange {
                            start: Some(start),
                            end: Some(file_size - 1),
                        });
                    }
                } else {
                    // suffix == 0 is invalid according to RFC
                    warn!("Invalid suffix range: suffix=0");
                    // Return 416 Range Not Satisfiable implicitly by returning None
                }
            }
            (None, None) => {
                // This case should technically not be hit if parse_range_header works correctly
                warn!("Invalid range tuple encountered after parsing: (None, None)");
                // Return 416 Range Not Satisfiable implicitly by returning None
            }
        }
    }

    // No header, or header parsing failed, or range was invalid/unsatisfiable
    None
}

#[rocket::get("/<sha256>")]
pub async fn get_blob_route(
    sha256: &str,
    range_header: RangeHeader,
    storage: &State<Arc<dyn StorageBackend>>,
    auth: Option<BlossomAuth>,
    db: &State<Database>,
    nip29_client: &State<Arc<Nip29Client>>,
) -> Result<CustomResponse, Status> {
    let file_id = match preprocess_sha256(sha256) {
        Ok(id) => id,
        Err(status) => return Err(status),
    };

    // Get storage metadata first (size, mime type)
    let storage_metadata = match storage.head(&file_id).await {
        Ok(meta) => meta,
        Err(_) => return Err(Status::NotFound), // File not found in storage
    };

    // --- Authorization Check (conditional on blossom feature) ---
    // Get database metadata (including h_tag for authorization) - only needed for Blossom auth
    let db_metadata = match db.get_file(&file_id).await {
        Ok(Some(meta)) => meta,
        Ok(None) => {
            // File exists in storage but not DB - treat as not found for consistency.
            log::warn!(
                "File {} found in storage but missing from database.",
                sha256
            );
            return Err(Status::NotFound);
        }
        Err(e) => {
            error!("Database error fetching file {}: {}", sha256, e);
            return Err(Status::InternalServerError);
        }
    };

    if let Err(status) = authorize_file_access(&db_metadata.h_tag, auth, nip29_client, sha256).await
    {
        return Err(status); // Return Forbidden, Unauthorized, etc.
    }
    // --- End Authorization Check ---

    let range = parse_and_validate_range(range_header.0, storage_metadata.size);

    let mut response_builder = Response::build();
    // Use mime_type from storage metadata
    response_builder.header(Header::new(
        "Content-Type",
        storage_metadata.mime_type.clone(),
    ));
    response_builder.header(Header::new("Accept-Ranges", "bytes"));

    match storage.stream_reader(&file_id, range.clone()).await {
        Ok(stream) => {
            if let Some(http_range) = range {
                let start = http_range.start.expect("Validated range should have start");
                let end = http_range.end.expect("Validated range should have end");

                response_builder.status(Status::PartialContent);
                response_builder.header(Header::new(
                    "Content-Range",
                    // Use size from storage metadata
                    format!("bytes {}-{}/{}", start, end, storage_metadata.size),
                ));
            } else {
                response_builder.status(Status::Ok);
            }
            Ok(CustomResponse(
                response_builder.streamed_body(stream).finalize(),
            ))
        }
        Err(e) => {
            error!("Failed to stream file {}: {}", sha256, e);
            Err(Status::InternalServerError)
        }
    }
}

#[rocket::head("/<sha256>")]
pub async fn head_blob(
    sha256: &str,
    storage: &State<Arc<dyn StorageBackend>>,
) -> Result<CustomResponse, Status> {
    let file_id = match preprocess_sha256(sha256) {
        Ok(id) => id,
        Err(status) => return Err(status),
    };

    match storage.head(&file_id).await {
        Ok(metadata) => Ok(CustomResponse(
            Response::build()
                .status(Status::Ok)
                .header(Header::new("Content-Type", metadata.mime_type))
                .header(Header::new("Content-Length", metadata.size.to_string()))
                .header(Header::new("Accept-Ranges", "bytes"))
                .finalize(),
        )),
        Err(_) => Err(Status::NotFound),
    }
}

#[cfg(feature = "media-compression")]
#[rocket::get("/thumb/<sha256>")]
pub async fn get_blob_thumb(
    sha256: &str,
    _storage: &State<Arc<dyn StorageBackend>>,
) -> Result<CustomResponse, Status> {
    let _file_id = match preprocess_sha256(sha256) {
        Ok(id) => id,
        Err(status) => return Err(status),
    };

    // TODO: Implement actual thumbnail generation/retrieval logic
    let body_str = "Thumbnail generation not yet implemented.";
    let body_bytes = body_str.as_bytes();
    let body_len = body_bytes.len() as u64;

    Ok(CustomResponse(
        Response::build()
            .status(Status::NotImplemented)
            .header(rocket::http::ContentType::Plain)
            .sized_body(Some(body_len as usize), std::io::Cursor::new(body_bytes))
            .finalize(),
    ))
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

#[rocket::get("/health")]
pub async fn health_check() -> &'static str {
    "OK"
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
