use crate::auth::blossom::BlossomAuth;
use crate::db::Database;
use crate::db::FileUpload;
use crate::filesystem::{FileSystemResult, HashMismatch};
use crate::routes::{AppState, Nip94Event, ban_check, delete_file};
use crate::settings::Settings;
use crate::whitelist::Whitelist;
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, RawQuery, State as AxumState},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, head, put},
};
use futures_util::TryStreamExt;
use futures_util::stream::StreamExt;
use log::{error, info};
use nostr::{Alphabet, JsonUtil, SingleLetterTag, TagKind};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::io::AsyncRead;
use tokio_util::io::StreamReader;
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobDescriptor {
    pub url: String,
    pub sha256: String,
    pub size: u64,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub uploaded: u64,
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
            uploaded: value.created.timestamp() as u64,
            nip94: Some(Nip94Event::from_upload(settings, value).tags),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MirrorRequest {
    pub url: String,
}

/// Extract SHA-256 hash from a URL path (last segment before optional extension)
fn url_hash_from_url(url: &str) -> Option<String> {
    use url::Url;
    
    let parsed = Url::parse(url).ok()?;
    let hash = parsed
        .path_segments()?.next_back()?
        .split('.')
        .next()?;
    
    if hash.len() == 64 {
        Some(hash.to_lowercase())
    } else {
        None
    }
}

pub fn blossom_routes() -> Router<Arc<AppState>> {
    let router = Router::new()
        .route(
            "/{sha256}",
            delete(delete_blob)
                .put(upload_by_hash)
                .options(hash_options),
        )
        .route("/list/{pubkey}", get(list_files))
        .route("/upload", head(upload_head).put(upload))
        .route("/mirror", put(mirror))
        .route("/report", put(report_file));

    #[cfg(feature = "media-compression")]
    let router = router.route("/media", head(head_media).put(upload_media));

    router
}

pub(crate) const HASH_RESOURCE_METHODS: &str = "GET, HEAD, PUT, DELETE, OPTIONS";

pub(crate) fn is_bud13_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn has_url_query(raw_query: Option<&str>) -> bool {
    raw_query.is_some_and(|query| {
        url::form_urlencoded::parse(query.as_bytes()).any(|(key, _)| key == "url")
    })
}

async fn hash_options(Path(sha256): Path<String>) -> Response {
    if !is_bud13_sha256(&sha256) {
        return BlossomResponse::bad_request("Invalid sha256 path").into_response();
    }

    (
        StatusCode::NO_CONTENT,
        [
            (header::ALLOW, HASH_RESOURCE_METHODS),
            (header::ACCESS_CONTROL_ALLOW_METHODS, HASH_RESOURCE_METHODS),
            (header::ACCESS_CONTROL_EXPOSE_HEADERS, "Allow"),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
        ],
    )
        .into_response()
}

/// Generic holder response, mostly for errors
struct BlossomGenericResponse {
    pub message: Option<String>,
    pub status: StatusCode,
    pub payment_headers: Option<PaymentHeaders>,
}

#[derive(Debug, Clone)]
struct PaymentHeaders {
    pub lightning: Option<String>,
    pub cashu: Option<String>,
}

impl IntoResponse for BlossomGenericResponse {
    fn into_response(self) -> Response {
        let mut headers = HeaderMap::new();
        headers.insert("access-control-allow-origin", "*".parse().unwrap());
        if let Some(message) = self.message
            && let Ok(value) = message.parse()
        {
            headers.insert("x-reason", value);
        }
        // Add payment headers if present
        if let Some(payment) = self.payment_headers {
            if let Some(lightning) = payment.lightning
                && let Ok(v) = lightning.parse() {
                    headers.insert("x-lightning", v);
                }
            if let Some(cashu) = payment.cashu
                && let Ok(v) = cashu.parse() {
                    headers.insert("x-cashu", v);
                }
        }
        (self.status, headers).into_response()
    }
}

enum BlossomResponse {
    Generic(BlossomGenericResponse),
    BlobDescriptorCreated(Json<BlobDescriptor>),
    BlobDescriptorOk(Json<BlobDescriptor>),
    BlobDescriptorList(Json<Vec<BlobDescriptor>>),
    /// BUD-12: identical media detected; 409 with X-Identical-Media header
    IdenticalMedia(String),
}

impl IntoResponse for BlossomResponse {
    fn into_response(self) -> Response {
        match self {
            BlossomResponse::Generic(g) => g.into_response(),
            BlossomResponse::BlobDescriptorCreated(j) => (StatusCode::CREATED, j).into_response(),
            BlossomResponse::BlobDescriptorOk(j) => (StatusCode::OK, j).into_response(),
            BlossomResponse::BlobDescriptorList(j) => (StatusCode::OK, j).into_response(),
            BlossomResponse::IdenticalMedia(sha256) => {
                let mut headers = HeaderMap::new();
                if let Ok(v) = sha256.parse() {
                    headers.insert("x-identical-media", v);
                }
                if let Ok(v) = "An identical image already exists on this server. \
                     Use the hash above to mirror it to other servers."
                    .parse()
                {
                    headers.insert("x-reason", v);
                }
                (StatusCode::CONFLICT, headers).into_response()
            }
        }
    }
}

impl BlossomResponse {
    pub fn error(msg: impl Into<String>) -> Self {
        Self::Generic(BlossomGenericResponse {
            message: Some(msg.into()),
            status: StatusCode::INTERNAL_SERVER_ERROR,
            payment_headers: None,
        })
    }

    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::Generic(BlossomGenericResponse {
            message: Some(msg.into()),
            status: StatusCode::BAD_REQUEST,
            payment_headers: None,
        })
    }

    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self::Generic(BlossomGenericResponse {
            message: Some(msg.into()),
            status: StatusCode::UNAUTHORIZED,
            payment_headers: None,
        })
    }

    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self::Generic(BlossomGenericResponse {
            message: Some(msg.into()),
            status: StatusCode::FORBIDDEN,
            payment_headers: None,
        })
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::Generic(BlossomGenericResponse {
            message: Some(msg.into()),
            status: StatusCode::NOT_FOUND,
            payment_headers: None,
        })
    }

    pub fn payment_required(msg: impl Into<String>) -> Self {
        Self::Generic(BlossomGenericResponse {
            message: Some(msg.into()),
            status: StatusCode::PAYMENT_REQUIRED,
            payment_headers: None,
        })
    }

    pub fn payment_required_with_headers(
        msg: impl Into<String>,
        lightning: Option<String>,
        cashu: Option<String>,
    ) -> Self {
        Self::Generic(BlossomGenericResponse {
            message: Some(msg.into()),
            status: StatusCode::PAYMENT_REQUIRED,
            payment_headers: Some(PaymentHeaders { lightning, cashu }),
        })
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::Generic(BlossomGenericResponse {
            message: Some(msg.into()),
            status: StatusCode::CONFLICT,
            payment_headers: None,
        })
    }

    pub fn length_required(msg: impl Into<String>) -> Self {
        Self::Generic(BlossomGenericResponse {
            message: Some(msg.into()),
            status: StatusCode::LENGTH_REQUIRED,
            payment_headers: None,
        })
    }

    pub fn content_too_large(msg: impl Into<String>) -> Self {
        Self::Generic(BlossomGenericResponse {
            message: Some(msg.into()),
            status: StatusCode::PAYLOAD_TOO_LARGE,
            payment_headers: None,
        })
    }

    pub fn unsupported_media_type(msg: impl Into<String>) -> Self {
        Self::Generic(BlossomGenericResponse {
            message: Some(msg.into()),
            status: StatusCode::UNSUPPORTED_MEDIA_TYPE,
            payment_headers: None,
        })
    }

    pub fn too_many_requests(msg: impl Into<String>) -> Self {
        Self::Generic(BlossomGenericResponse {
            message: Some(msg.into()),
            status: StatusCode::TOO_MANY_REQUESTS,
            payment_headers: None,
        })
    }

    pub fn unprocessable_content(msg: impl Into<String>) -> Self {
        Self::Generic(BlossomGenericResponse {
            message: Some(msg.into()),
            status: StatusCode::UNPROCESSABLE_ENTITY,
            payment_headers: None,
        })
    }

    pub fn bad_gateway(msg: impl Into<String>) -> Self {
        Self::Generic(BlossomGenericResponse {
            message: Some(msg.into()),
            status: StatusCode::BAD_GATEWAY,
            payment_headers: None,
        })
    }

    pub fn service_unavailable(msg: impl Into<String>) -> Self {
        Self::Generic(BlossomGenericResponse {
            message: Some(msg.into()),
            status: StatusCode::SERVICE_UNAVAILABLE,
            payment_headers: None,
        })
    }
}

