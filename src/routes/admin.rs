use crate::auth::nip98::Nip98Auth;
use crate::db::{Database, FileUpload, Report, ReviewState, User};
use crate::routes::{AppState, Nip94Event, PagedResult};
use axum::{
    Json, Router,
    extract::{Path, Query, State as AxumState},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get},
};
use serde::{Deserialize, Serialize};
use sqlx::{Error, QueryBuilder, Row};
use std::sync::Arc;

pub fn admin_routes() -> Router<Arc<AppState>> {
    #[allow(unused_mut)]
    let mut router = Router::new()
        .route("/admin/self", get(admin_get_self))
        .route("/admin/files", get(admin_list_files))
        .route(
            "/admin/files/review",
            get(admin_list_pending_review)
                .patch(admin_review_files)
                .delete(admin_delete_files),
        )
        .route("/admin/reports", get(admin_list_reports))
        .route("/admin/reports", delete(admin_acknowledge_reports))
        .route("/admin/user/{user_pubkey}", get(admin_get_user_info))
        .route("/admin/user/{user_pubkey}/purge", delete(admin_purge_user));

    #[cfg(feature = "media-compression")]
    {
        router = router.route("/admin/files/{file_id}/similar", get(admin_similar_files));
    }

    router
}

#[derive(Serialize, Default)]
struct AdminResponseBase<T> {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

enum AdminResponse<T> {
    GenericError(Json<AdminResponseBase<T>>),
    Ok(Json<AdminResponseBase<T>>),
}

impl<T> AdminResponse<T>
where
    T: Serialize,
{
    pub fn error(msg: &str) -> Self {
        Self::GenericError(Json(AdminResponseBase {
            status: "error".to_string(),
            message: Some(msg.to_string()),
            data: None,
        }))
    }

    pub fn success(msg: T) -> Self {
        Self::Ok(Json(AdminResponseBase {
            status: "success".to_string(),
            message: None,
            data: Some(msg),
        }))
    }
}

impl<T> IntoResponse for AdminResponse<T>
where
    T: Serialize,
{
    fn into_response(self) -> Response {
        match self {
            AdminResponse::GenericError(json) => {
                (StatusCode::INTERNAL_SERVER_ERROR, json).into_response()
            }
            AdminResponse::Ok(json) => (StatusCode::OK, json).into_response(),
        }
    }
}

#[derive(Serialize)]
pub struct SelfUser {
    pub is_admin: bool,
    pub file_count: u64,
    pub total_size: u64,
    #[cfg(feature = "payments")]
    pub paid_until: u64,
    #[cfg(feature = "payments")]
    pub quota: u64,
    #[cfg(feature = "payments")]
    pub free_quota: u64,
    #[cfg(feature = "payments")]
    pub total_available_quota: u64,
}

#[derive(Serialize)]
pub struct AdminNip94File {
    #[serde(flatten)]
    pub inner: Nip94Event,
    pub uploader: Vec<String>,
}

#[derive(Serialize)]
pub struct AdminUserInfo {
    pub pubkey: String,
    pub is_admin: bool,
    pub file_count: u64,
    pub total_size: u64,
    pub created: String,
    #[cfg(feature = "payments")]
    pub paid_until: u64,
    #[cfg(feature = "payments")]
    pub quota: u64,
    #[cfg(feature = "payments")]
    pub free_quota: u64,
    #[cfg(feature = "payments")]
    pub total_available_quota: u64,
    #[cfg(feature = "payments")]
    pub payments: Vec<crate::db::Payment>,
    pub files: PagedResult<AdminNip94File>,
}

/// Shared request body for batch file operations.
#[derive(Deserialize)]
struct AdminFileIdsBody {
    ids: Vec<String>,
}

impl AdminFileIdsBody {
    /// Decode all hex IDs, returning an error string on the first invalid one.
    fn decode(&self) -> Result<Vec<Vec<u8>>, String> {
        self.ids
            .iter()
            .map(|id| hex::decode(id).map_err(|_| format!("Invalid file id: {}", id)))
            .collect()
    }
}

/// Shared request body for batch report operations.
#[derive(Deserialize)]
struct AdminReportIdsBody {
    ids: Vec<u64>,
}

/// Verify the request comes from an admin. Returns the user on success.
async fn require_admin(auth: &Nip98Auth, db: &Database) -> Result<User, String> {
    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
    let user = db
        .get_user(&pubkey_vec)
        .await
        .map_err(|_| "User not found".to_string())?;
    if !user.is_admin {
        return Err("User is not an admin".to_string());
    }
    Ok(user)
}

async fn admin_get_self(
    auth: Nip98Auth,
    AxumState(state): AxumState<Arc<AppState>>,
) -> AdminResponse<SelfUser> {
    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
    match state.db.get_user(&pubkey_vec).await {
        Ok(user) => {
            let s = match state.db.get_user_stats(user.id).await {
                Ok(r) => r,
                Err(e) => {
                    return AdminResponse::error(&format!("Failed to load user stats: {}", e));
                }
            };

            #[cfg(feature = "payments")]
            let (free_quota, total_available_quota) = {
                if let Some(payment_config) = &state.settings.payments {
                    let free_quota = payment_config.free_quota_bytes.unwrap_or(104857600);
                    let mut total_available = free_quota;

                    // Add paid quota if still valid
                    if let Some(paid_until) = &user.paid_until {
                        if *paid_until > chrono::Utc::now() {
                            total_available += user.paid_size;
                        }
                    }

                    (free_quota, total_available)
                } else {
                    // No payments config - quota disabled
                    (0, 0)
                }
            };

            AdminResponse::success(SelfUser {
                is_admin: user.is_admin,
                file_count: s.file_count,
                total_size: s.total_size,
                #[cfg(feature = "payments")]
                paid_until: if let Some(u) = &user.paid_until {
                    u.timestamp() as u64
                } else {
                    0
                },
                #[cfg(feature = "payments")]
                quota: user.paid_size,
                #[cfg(feature = "payments")]
                free_quota,
                #[cfg(feature = "payments")]
                total_available_quota,
            })
        }
        Err(_) => AdminResponse::error("User not found"),
    }
}

#[derive(Deserialize)]
struct AdminListFilesQuery {
    #[serde(default)]
    page: u32,
    #[serde(default = "default_count")]
    count: u32,
    mime_type: Option<String>,
    /// Filter to files that have at least one label containing this substring
    /// (case-insensitive). Only available when the `labels` feature is enabled.
    label: Option<String>,
}

fn default_count() -> u32 {
    50
}

async fn admin_list_files(
    auth: Nip98Auth,
    Query(params): Query<AdminListFilesQuery>,
    AxumState(state): AxumState<Arc<AppState>>,
) -> AdminResponse<PagedResult<AdminNip94File>> {
    let server_count = params.count.clamp(1, 5_000);

    if let Err(e) = require_admin(&auth, &state.db).await {
        return AdminResponse::error(&e);
    }
    match state
        .db
        .list_all_files(
            params.page * server_count,
            server_count,
            params.mime_type,
            params.label,
        )
        .await
    {
        Ok((files, count)) => AdminResponse::success(PagedResult {
            count: files.len() as u32,
            page: params.page,
            total: count as u32,
            files: files
                .into_iter()
                .map(|f| AdminNip94File {
                    inner: Nip94Event::from_upload(&state.settings, &f.0),
                    uploader: f.1.into_iter().map(|u| hex::encode(&u.pubkey)).collect(),
                })
                .collect(),
        }),
        Err(e) => AdminResponse::error(&format!("Could not list files: {}", e)),
    }
}

#[derive(Deserialize)]
struct AdminListReportsQuery {
    #[serde(default)]
    page: u32,
    #[serde(default = "default_count")]
    count: u32,
}

async fn admin_list_reports(
    auth: Nip98Auth,
    Query(params): Query<AdminListReportsQuery>,
    AxumState(state): AxumState<Arc<AppState>>,
) -> AdminResponse<PagedResult<Report>> {
    let server_count = params.count.clamp(1, 5_000);

    if let Err(e) = require_admin(&auth, &state.db).await {
        return AdminResponse::error(&e);
    }

    match state
        .db
        .list_reports(params.page * server_count, server_count)
        .await
    {
        Ok((reports, total_count)) => AdminResponse::success(PagedResult {
            count: reports.len() as u32,
            page: params.page,
            total: total_count as u32,
            files: reports,
        }),
        Err(e) => AdminResponse::error(&format!("Could not list reports: {}", e)),
    }
}

/// DELETE /admin/reports — acknowledge (dismiss) reports by ID
async fn admin_acknowledge_reports(
    auth: Nip98Auth,
    AxumState(state): AxumState<Arc<AppState>>,
    Json(body): Json<AdminReportIdsBody>,
) -> AdminResponse<()> {
    if let Err(e) = require_admin(&auth, &state.db).await {
        return AdminResponse::error(&e);
    }

    if let Err(e) = state.db.mark_reports_reviewed(&body.ids).await {
        return AdminResponse::error(&format!("Failed to acknowledge reports: {}", e));
    }

    AdminResponse::success(())
}

#[derive(Deserialize)]
struct AdminGetUserInfoQuery {
    page: Option<u32>,
    count: Option<u32>,
}

async fn admin_get_user_info(
    auth: Nip98Auth,
    Path(user_pubkey): Path<String>,
    Query(params): Query<AdminGetUserInfoQuery>,
    AxumState(state): AxumState<Arc<AppState>>,
) -> AdminResponse<AdminUserInfo> {
    if let Err(e) = require_admin(&auth, &state.db).await {
        return AdminResponse::error(&e);
    }

    // Parse target user pubkey
    let target_pubkey = match hex::decode(&user_pubkey) {
        Ok(pk) => pk,
        Err(_) => return AdminResponse::error("Invalid pubkey format"),
    };

    // Get target user
    let target_user = match state.db.get_user(&target_pubkey).await {
        Ok(user) => user,
        Err(_) => return AdminResponse::error("Target user not found"),
    };

    // Get user stats
    let user_stats = match state.db.get_user_stats(target_user.id).await {
        Ok(stats) => stats,
        Err(e) => return AdminResponse::error(&format!("Failed to load user stats: {}", e)),
    };

    // Get user files with pagination
    let page = params.page.unwrap_or(0);
    let count = params.count.unwrap_or(50).clamp(1, 100);
    let (files, total_files) = match state
        .db
        .list_files(&target_pubkey, page * count, count)
        .await
    {
        Ok((files, total)) => (files, total),
        Err(e) => return AdminResponse::error(&format!("Failed to load user files: {}", e)),
    };

    let files_result = PagedResult {
        count: files.len() as u32,
        page,
        total: total_files as u32,
        files: files
            .into_iter()
            .map(|f| AdminNip94File {
                inner: Nip94Event::from_upload(&state.settings, &f),
                uploader: vec![hex::encode(&target_pubkey)],
            })
            .collect(),
    };

    #[cfg(feature = "payments")]
    let (free_quota, total_available_quota, payments) = {
        if let Some(payment_config) = &state.settings.payments {
            let free_quota = payment_config.free_quota_bytes.unwrap_or(104857600);
            let mut total_available = free_quota;

            // Add paid quota if still valid
            if let Some(paid_until) = &target_user.paid_until {
                if *paid_until > chrono::Utc::now() {
                    total_available += target_user.paid_size;
                }
            }

            let payments = state
                .db
                .get_user_payments(target_user.id)
                .await
                .unwrap_or_default();

            (free_quota, total_available, payments)
        } else {
            // No payments config - quota disabled
            (0, 0, vec![])
        }
    };

    AdminResponse::success(AdminUserInfo {
        pubkey: hex::encode(&target_pubkey),
        is_admin: target_user.is_admin,
        file_count: user_stats.file_count,
        total_size: user_stats.total_size,
        created: target_user.created.to_rfc3339(),
        #[cfg(feature = "payments")]
        paid_until: if let Some(u) = &target_user.paid_until {
            u.timestamp() as u64
        } else {
            0
        },
        #[cfg(feature = "payments")]
        quota: target_user.paid_size,
        #[cfg(feature = "payments")]
        free_quota,
        #[cfg(feature = "payments")]
        total_available_quota,
        #[cfg(feature = "payments")]
        payments,
        files: files_result,
    })
}

async fn admin_purge_user(
    auth: Nip98Auth,
    Path(user_pubkey): Path<String>,
    AxumState(state): AxumState<Arc<AppState>>,
) -> AdminResponse<()> {
    if let Err(e) = require_admin(&auth, &state.db).await {
        return AdminResponse::error(&e);
    }

    // Parse target user pubkey
    let target_pubkey = match hex::decode(&user_pubkey) {
        Ok(pk) => pk,
        Err(_) => return AdminResponse::error("Invalid pubkey format"),
    };

    // Bulk-delete DB records and get back the file IDs to remove from disk
    let file_ids = match state.db.purge_user_files(&target_pubkey).await {
        Ok(ids) => ids,
        Err(e) => return AdminResponse::error(&format!("Failed to purge user files: {}", e)),
    };

    let count = file_ids.len();

    // Delete physical files in the background so we can return immediately
    let fs = state.fs.clone();
    tokio::spawn(async move {
        for id in file_ids {
            if let Err(e) = tokio::fs::remove_file(fs.get(&id)).await {
                log::warn!("Failed to delete physical file {}: {}", hex::encode(&id), e);
            }
        }
    });

    AdminResponse::Ok(Json(AdminResponseBase {
        status: "success".to_string(),
        message: Some(format!("Deleting {} files", count)),
        data: None,
    }))
}

#[derive(Deserialize)]
struct AdminListPendingReviewQuery {
    #[serde(default)]
    page: u32,
    #[serde(default = "default_count")]
    count: u32,
}

async fn admin_list_pending_review(
    auth: Nip98Auth,
    Query(params): Query<AdminListPendingReviewQuery>,
    AxumState(state): AxumState<Arc<AppState>>,
) -> AdminResponse<PagedResult<AdminNip94File>> {
    let server_count = params.count.clamp(1, 5_000);

    if let Err(e) = require_admin(&auth, &state.db).await {
        return AdminResponse::error(&e);
    }

    match state
        .db
        .list_files_pending_review(params.page * server_count, server_count)
        .await
    {
        Ok((files, total)) => AdminResponse::success(PagedResult {
            count: files.len() as u32,
            page: params.page,
            total: total as u32,
            files: files
                .into_iter()
                .map(|f| AdminNip94File {
                    inner: Nip94Event::from_upload(&state.settings, &f.0),
                    uploader: f.1.into_iter().map(|u| hex::encode(&u.pubkey)).collect(),
                })
                .collect(),
        }),
        Err(e) => AdminResponse::error(&format!("Could not list pending review files: {}", e)),
    }
}

/// PATCH /admin/files/review — mark files as reviewed (clears flag)
async fn admin_review_files(
    auth: Nip98Auth,
    AxumState(state): AxumState<Arc<AppState>>,
    Json(body): Json<AdminFileIdsBody>,
) -> AdminResponse<()> {
    if let Err(e) = require_admin(&auth, &state.db).await {
        return AdminResponse::error(&e);
    }

    let ids = match body.decode() {
        Ok(ids) => ids,
        Err(e) => return AdminResponse::error(&e),
    };

    if let Err(e) = state
        .db
        .set_files_review_state(&ids, ReviewState::Reviewed)
        .await
    {
        return AdminResponse::error(&format!("Failed to review files: {}", e));
    }

    AdminResponse::success(())
}

/// DELETE /admin/files/review — ban files and remove from disk
async fn admin_delete_files(
    auth: Nip98Auth,
    AxumState(state): AxumState<Arc<AppState>>,
    Json(body): Json<AdminFileIdsBody>,
) -> AdminResponse<()> {
    if let Err(e) = require_admin(&auth, &state.db).await {
        return AdminResponse::error(&e);
    }

    let ids = match body.decode() {
        Ok(ids) => ids,
        Err(e) => return AdminResponse::error(&e),
    };

    // Ban all files in one transaction (ownership + tombstone).
    if let Err(e) = state.db.ban_files(&ids).await {
        return AdminResponse::error(&format!("Failed to ban files: {}", e));
    }

    // Remove physical files (best-effort, DB is already updated).
    for id in &ids {
        if let Err(e) = tokio::fs::remove_file(state.fs.get(id)).await {
            log::warn!("Could not remove file from disk {}: {}", hex::encode(id), e);
        }
    }

    AdminResponse::success(())
}

/// Query params for the similar-files endpoint.
#[derive(Deserialize)]
struct AdminSimilarFilesQuery {
    /// Maximum Hamming distance (default: `MAX_HAMMING_DISTANCE`).
    distance: Option<u32>,
}

#[derive(Serialize)]
struct SimilarFile {
    #[serde(flatten)]
    pub inner: Nip94Event,
    /// Hamming distance from the query image's pHash.
    pub distance: u32,
}

/// GET /admin/files/{file_id}/similar
///
/// Returns files whose perceptual hash is within `distance` Hamming bits of
/// the queried file (default: `MAX_HAMMING_DISTANCE` = 10).
/// Requires admin auth.
#[cfg(feature = "media-compression")]
async fn admin_similar_files(
    auth: Nip98Auth,
    Path(file_id): Path<String>,
    Query(params): Query<AdminSimilarFilesQuery>,
    AxumState(state): AxumState<Arc<AppState>>,
) -> AdminResponse<Vec<SimilarFile>> {
    use crate::phash::MAX_HAMMING_DISTANCE;

    if let Err(e) = require_admin(&auth, &state.db).await {
        return AdminResponse::error(&e);
    }

    let id = match hex::decode(&file_id) {
        Ok(id) => id,
        Err(_) => return AdminResponse::error("Invalid file id"),
    };

    let max_dist = params.distance.unwrap_or(MAX_HAMMING_DISTANCE);

    let query_hash = match state.db.get_phash(&id).await {
        Ok(Some(h)) => h,
        Ok(None) => return AdminResponse::error("No perceptual hash for this file yet"),
        Err(e) => return AdminResponse::error(&format!("DB error: {}", e)),
    };

    let candidates = match state
        .db
        .find_similar_images(&query_hash, max_dist, Some(&id))
        .await
    {
        Ok(c) => c,
        Err(e) => return AdminResponse::error(&format!("DB error: {}", e)),
    };

    let mut results = Vec::with_capacity(candidates.len());
    for (file_id_bytes, dist) in candidates {
        if let Ok(Some(upload)) = state.db.get_file(&file_id_bytes).await {
            results.push(SimilarFile {
                inner: Nip94Event::from_upload(&state.settings, &upload),
                distance: dist,
            });
        }
    }

    AdminResponse::success(results)
}

impl Database {
    /// Build the shared WHERE clause for `list_all_files` queries.
    fn build_all_files_where<'a>(
        qb: &mut QueryBuilder<'a, sqlx::MySql>,
        mime_type: &'a Option<String>,
        label: &'a Option<String>,
    ) {
        if let Some(m) = mime_type {
            qb.push("and u.mime_type like ");
            qb.push_bind(format!("%{}%", m));
            qb.push(" ");
        }
        if let Some(l) = label {
            qb.push(
                "and exists (select 1 from upload_labels ul where ul.file = u.id and ul.label = ",
            );
            qb.push_bind(l.clone());
            qb.push(") ");
        }
    }

