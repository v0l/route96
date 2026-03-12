use crate::auth::blossom::BlossomAuth;
use crate::auth::nip98::Nip98Auth;
use crate::db::{Database, FileStatSort, FileUpload, FileUploadWithStats, Report, ReviewState, SortOrder, User, WhitelistEntry};
use crate::file_stats::FileStats;
use crate::routes::{AppState, Nip94Event, PagedResult};
use axum::{
    Json, Router,
    extract::{Path, Query, State as AxumState},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, put},
};
use serde::{Deserialize, Serialize};
use sqlx::{Error, QueryBuilder, Row};
use std::sync::Arc;

pub fn admin_routes() -> Router<Arc<AppState>> {
    #[allow(unused_mut)]
    let mut router = Router::new()
        .route("/admin/self", get(admin_get_self))
        .route("/user/files", get(user_list_files))
        .route("/admin/files", get(admin_list_files))
        .route(
            "/admin/files/review",
            get(admin_list_pending_review)
                .patch(admin_review_files)
                .delete(admin_delete_files),
        )
        .route("/admin/files/{file_id}/stats", get(admin_file_stats))
        .route("/admin/reports", get(admin_list_reports))
        .route("/admin/reports", delete(admin_acknowledge_reports))
        .route("/admin/user/{user_pubkey}", get(admin_get_user_info))
        .route("/admin/user/{user_pubkey}/purge", delete(admin_purge_user))
        .route(
            "/admin/whitelist",
            get(admin_list_whitelist)
                .post(admin_add_whitelist)
                .delete(admin_remove_whitelist),
        )
        .route(
            "/admin/config",
            get(admin_list_config),
        )
        .route(
            "/admin/config/{key}",
            put(admin_set_config).delete(admin_delete_config),
        );

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
pub struct Route96File {
    #[serde(flatten)]
    pub inner: Nip94Event,
    pub uploader: Vec<String>,
    pub stats: FileStats,
}

/// A file entry returned by the user-facing list endpoint.
///
/// Similar to [`Route96File`] but without the `uploader` field — callers
/// are always the owner by definition.
#[derive(Serialize)]
pub struct UserFile {
    #[serde(flatten)]
    pub inner: Nip94Event,
    pub stats: FileStats,
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
    pub files: PagedResult<Route96File>,
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
                if let Some(payment_config) = &state.settings().await.payments {
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
    #[serde(default)]
    sort: FileStatSort,
    #[serde(default)]
    order: SortOrder,
}

fn default_count() -> u32 {
    50
}

async fn admin_list_files(
    auth: Nip98Auth,
    Query(params): Query<AdminListFilesQuery>,
    AxumState(state): AxumState<Arc<AppState>>,
) -> AdminResponse<PagedResult<Route96File>> {
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
            params.sort,
            params.order,
        )
        .await
    {
        Ok((files, count)) => {
            let settings = state.settings().await;
            AdminResponse::success(PagedResult {
                count: files.len() as u32,
                page: params.page,
                total: count as u32,
                files: files
                    .into_iter()
                    .map(|(upload, stats, owners)| Route96File {
                        stats,
                        inner: Nip94Event::from_upload(&settings, &upload),
                        uploader: owners.into_iter().map(|u| hex::encode(&u.pubkey)).collect(),
                    })
                    .collect(),
            })
        }
        Err(e) => AdminResponse::error(&format!("Could not list files: {}", e)),
    }
}

/// Query parameters for `GET /user/files`.
#[derive(Deserialize)]
struct UserListFilesQuery {
    #[serde(default)]
    page: u32,
    #[serde(default = "default_count")]
    count: u32,
    mime_type: Option<String>,
    /// Filter to files that have at least one label containing this substring
    /// (case-insensitive). Only available when the `labels` feature is enabled.
    label: Option<String>,
    #[serde(default)]
    sort: FileStatSort,
    #[serde(default)]
    order: SortOrder,
}

/// `GET /user/files` — list the authenticated user's own files with full
/// metadata and access statistics.
///
/// This endpoint is the non-admin equivalent of `GET /admin/files`.  It
/// returns the same rich per-file information (NIP-94 tags, dimensions,
/// duration, labels, …) plus download statistics, but is scoped to the
/// calling user's files rather than the entire server.
async fn user_list_files(
    auth: BlossomAuth,
    Query(params): Query<UserListFilesQuery>,
    AxumState(state): AxumState<Arc<AppState>>,
) -> AdminResponse<PagedResult<UserFile>> {
    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
    let server_count = params.count.clamp(1, 5_000);

    match state
        .db
        .list_files_with_stats(
            &pubkey_vec,
            params.page * server_count,
            server_count,
            params.mime_type,
            params.label,
            params.sort,
            params.order,
        )
        .await
    {
        Ok((files, total)) => {
            let settings = state.settings().await;
            AdminResponse::success(PagedResult {
                count: files.len() as u32,
                page: params.page,
                total: total as u32,
                files: files
                    .into_iter()
                    .map(|(upload, stats)| UserFile {
                        inner: Nip94Event::from_upload(&settings, &upload),
                        stats,
                    })
                    .collect(),
            })
        }
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

    let ids: Vec<&[u8]> = files.iter().map(|f| f.id.as_slice()).collect();
    let stats_map = state
        .db
        .get_file_stats_batch(&ids)
        .await
        .unwrap_or_default();
    let settings = state.settings().await;
    let files_result = PagedResult {
        count: files.len() as u32,
        page,
        total: total_files as u32,
        files: files
            .into_iter()
            .map(|f| Route96File {
                stats: stats_map.get(f.id.as_slice()).cloned().unwrap_or_default(),
                inner: Nip94Event::from_upload(&settings, &f),
                uploader: vec![hex::encode(&target_pubkey)],
            })
            .collect(),
    };

    #[cfg(feature = "payments")]
    let (free_quota, total_available_quota, payments) = {
        if let Some(payment_config) = &settings.payments {
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
) -> AdminResponse<PagedResult<Route96File>> {
    let server_count = params.count.clamp(1, 5_000);

    if let Err(e) = require_admin(&auth, &state.db).await {
        return AdminResponse::error(&e);
    }

    match state
        .db
        .list_files_pending_review(params.page * server_count, server_count)
        .await
    {
        Ok((files, total)) => {
            let ids: Vec<&[u8]> = files.iter().map(|f| f.0.id.as_slice()).collect();
            let stats_map = state
                .db
                .get_file_stats_batch(&ids)
                .await
                .unwrap_or_default();
            let settings = state.settings().await;
            AdminResponse::success(PagedResult {
                count: files.len() as u32,
                page: params.page,
                total: total as u32,
                files: files
                    .into_iter()
                    .map(|f| Route96File {
                        stats: stats_map
                            .get(f.0.id.as_slice())
                            .cloned()
                            .unwrap_or_default(),
                        inner: Nip94Event::from_upload(&settings, &f.0),
                        uploader: f.1.into_iter().map(|u| hex::encode(&u.pubkey)).collect(),
                    })
                    .collect(),
            })
        }
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
                inner: Nip94Event::from_upload(&state.settings().await, &upload),
                distance: dist,
            });
        }
    }

    AdminResponse::success(results)
}

/// GET /admin/files/{file_id}/stats
///
/// Returns persisted access statistics (last access time and cumulative egress
/// bytes) for a single file.  Requires admin auth.
async fn admin_file_stats(
    auth: Nip98Auth,
    Path(file_id): Path<String>,
    AxumState(state): AxumState<Arc<AppState>>,
) -> AdminResponse<FileStats> {
    if let Err(e) = require_admin(&auth, &state.db).await {
        return AdminResponse::error(&e);
    }

    let id = match hex::decode(&file_id) {
        Ok(id) => id,
        Err(_) => return AdminResponse::error("Invalid file id"),
    };

    match state.db.get_file_stats(&id).await {
        Ok(Some(stats)) => AdminResponse::success(stats),
        Ok(None) => AdminResponse::success(FileStats {
            last_accessed: None,
            egress_bytes: 0,
        }),
        Err(e) => AdminResponse::error(&format!("DB error: {}", e)),
    }
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
        sort: FileStatSort,
        order: SortOrder,
    ) -> Result<(Vec<(FileUpload, FileStats, Vec<User>)>, i64), Error> {
        let order_sql = match order {
            SortOrder::Desc => "desc",
            SortOrder::Asc => "asc",
        };
        // For stats sorts use INNER JOIN so nulls never appear in the sort column.
        // For created (default) use LEFT JOIN so all files are returned.
        let (join_sql, sort_col) = match sort {
            FileStatSort::Created => ("left join file_stats fs on fs.file = u.id", "u.created"),
            FileStatSort::EgressBytes => (
                "inner join file_stats fs on fs.file = u.id",
                "fs.egress_bytes",
            ),
            FileStatSort::LastAccessed => (
                "inner join file_stats fs on fs.file = u.id",
                "fs.last_accessed",
            ),
        };

        let mut q = QueryBuilder::new(
            "select u.*, coalesce(fs.last_accessed, null) as last_accessed, \
             cast(coalesce(fs.egress_bytes, 0) as unsigned) as egress_bytes \
             from uploads u ",
        );
        q.push(join_sql);
        q.push(" where u.banned = false ");
        Self::build_all_files_where(&mut q, &mime_type, &label);
        q.push(format!("order by {} {} limit ", sort_col, order_sql));
        q.push_bind(limit);
        q.push(" offset ");
        q.push_bind(offset);
        let mut rows: Vec<FileUploadWithStats> = q.build_query_as().fetch_all(&self.pool).await?;

        let mut cq = QueryBuilder::new("select count(u.id) from uploads u where u.banned = false ");
        Self::build_all_files_where(&mut cq, &mime_type, &label);
        let count: i64 = cq.build().fetch_one(&self.pool).await?.try_get(0)?;

        #[cfg(feature = "labels")]
        {
            let mut uploads: Vec<FileUpload> = rows.iter().map(|r| r.upload.clone()).collect();
            self.populate_labels_vec(&mut uploads).await?;
            for (row, upload) in rows.iter_mut().zip(uploads) {
                row.upload.labels = upload.labels;
            }
        }

        let file_ids: Vec<&[u8]> = rows.iter().map(|r| r.upload.id.as_slice()).collect();
        let owners_map = self.get_file_owners_batch(&file_ids).await?;

        let res = rows
            .into_iter()
            .map(|row| {
                let owners = owners_map
                    .get(row.upload.id.as_slice())
                    .cloned()
                    .unwrap_or_default();
                (row.upload, row.stats, owners)
            })
            .collect();
        Ok((res, count))
    }

    // ── Database-backed whitelist queries ──────────────────────────────────

    /// Return all entries in the database whitelist.
    pub async fn list_whitelist_entries(&self) -> Result<Vec<WhitelistEntry>, Error> {
        self.whitelist_list().await
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

// ── Whitelist admin handlers ────────────────────────────────────────────────

/// GET /admin/whitelist — list all DB whitelist entries
async fn admin_list_whitelist(
    auth: Nip98Auth,
    AxumState(state): AxumState<Arc<AppState>>,
) -> AdminResponse<Vec<WhitelistEntry>> {
    if let Err(e) = require_admin(&auth, &state.db).await {
        return AdminResponse::error(&e);
    }

    match state.db.whitelist_list().await {
        Ok(entries) => AdminResponse::success(entries),
        Err(e) => AdminResponse::error(&format!("Failed to list whitelist: {}", e)),
    }
}

/// Request body for adding/removing a pubkey from the whitelist.
#[derive(Deserialize)]
struct WhitelistPubkeyBody {
    pubkey: String,
}

/// POST /admin/whitelist — add a pubkey to the DB whitelist
async fn admin_add_whitelist(
    auth: Nip98Auth,
    AxumState(state): AxumState<Arc<AppState>>,
    Json(body): Json<WhitelistPubkeyBody>,
) -> AdminResponse<()> {
    if let Err(e) = require_admin(&auth, &state.db).await {
        return AdminResponse::error(&e);
    }

    // Validate: must be a 64-char hex string (32-byte pubkey)
    if body.pubkey.len() != 64 || hex::decode(&body.pubkey).is_err() {
        return AdminResponse::error("Invalid pubkey: must be a 64-character hex string");
    }

    match state.db.whitelist_add(&body.pubkey).await {
        Ok(()) => AdminResponse::success(()),
        Err(e) => AdminResponse::error(&format!("Failed to add to whitelist: {}", e)),
    }
}

/// DELETE /admin/whitelist — remove a pubkey from the DB whitelist
async fn admin_remove_whitelist(
    auth: Nip98Auth,
    AxumState(state): AxumState<Arc<AppState>>,
    Json(body): Json<WhitelistPubkeyBody>,
) -> AdminResponse<()> {
    if let Err(e) = require_admin(&auth, &state.db).await {
        return AdminResponse::error(&e);
    }

    match state.db.whitelist_remove(&body.pubkey).await {
        Ok(()) => AdminResponse::success(()),
        Err(e) => AdminResponse::error(&format!("Failed to remove from whitelist: {}", e)),
    }
}

// ── Dynamic config endpoints ─────────────────────────────────────────────────

/// A single key/value config entry returned by the list endpoint.
#[derive(Serialize)]
struct ConfigEntry {
    pub key: String,
    pub value: String,
}

/// `PUT /admin/config/{key}` body.
#[derive(Deserialize)]
struct SetConfigBody {
    pub value: String,
}

/// `GET /admin/config` — list all database config overrides.
async fn admin_list_config(
    auth: Nip98Auth,
    AxumState(state): AxumState<Arc<AppState>>,
) -> AdminResponse<Vec<ConfigEntry>> {
    if let Err(e) = require_admin(&auth, &state.db).await {
        return AdminResponse::error(&e);
    }

    match state.db.config_list().await {
        Ok(entries) => AdminResponse::success(
            entries
                .into_iter()
                .map(|(key, value)| ConfigEntry { key, value })
                .collect(),
        ),
        Err(e) => AdminResponse::error(&format!("Failed to list config: {}", e)),
    }
}

/// `PUT /admin/config/{key}` — set (upsert) a single config key.
///
/// The change is persisted to the database and the in-memory settings are
/// reloaded by the background watcher within [`DB_POLL_INTERVAL`] seconds (or
/// immediately on the next file-system event).
async fn admin_set_config(
    auth: Nip98Auth,
    Path(key): Path<String>,
    AxumState(state): AxumState<Arc<AppState>>,
    Json(body): Json<SetConfigBody>,
) -> AdminResponse<()> {
    if let Err(e) = require_admin(&auth, &state.db).await {
        return AdminResponse::error(&e);
    }

    match state.db.config_set(&key, &body.value).await {
        Ok(()) => {
            state.reload_config().await;
            AdminResponse::success(())
        }
        Err(e) => AdminResponse::error(&format!("Failed to set config key '{}': {}", key, e)),
    }
}

/// `DELETE /admin/config/{key}` — remove a config override, reverting to the
/// static file value on the next reload.
async fn admin_delete_config(
    auth: Nip98Auth,
    Path(key): Path<String>,
    AxumState(state): AxumState<Arc<AppState>>,
) -> AdminResponse<()> {
    if let Err(e) = require_admin(&auth, &state.db).await {
        return AdminResponse::error(&e);
    }

    match state.db.config_delete(&key).await {
        Ok(()) => {
            state.reload_config().await;
            AdminResponse::success(())
        }
        Err(e) => AdminResponse::error(&format!("Failed to delete config key '{}': {}", key, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{FileUpload, ReviewState};
    use crate::file_stats::FileStats;
    use crate::routes::Nip94Event;
    use crate::settings::Settings;
    use chrono::DateTime;

    fn make_upload(id_byte: u8, mime: &str) -> FileUpload {
        FileUpload {
            id: vec![id_byte; 32],
            name: Some("test.png".to_string()),
            size: 1024,
            mime_type: mime.to_string(),
            created: DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            width: Some(800),
            height: Some(600),
            blur_hash: Some("LEHV6nWB2yk8pyo0adR*.7kCMdnj".to_string()),
            alt: None,
            duration: None,
            bitrate: None,
            review_state: ReviewState::None,
            banned: false,
            #[cfg(feature = "labels")]
            labels: vec![],
        }
    }

    fn make_stats(egress: u64) -> FileStats {
        FileStats {
            last_accessed: Some(DateTime::from_timestamp(1_700_001_000, 0).unwrap()),
            egress_bytes: egress,
        }
    }

    fn default_settings() -> Settings {
        Settings {
            listen: None,
            storage_dir: "/tmp".to_string(),
            database: "mysql://localhost/test".to_string(),
            max_upload_bytes: 104_857_600,
            public_url: "https://example.com".to_string(),
            whitelist: None,
            webhook_url: None,
            #[cfg(feature = "labels")]
            models_dir: None,
            #[cfg(feature = "labels")]
            label_models: None,
            #[cfg(feature = "labels")]
            label_flag_terms: None,
            #[cfg(feature = "blossom")]
            reject_sensitive_exif: None,
            #[cfg(feature = "payments")]
            payments: None,
            delete_unaccessed_days: None,
        }
    }

    /// `UserFile` wraps `Nip94Event` + `FileStats`; confirm both are set.
    #[test]
    fn user_file_contains_nip94_and_stats() {
        let upload = make_upload(1, "image/png");
        let stats = make_stats(9999);
        let settings = default_settings();

        let user_file = UserFile {
            inner: Nip94Event::from_upload(&settings, &upload),
            stats: stats.clone(),
        };

        // The NIP-94 tags must include the url and sha256 hash.
        let url_tag = user_file
            .inner
            .tags
            .iter()
            .find(|t| t.first().map(|s| s.as_str()) == Some("url"));
        assert!(url_tag.is_some(), "url tag must be present");
        let url_val = &url_tag.unwrap()[1];
        assert!(url_val.starts_with("https://example.com/"), "url must use public_url");

        let x_tag = user_file
            .inner
            .tags
            .iter()
            .find(|t| t.first().map(|s| s.as_str()) == Some("x"));
        assert!(x_tag.is_some(), "sha256 (x) tag must be present");

        // Stats should be carried through unchanged.
        assert_eq!(user_file.stats.egress_bytes, 9999);
        assert!(user_file.stats.last_accessed.is_some());
    }

    /// `UserFile` serialises to JSON without the `uploader` key.
    #[test]
    fn user_file_json_has_no_uploader_field() {
        let upload = make_upload(2, "image/jpeg");
        let settings = default_settings();

        let user_file = UserFile {
            inner: Nip94Event::from_upload(&settings, &upload),
            stats: make_stats(0),
        };

        let json = serde_json::to_value(&user_file).expect("serialisation must succeed");
        assert!(
            json.get("uploader").is_none(),
            "user file must not expose uploader"
        );
        // But it should have the stats object.
        let stats = json.get("stats").expect("stats must be present");
        assert!(
            stats.get("egress_bytes").is_some(),
            "egress_bytes must be present inside stats"
        );
    }

    /// Verify the mapping from `(FileUpload, FileStats)` to `UserFile` used in the handler.
    #[test]
    fn user_file_mapping_preserves_all_fields() {
        let settings = default_settings();
        let pairs: Vec<(FileUpload, FileStats)> = vec![
            (make_upload(3, "image/gif"), make_stats(100)),
            (make_upload(4, "video/mp4"), make_stats(200)),
        ];

        let user_files: Vec<UserFile> = pairs
            .into_iter()
            .map(|(upload, stats)| UserFile {
                inner: Nip94Event::from_upload(&settings, &upload),
                stats,
            })
            .collect();

        assert_eq!(user_files.len(), 2);
        assert_eq!(user_files[0].stats.egress_bytes, 100);
        assert_eq!(user_files[1].stats.egress_bytes, 200);
    }

    /// `UserListFilesQuery` default values: page=0, count=50, sort=created, order=desc.
    #[test]
    fn user_list_files_query_defaults() {
        let q: UserListFilesQuery =
            serde_json::from_str("{}").expect("empty object must deserialise with defaults");
        assert_eq!(q.page, 0);
        assert_eq!(q.count, 50);
        assert!(matches!(q.sort, FileStatSort::Created));
        assert!(matches!(q.order, SortOrder::Desc));
        assert!(q.mime_type.is_none());
        assert!(q.label.is_none());
    }

    /// `UserListFilesQuery` accepts all sort/order/filter variants.
    #[test]
    fn user_list_files_query_sort_and_filter_params() {
        let q: UserListFilesQuery = serde_json::from_str(
            r#"{"sort":"egress_bytes","order":"asc","mime_type":"image/","label":"nsfw"}"#,
        )
        .expect("must deserialise");
        assert!(matches!(q.sort, FileStatSort::EgressBytes));
        assert!(matches!(q.order, SortOrder::Asc));
        assert_eq!(q.mime_type.as_deref(), Some("image/"));
        assert_eq!(q.label.as_deref(), Some("nsfw"));

        let q2: UserListFilesQuery =
            serde_json::from_str(r#"{"sort":"last_accessed","order":"desc"}"#)
                .expect("must deserialise");
        assert!(matches!(q2.sort, FileStatSort::LastAccessed));
        assert!(matches!(q2.order, SortOrder::Desc));
    }
}