struct BlossomHead {
    pub msg: Option<&'static str>,
    pub status: StatusCode,
}

impl IntoResponse for BlossomHead {
    fn into_response(self) -> Response {
        match self.msg {
            Some(m) => {
                let mut headers = HeaderMap::new();
                if let Ok(v) = m.parse() {
                    headers.insert("x-reason", v);
                }
                (self.status, headers).into_response()
            }
            None => self.status.into_response(),
        }
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

async fn check_whitelist(
    auth: &BlossomAuth,
    whitelist: &Whitelist,
    db: &Database,
) -> Option<BlossomResponse> {
    if !whitelist.is_allowed(&auth.event.pubkey.to_hex()).await {
        return Some(BlossomResponse::Generic(BlossomGenericResponse {
            status: StatusCode::FORBIDDEN,
            message: Some("Not on whitelist".to_string()),
            payment_headers: None,
        }));
    }
    if let Some(msg) = ban_check(db, &auth.event.pubkey.to_bytes().to_vec()).await {
        return Some(BlossomResponse::forbidden(msg));
    }
    None
}

/// Validate server tag against the server's domain
fn check_server_tag(auth: &BlossomAuth, server_domain: &str) -> Option<BlossomResponse> {
    if auth.validate_server_tag(server_domain).is_err() {
        return Some(BlossomResponse::unauthorized("Server not in authorization token scope"));
    }
    None
}

async fn delete_blob(
    axum::extract::Path(sha256): axum::extract::Path<String>,
    auth: BlossomAuth,
    AxumState(state): AxumState<Arc<AppState>>,
) -> BlossomResponse {
    let settings = state.settings().await;
    
    // BUD-11: validate x tag for DELETE endpoint
    if auth.validate_x_tag(&sha256).is_err() {
        return BlossomResponse::unauthorized("Missing or mismatched x tag");
    }
    
    // BUD-11: validate server tag
    if let Some(e) = check_server_tag(&auth, &settings.public_url) {
        return e;
    }
    
    match delete_file(&sha256, &auth.event, &state.fs, &state.db).await {
        Ok(()) => BlossomResponse::Generic(BlossomGenericResponse {
            status: StatusCode::NO_CONTENT,
            message: None,
            payment_headers: None,
        }),
        Err(e) => {
            if e.to_string().contains("not found") {
                BlossomResponse::not_found(format!("File not found: {}", e))
            } else {
                BlossomResponse::service_unavailable(format!("Failed to delete file: {}", e))
            }
        }
    }
}

async fn list_files(
    axum::extract::Path(pubkey): axum::extract::Path<String>,
    auth: BlossomAuth,
    AxumState(state): AxumState<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<ListFilesParams>,
) -> BlossomResponse {
    let settings = state.settings().await;
    
    // BUD-11: validate server tag for list endpoint
    if let Some(e) = check_server_tag(&auth, &settings.public_url) {
        return e;
    }
    
    let id = if let Ok(i) = hex::decode(&pubkey) {
        i
    } else {
        return BlossomResponse::bad_request("invalid pubkey");
    };
    
    let limit = params.limit.unwrap_or(50).min(5000); // Max 5000 per BUD-12
    let cursor = params.cursor.as_deref();
    
    match state.db.list_files_cursor(&id, cursor, limit).await {
        Ok(files) => BlossomResponse::BlobDescriptorList(Json(
            files
                .iter()
                .map(|f| BlobDescriptor::from_upload(&settings, f))
                .collect(),
        )),
        Err(e) => BlossomResponse::service_unavailable(format!("Could not list files: {}", e)),
    }
}

#[derive(serde::Deserialize, Debug)]
struct ListFilesParams {
    cursor: Option<String>,
    limit: Option<u32>,
}

async fn upload_head(auth: BlossomAuth, AxumState(state): AxumState<Arc<AppState>>) -> BlossomHead {
    let settings = state.settings().await;
    check_head(
        auth,
        &state.wl().await,
        &state.db,
        &settings,
        &settings.public_url,
    )
    .await
}

async fn upload(
    auth: BlossomAuth,
    AxumState(state): AxumState<Arc<AppState>>,
    body: Body,
) -> BlossomResponse {
    process_upload("upload", false, auth, state, body).await
}

async fn upload_by_hash(
    Path(sha256): Path<String>,
    RawQuery(raw_query): RawQuery,
    auth: BlossomAuth,
    AxumState(state): AxumState<Arc<AppState>>,
    body: Body,
) -> BlossomResponse {
    if !is_bud13_sha256(&sha256) {
        return BlossomResponse::bad_request("Invalid sha256 path");
    }
    if has_url_query(raw_query.as_deref()) {
        return BlossomResponse::bad_request(
            "The url query parameter is not supported for PUT /<sha256>",
        );
    }

    process_upload_with_expected_hash("upload", false, auth, state, body, Some(&sha256)).await
}

async fn mirror(
    auth: BlossomAuth,
    AxumState(state): AxumState<Arc<AppState>>,
    body: String,
) -> BlossomResponse {
    let req: MirrorRequest = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(_) => return BlossomResponse::bad_request("Invalid request body"),
    };
    let settings = state.settings().await;
    
    if !check_method(&auth.event, "upload") {
        return BlossomResponse::bad_request("Invalid request method tag");
    }
    
    // BUD-11: validate x tag for mirror endpoint
    // Extract expected hash from URL and validate against x tags
    if let Some(url_hash) = url_hash_from_url(&req.url)
        && auth.validate_x_tag(&url_hash).is_err() {
            return BlossomResponse::unauthorized("Missing or mismatched x tag");
        }
    
    // BUD-11: validate server tag
    if let Some(e) = check_server_tag(&auth, &settings.public_url) {
        return e;
    }
    
    if let Some(e) = check_whitelist(&auth, &state.wl().await, &state.db).await {
        return e;
    }

    let url = match Url::parse(&req.url) {
        Ok(u) => u,
        Err(e) => return BlossomResponse::bad_request(format!("Invalid URL: {}", e)),
    };

    // SSRF protection: only allow fetching public http(s) URLs.
    let validated_addrs = match BlossomAuth::validate_mirror_url(&url).await {
        Ok(addrs) => addrs,
        Err(_) => return BlossomResponse::bad_request("URL is not fetchable by this server"),
    };
    let validated_host = match url.host_str() {
        Some(h) => h.trim_start_matches('[').trim_end_matches(']').to_string(),
        None => return BlossomResponse::bad_request("URL is not fetchable by this server"),
    };

    let hash = url
        .path_segments()
        .and_then(|mut c| c.next_back())
        .and_then(|s| s.split(".").next());

    let client = match Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .connect_timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        // Pin the addresses validated above. Without this the client performs
        // its own DNS lookup, which can return a private address the check
        // never saw (DNS rebinding) — defeating the SSRF guard entirely.
        .resolve_to_addrs(&validated_host, &validated_addrs)
        // Ignore HTTP(S)_PROXY from the environment: a proxy would tunnel the
        // request past the pinned addresses and the IP checks.
        .no_proxy()
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to build HTTP client: {}", e);
            return BlossomResponse::service_unavailable("Mirror fetch failed");
        }
    };

    let req_builder = client.get(url.clone()).header(
        "user-agent",
        format!("route96 ({})", state.settings().await.public_url),
    );
    info!("Requesting mirror: {}", url);
    info!("{:?}", req_builder);

    // download file
    let rsp = match req_builder.send().await {
        Err(e) => {
            error!("Error downloading file: {}", e);
            return BlossomResponse::bad_gateway("Failed to fetch blob from origin URL");
        }
        Ok(rsp) if !rsp.status().is_success() => {
            let status = rsp.status();
            let body = rsp.bytes().await.unwrap_or(Default::default());
            error!(
                "Error downloading file, status is not OK({}): {}",
                status,
                String::from_utf8_lossy(&body)
            );
            return BlossomResponse::bad_gateway("Failed to fetch blob from origin URL");
        }
        Ok(rsp) => rsp,
    };

    let mime_type = rsp
        .headers()
        .get("content-type")
        // A mirror origin is untrusted: HeaderValue permits obs-text bytes
        // (0x80-0xFF) that to_str() rejects, so unwrap() here was a remotely
        // triggerable panic. Fall back to octet-stream on any non-ASCII value.
        .and_then(|h| h.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let pubkey = auth.event.pubkey.to_bytes().to_vec();

    process_stream(
        StreamReader::new(
            rsp.bytes_stream()
                .map(|result| result.map_err(std::io::Error::other)),
        ),
        &mime_type,
        &None,
        &pubkey,
        false,
        0, // No size info for mirror
        state,
        hash.and_then(|h| hex::decode(h).ok()),
        None,
        None,
    )
    .await
}