    pub async fn list_all_files(
        &self,
        offset: u32,
        limit: u32,
        mime_type: Option<String>,
        label: Option<String>,
    ) -> Result<(Vec<(FileUpload, Vec<User>)>, i64), Error> {
        let mut q = QueryBuilder::new("select u.* from uploads u where u.banned = false ");
        Self::build_all_files_where(&mut q, &mime_type, &label);
        q.push("order by u.created desc limit ");
        q.push_bind(limit);
        q.push(" offset ");
        q.push_bind(offset);
        let results: Vec<FileUpload> = q.build_query_as().fetch_all(&self.pool).await?;

        let mut cq = QueryBuilder::new("select count(u.id) from uploads u where u.banned = false ");
        Self::build_all_files_where(&mut cq, &mime_type, &label);
        let count: i64 = cq.build().fetch_one(&self.pool).await?.try_get(0)?;

        #[allow(unused_mut)]
        let mut results = results;
        #[cfg(feature = "labels")]
        self.populate_labels_vec(&mut results).await?;

        let file_ids: Vec<&[u8]> = results.iter().map(|f| f.id.as_slice()).collect();
        let owners_map = self.get_file_owners_batch(&file_ids).await?;

        let res: Vec<(FileUpload, Vec<User>)> = results
            .into_iter()
            .map(|upload| {
                let owners = owners_map
                    .get(upload.id.as_slice())
                    .cloned()
                    .unwrap_or_default();
                (upload, owners)
            })
            .collect();
        Ok((res, count))
    }