#[cfg(feature = "media-compression")]
async fn head_media(auth: BlossomAuth, AxumState(state): AxumState<Arc<AppState>>) -> BlossomHead {
    let settings = state.settings().await;
    check_head_media(
        auth,
        &state.wl().await,
        &state.db,
        &settings,
        &settings.public_url,
    )
    .await
}

#[cfg(feature = "media-compression")]
async fn check_head_media(
    auth: BlossomAuth,
    whitelist: &Whitelist,
    db: &Database,
    settings: &Settings,
    server_domain: &str,
) -> BlossomHead {
    if !check_method(&auth.event, "media") {
        return BlossomHead {
            msg: Some("Invalid auth method tag"),
            status: StatusCode::BAD_REQUEST,
        };
    }

    // BUD-06: X-Content-Length is required for HEAD /media
    let content_length = match auth.x_content_length {
        Some(z) => z,
        None => {
            return BlossomHead {
                msg: Some("X-Content-Length header required"),
                status: StatusCode::LENGTH_REQUIRED,
            };
        }
    };

    // Check size limit
    if content_length > settings.max_upload_bytes {
        return BlossomHead {
            msg: Some("File too large"),
            status: StatusCode::PAYLOAD_TOO_LARGE,
        };
    }

    // BUD-06: X-SHA-256 is required for HEAD /media
    let sha = match &auth.x_sha_256 {
        Some(s) => s.clone(),
        None => {
            return BlossomHead {
                msg: Some("X-SHA-256 header required"),
                status: StatusCode::BAD_REQUEST,
            };
        }
    };

    // Validate X-SHA-256 is valid hex
    if hex::decode(&sha).is_err() {
        return BlossomHead {
            msg: Some("X-SHA-256 must be valid hex"),
            status: StatusCode::BAD_REQUEST,
        };
    }

    // check whitelist
    if !whitelist.is_allowed(&auth.event.pubkey.to_hex()).await {
        return BlossomHead {
            msg: Some("Not on whitelist"),
            status: StatusCode::FORBIDDEN,
        };
    }

    if ban_check(db, &auth.event.pubkey.to_bytes().to_vec())
        .await
        .is_some()
    {
        return BlossomHead {
            msg: Some("Pubkey is banned"),
            status: StatusCode::FORBIDDEN,
        };
    }

    // BUD-11: validate server tag
    if auth.validate_server_tag(server_domain).is_err() {
        return BlossomHead {
            msg: Some("Server not in authorization token scope"),
            status: StatusCode::UNAUTHORIZED,
        };
    }

    BlossomHead { msg: None, status: StatusCode::OK }
}

#[cfg(feature = "media-compression")]
async fn upload_media(
    auth: BlossomAuth,
    AxumState(state): AxumState<Arc<AppState>>,
    body: Body,
) -> BlossomResponse {
    process_upload("media", true, auth, state, body).await
}

async fn check_head(
    auth: BlossomAuth,
    whitelist: &Whitelist,
    db: &Database,
    settings: &Settings,
    server_domain: &str,
) -> BlossomHead {
    if !check_method(&auth.event, "upload") {
        return BlossomHead {
            msg: Some("Invalid auth method tag"),
            status: StatusCode::BAD_REQUEST,
        };
    }

    // BUD-06: X-Content-Length is required for HEAD /upload
    let content_length = match auth.x_content_length {
        Some(z) => z,
        None => {
            return BlossomHead {
                msg: Some("X-Content-Length header required"),
                status: StatusCode::LENGTH_REQUIRED,
            };
        }
    };

    // Check size limit
    if content_length > settings.max_upload_bytes {
        return BlossomHead {
            msg: Some("File too large"),
            status: StatusCode::PAYLOAD_TOO_LARGE,
        };
    }

    // BUD-06: X-SHA-256 is required for HEAD /upload
    let sha = match &auth.x_sha_256 {
        Some(s) => s.clone(),
        None => {
            return BlossomHead {
                msg: Some("X-SHA-256 header required"),
                status: StatusCode::BAD_REQUEST,
            };
        }
    };

    // Validate X-SHA-256 is valid hex
    if hex::decode(&sha).is_err() {
        return BlossomHead {
            msg: Some("X-SHA-256 must be valid hex"),
            status: StatusCode::BAD_REQUEST,
        };
    }

    // check whitelist
    if !whitelist.is_allowed(&auth.event.pubkey.to_hex()).await {
        return BlossomHead {
            msg: Some("Not on whitelist"),
            status: StatusCode::FORBIDDEN,
        };
    }

    if ban_check(db, &auth.event.pubkey.to_bytes().to_vec())
        .await
        .is_some()
    {
        return BlossomHead {
            msg: Some("Pubkey is banned"),
            status: StatusCode::FORBIDDEN,
        };
    }

    // BUD-11: validate server tag
    if auth.validate_server_tag(server_domain).is_err() {
        return BlossomHead {
            msg: Some("Server not in authorization token scope"),
            status: StatusCode::UNAUTHORIZED,
        };
    }

    BlossomHead { msg: None, status: StatusCode::OK }
}