    /// List files whose `review_state` is not `None` and not `Reviewed`,
    /// ordered oldest-first so the backlog drains naturally.
    pub async fn list_files_pending_review(
        &self,
        offset: u32,
        limit: u32,
    ) -> Result<(Vec<(FileUpload, Vec<User>)>, i64), Error> {
        let results: Vec<FileUpload> = sqlx::query_as(
            "select * from uploads \
             where banned = false and review_state != 0 and review_state != 3 \
             order by created asc limit ? offset ?",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let count: i64 = sqlx::query(
            "select count(id) from uploads \
             where banned = false and review_state != 0 and review_state != 3",
        )
        .fetch_one(&self.pool)
        .await?
        .try_get(0)?;

        #[allow(unused_mut)]
        let mut results = results;
        #[cfg(feature = "labels")]
        self.populate_labels_vec(&mut results).await?;

        let file_ids: Vec<&[u8]> = results.iter().map(|f| f.id.as_slice()).collect();
        let owners_map = self.get_file_owners_batch(&file_ids).await?;

        let res: Vec<(FileUpload, Vec<User>)> = results
            .into_iter()
            .map(|upload| {
                let owners = owners_map
                    .get(upload.id.as_slice())
                    .cloned()
                    .unwrap_or_default();
                (upload, owners)
            })
            .collect();
        Ok((res, count))
    }
}