async fn process_upload(
    method: &str,
    compress: bool,
    auth: BlossomAuth,
    state: Arc<AppState>,
    body: Body,
) -> BlossomResponse {
    process_upload_with_expected_hash(method, compress, auth, state, body, None).await
}

async fn process_upload_with_expected_hash(
    method: &str,
    compress: bool,
    auth: BlossomAuth,
    state: Arc<AppState>,
    body: Body,
    expected_hash: Option<&str>,
) -> BlossomResponse {
    if !check_method(&auth.event, method) {
        return BlossomResponse::bad_request("Invalid request method tag");
    }

    // BUD-11: BUD-13 requires an x tag matching the path hash. Legacy uploads
    // validate the x tag when X-SHA-256 is supplied.
    let authorization_hash = expected_hash.or(auth.x_sha_256.as_deref());
    if let Some(hash) = authorization_hash
        && auth.validate_x_tag(hash).is_err()
    {
        return BlossomResponse::unauthorized("Missing or mismatched x tag");
    }

    let name = auth.event.tags.iter().find_map(|t| {
        if t.kind() == TagKind::Name {
            t.content()
        } else {
            None
        }
    });
    let size_tag = auth.event.tags.iter().find_map(|t| {
        if t.kind() == TagKind::Size {
            t.content().and_then(|v| v.parse::<u64>().ok())
        } else {
            None
        }
    });

    let size = size_tag.or(auth.x_content_length).unwrap_or(0);
    let settings = state.settings().await;
    if size > 0 && size > settings.max_upload_bytes {
        return BlossomResponse::content_too_large("File too large");
    }

    // check whitelist
    if let Some(e) = check_whitelist(&auth, &state.wl().await, &state.db).await {
        return e;
    }

    // BUD-11: validate server tag
    if let Some(e) = check_server_tag(&auth, &settings.public_url) {
        return e;
    }

    let data_stream = body.into_data_stream();
    let stream = TryStreamExt::map_err(data_stream, std::io::Error::other);
    let reader = StreamReader::new(stream);

    // BUD-13 uses the path as the authoritative hash. X-SHA-256 remains a
    // legacy PUT /upload check and is ignored for path-based uploads.
    let x_sha_256 = expected_hash
        .is_none()
        .then_some(auth.x_sha_256.as_deref())
        .flatten();

    process_stream(
        reader,
        &auth
            .content_type
            .unwrap_or("application/octet-stream".to_string()),
        &name,
        &auth.event.pubkey.to_bytes().to_vec(),
        compress,
        size,
        state,
        expected_hash.and_then(|hash| hex::decode(hash).ok()),
        auth.x_identical_media,
        x_sha_256,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn process_stream<'p, S>(
    stream: S,
    mime_type: &str,
    name: &Option<&str>,
    pubkey: &Vec<u8>,
    compress: bool,
    #[cfg(feature = "payments")] size: u64,
    #[cfg(not(feature = "payments"))] _size: u64,
    state: Arc<AppState>,
    // If Some, this is the SHA-256 the client echoed back via X-Identical-Media,
    // acknowledging a prior 409 and requesting to skip deduplication.
    expect_hash: Option<Vec<u8>>,
    acknowledged_identical: Option<Vec<u8>>,
    // X-SHA-256 header value — if provided, actual file hash must match
    x_sha_256: Option<&str>,
) -> BlossomResponse
where
    S: AsyncRead + Unpin + 'p,
{
    let settings = state.settings().await;
    let x_sha_256_hash = x_sha_256.and_then(|expected| hex::decode(expected).ok());
    let expected_input_hash = expect_hash.as_deref().or(x_sha_256_hash.as_deref());

    let mut is_new_file = false;
    let filesystem_result = match state
        .fs
        .put_with_expected_hash(&state.db, stream, mime_type, compress, expected_input_hash)
        .await
    {
        Ok(result) => result,
        Err(e) if e.downcast_ref::<HashMismatch>().is_some() => {
            return BlossomResponse::conflict("SHA-256 hash mismatch");
        }
        Err(e) => {
            error!("{}", e);
            if e.to_string().contains("exceeds maximum upload size") {
                return BlossomResponse::content_too_large("File too large");
            }
            return BlossomResponse::service_unavailable(format!(
                "Error saving file (disk): {}",
                e
            ));
        }
    };

    let upload = match filesystem_result {
        FileSystemResult::NewFile(blob) => {
            is_new_file = true;
            let mut ret: FileUpload = (&blob).into();

            // Check for sensitive EXIF metadata if enabled
            #[cfg(feature = "blossom")]
            if settings.reject_sensitive_exif.unwrap_or(false) && mime_type.starts_with("image/") {
                let file_path = state.fs.get(&ret.id);
                if let Err(e) = crate::exif_validator::check_for_sensitive_exif(&file_path) {
                    if let Err(cleanup_err) = state.fs.delete(&ret.id).await {
                        log::warn!(
                            "Failed to cleanup file with sensitive EXIF: {}",
                            cleanup_err
                        );
                    }
                    return BlossomResponse::unprocessable_content(format!("Upload rejected: {}", e));
                }
            }

            // Check for steganography/hidden data if enabled
            #[cfg(feature = "blossom")]
            if settings.reject_steganography.unwrap_or(false) && mime_type.starts_with("image/") {
                let file_path = state.fs.get(&ret.id);
                if let Err(e) = crate::steganography_detector::check_for_steganography(&file_path) {
                    if let Err(cleanup_err) = state.fs.delete(&ret.id).await {
                        log::warn!(
                            "Failed to cleanup file with steganography: {}",
                            cleanup_err
                        );
                    }
                    return BlossomResponse::unprocessable_content(format!("Upload rejected: {}", e));
                }
            }

            // update file data before inserting
            ret.name = name.map(|s| s.to_string());

            // BUD-12: identical media deduplication.
            // phash was computed inside fs.put; we just query for similar images here.
            // Skipped when the client echoes back X-Identical-Media and the server
            // is configured to allow client overrides.
            #[cfg(feature = "media-compression")]
            let client_override = acknowledged_identical.is_some()
                && settings
                    .identical_media_dedup_allow_override
                    .unwrap_or(true);
            #[cfg(feature = "media-compression")]
            if settings.identical_media_dedup.unwrap_or(false)
                && !client_override
                && let Some(hash_bytes) = blob.phash
            {
                let max_distance = settings.identical_media_dedup_distance.unwrap_or(0);
                match state
                    .db
                    .find_similar_images(&hash_bytes, max_distance, Some(&blob.id))
                    .await
                {
                    Ok(matches) if !matches.is_empty() => {
                        let existing_sha256 = hex::encode(&matches[0].0);
                        if let Err(e) = state.fs.delete(&blob.id).await {
                            log::warn!("BUD-12: failed to remove duplicate file: {}", e);
                        }
                        return BlossomResponse::IdenticalMedia(existing_sha256);
                    }
                    Err(e) => log::warn!("BUD-12: phash similarity query failed: {}", e),
                    Ok(_) => {} // no match — proceed normally
                }
            }

            ret
        }
        FileSystemResult::Banned => {
            return BlossomResponse::Generic(BlossomGenericResponse {
                message: Some("File is not allowed on this server".to_string()),
                status: StatusCode::FORBIDDEN,
                payment_headers: None,
            });
        }
        FileSystemResult::AlreadyExists(i) => match state.db.get_file(&i).await {
            Ok(Some(f)) if !f.banned => f,
            Ok(Some(_)) => {
                return BlossomResponse::Generic(BlossomGenericResponse {
                    message: Some("File is not allowed on this server".to_string()),
                    status: StatusCode::FORBIDDEN,
                    payment_headers: None,
                });
            }
            _ => return BlossomResponse::not_found("File not found"),
        },
    };

    let user_id = match state.db.upsert_user(pubkey).await {
        Ok(u) => u,
        Err(e) => {
            return BlossomResponse::service_unavailable(format!("Failed to save file (db): {}", e));
        }
    };

    // Post-upload quota check, using the actual stored size. The declared
    // size is only a hint and is never trusted for quota enforcement.
    #[cfg(feature = "payments")]
    {
        let _ = size;
        if is_new_file && let Some(payment_config) = &settings.payments {
            let free_quota = payment_config.free_quota_bytes.unwrap_or(104857600); // Default to 100MB

            match state
                .db
                .check_user_quota(pubkey, upload.size, free_quota)
                .await
            {
                Ok(false) => {
                    if let Err(e) = state.fs.delete(&upload.id).await {
                        log::warn!("Failed to cleanup quota-exceeding file: {}", e);
                    }
                    return BlossomResponse::content_too_large("Upload would exceed quota");
                }
                Err(_) => {
                    if let Err(e) = state.fs.delete(&upload.id).await {
                        log::warn!("Failed to cleanup file after quota check error: {}", e);
                    }
                    return BlossomResponse::service_unavailable("Failed to check quota");
                }
                Ok(true) => {} // Quota check passed
            }
        }
    }
    if let Err(e) = state.db.add_file(&upload, Some(user_id)).await {
        error!("{}", e);
        return BlossomResponse::service_unavailable(format!("Error saving file (db): {}", e));
    }

    // Return 201 for new files, 200 for existing
    if is_new_file {
        BlossomResponse::BlobDescriptorCreated(Json(BlobDescriptor::from_upload(&settings, &upload)))
    } else {
        BlossomResponse::BlobDescriptorOk(Json(BlobDescriptor::from_upload(&settings, &upload)))
    }
}

async fn report_file(
    AxumState(state): AxumState<Arc<AppState>>,
    Json(data): Json<nostr::Event>,
) -> BlossomResponse {
    // BUD-09: the body MUST be a signed NIP-56 report event (kind 1984)
    if data.kind != nostr::Kind::Custom(1984) {
        return BlossomResponse::bad_request("Event kind must be 1984 (NIP-56 report)");
    }

    // Verify the event signature
    if data.verify().is_err() {
        return BlossomResponse::bad_request("Invalid event signature");
    }

    // Extract all file SHA256 hashes from "x" tags
    let file_hashes: Vec<Vec<u8>> = data
        .tags
        .iter()
        .filter_map(|t| {
            if t.kind() == TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::X)) {
                t.content().and_then(|h| hex::decode(h).ok())
            } else {
                None
            }
        })
        .collect();

    if file_hashes.is_empty() {
        return BlossomResponse::bad_request("Missing file hash in x tag");
    }

    // Cap the number of files per report to bound request cost.
    if file_hashes.len() > 100 {
        return BlossomResponse::bad_request("Too many files in one report (max 100)");
    }

    // Only consider 32-byte hashes (SHA-256); shorter values are invalid.
    let file_hashes: Vec<Vec<u8>> = file_hashes
        .into_iter()
        .filter(|h| h.len() == 32)
        .collect();
    if file_hashes.is_empty() {
        return BlossomResponse::bad_request("No valid file hashes in x tags");
    }

    if let Some(msg) = ban_check(&state.db, &data.pubkey.to_bytes().to_vec()).await {
        return BlossomResponse::forbidden(msg);
    }

    // Get or create the reporter user from the report event pubkey
    let reporter_id = match state
        .db
        .upsert_user(&data.pubkey.to_bytes().to_vec())
        .await
    {
        Ok(user_id) => user_id,
        Err(e) => return BlossomResponse::service_unavailable(format!("Failed to get user: {}", e)),
    };

    let event_json = data.as_json();
    let mut errors = Vec::new();

    for file_sha256 in &file_hashes {
        // Verify the reported file exists
        match state.db.get_file(file_sha256).await {
            Ok(Some(_)) => {} // File exists, continue
            Ok(None) => {
                errors.push(format!(
                    "File {} not found",
                    hex::encode(file_sha256)
                ));
                continue;
            }
            Err(e) => {
                errors.push(format!(
                    "Failed to check file {}: {}",
                    hex::encode(file_sha256),
                    e
                ));
                continue;
            }
        }

        // Store the report
        if let Err(e) = state
            .db
            .add_report(file_sha256, reporter_id, &event_json)
            .await
            && !e.to_string().contains("Duplicate entry")
        {
            errors.push(format!(
                "Failed to submit report for {}: {}",
                hex::encode(file_sha256),
                e
            ));
            // Duplicate reports are silently skipped
        }
    }

    if errors.is_empty() {
        BlossomResponse::Generic(BlossomGenericResponse {
            status: StatusCode::OK,
            message: Some("Report submitted successfully".to_string()),
            payment_headers: None,
        })
    } else if errors.len() == file_hashes.len() {
        // All files failed
        BlossomResponse::bad_request(errors.join("; "))
    } else {
        // Some files succeeded, some failed
        BlossomResponse::Generic(BlossomGenericResponse {
            status: StatusCode::OK,
            message: Some(format!(
                "Report submitted with some errors: {}",
                errors.join("; ")
            )),
            payment_headers: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_stats::FileStatsTracker;
    use crate::filesystem::FileStore;
    use axum::extract::{Path, RawQuery, State};
    use nostr::{EventBuilder, Keys, Kind};
    use sha2::Digest;
    use sqlx::mysql::MySqlPoolOptions;
    use tempfile::TempDir;
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    fn test_auth() -> BlossomAuth {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::Custom(24242), "")
            .sign_with_keys(&keys)
            .unwrap();
        BlossomAuth {
            content_type: Some("application/octet-stream".to_string()),
            x_content_type: None,
            x_sha_256: None,
            x_content_length: None,
            x_identical_media: None,
            event,
        }
    }

    fn test_state() -> (Arc<AppState>, TempDir) {
        let temp = tempfile::tempdir().unwrap();
        let settings = Settings {
            listen: None,
            storage_dir: temp.path().to_string_lossy().into_owned(),
            database: "mysql://user:pass@127.0.0.1:1/route96".to_string(),
            max_upload_bytes: 1024,
            public_url: "https://example.com".to_string(),
            whitelist: None,
            #[cfg(feature = "labels")]
            models_dir: None,
            #[cfg(feature = "labels")]
            label_models: None,
            #[cfg(feature = "labels")]
            label_flag_terms: None,
            webhook_url: None,
            reject_sensitive_exif: None,
            reject_steganography: None,
            #[cfg(feature = "media-compression")]
            identical_media_dedup: None,
            #[cfg(feature = "media-compression")]
            identical_media_dedup_distance: None,
            #[cfg(feature = "media-compression")]
            identical_media_dedup_allow_override: None,
            #[cfg(feature = "payments")]
            payments: None,
            delete_unaccessed_days: None,
            delete_after_days: None,
            delete_zero_egress_days: None,
        };
        let settings = Arc::new(RwLock::new(settings));
        let pool = MySqlPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(50))
            .connect_lazy("mysql://user:pass@127.0.0.1:1/route96")
            .unwrap();
        let db = Database { pool };
        let state = Arc::new(AppState {
            fs: FileStore::new(settings.clone()),
            db,
            config_path: String::new(),
            settings,
            wl: Arc::new(RwLock::new(Whitelist::default())),
            file_stats: FileStatsTracker::new(),
            #[cfg(feature = "payments")]
            lnd: None,
        });
        (state, temp)
    }

    #[test]
    fn bud13_hash_requires_lowercase_sha256() {
        assert!(is_bud13_sha256(&"a".repeat(64)));
        assert!(is_bud13_sha256(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
        assert!(!is_bud13_sha256(&"A".repeat(64)));
        assert!(!is_bud13_sha256(&"g".repeat(64)));
        assert!(!is_bud13_sha256(&"a".repeat(63)));
        assert!(!is_bud13_sha256(&("a".repeat(64) + ".jpg")));
    }

    #[test]
    fn detects_url_query_parameter_after_decoding() {
        assert!(has_url_query(Some("url=https%3A%2F%2Fexample.com")));
        assert!(has_url_query(Some("u%72l=https%3A%2F%2Fexample.com")));
        assert!(!has_url_query(Some("source=https%3A%2F%2Fexample.com")));
        assert!(!has_url_query(None));
    }

    #[tokio::test]
    async fn hash_options_advertises_put_but_not_post() {
        let hash = "a".repeat(64);
        let response = hash_options(Path(hash.clone())).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            response.headers().get(header::ALLOW).unwrap(),
            HASH_RESOURCE_METHODS
        );
        assert!(!response
            .headers()
            .get(header::ALLOW)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("POST"));

        let response = hash_options(Path("A".repeat(64))).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let (state, _temp) = test_state();
        let response = blossom_routes()
            .with_state(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("OPTIONS")
                    .uri(format!("/{hash}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn path_upload_rejects_invalid_path_and_remote_url_query() {
        let (state, _temp) = test_state();
        let response = upload_by_hash(
            Path("A".repeat(64)),
            RawQuery(None),
            test_auth(),
            State(state.clone()),
            Body::empty(),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let response = upload_by_hash(
            Path("a".repeat(64)),
            RawQuery(Some("url=https%3A%2F%2Fexample.com".to_string())),
            test_auth(),
            State(state.clone()),
            Body::empty(),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.headers().get("x-reason").unwrap(),
            "The url query parameter is not supported for PUT /<sha256>"
        );

        // A valid BUD-13 path reaches the shared upload authorization pipeline.
        let response = upload_by_hash(
            Path("a".repeat(64)),
            RawQuery(None),
            test_auth(),
            State(state),
            Body::empty(),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let reason = response.headers().get("x-reason").unwrap();
        assert_eq!(reason, "Invalid request method tag");

        let response = process_upload(
            "upload",
            false,
            test_auth(),
            test_state().0,
            Body::empty(),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn process_stream_maps_storage_failure_to_service_unavailable() {
        let (state, _temp) = test_state();
        let reader = tokio::io::empty();
        let response = process_stream(
            reader,
            "application/octet-stream",
            &None,
            &vec![1; 32],
            false,
            0,
            state,
            None,
            None,
            None,
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn process_stream_rejects_hash_mismatch_before_publishing() {
        let (state, _temp) = test_state();
        let content = b"mismatched content";
        let response = process_stream(
            std::io::Cursor::new(content),
            "application/octet-stream",
            &None,
            &vec![1; 32],
            false,
            0,
            state.clone(),
            Some(vec![0; 32]),
            None,
            None,
        )
        .await
        .into_response();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let actual_hash = sha2::Sha256::digest(content).to_vec();
        assert!(!state.fs.get(&actual_hash).exists());
    }

    #[tokio::test]
    async fn process_stream_returns_413_for_actual_oversized_body() {
        let (state, _temp) = test_state();
        let reader = std::io::Cursor::new(vec![0; 1025]);
        let response = process_stream(
            reader,
            "application/octet-stream",
            &None,
            &vec![1; 32],
            false,
            0,
            state,
            None,
            None,
            None,
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
