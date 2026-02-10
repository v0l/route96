use crate::auth::nip98::Nip98Auth;
use crate::db::{Database, FileUpload, Report, User};
use crate::routes::{AppState, Nip94Event, PagedResult};
use axum::{
    extract::{Path, Query, State as AxumState},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::{Error, QueryBuilder, Row};
use std::sync::Arc;

pub fn admin_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/self", get(admin_get_self))
        .route("/files", get(admin_list_files))
        .route("/reports", get(admin_list_reports))
        .route("/reports/:report_id", delete(admin_acknowledge_report))
        .route("/user/:user_pubkey", get(admin_get_user_info))
        .route("/user/:user_pubkey/purge", delete(admin_purge_user))
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
            AdminResponse::GenericError(json) => (StatusCode::INTERNAL_SERVER_ERROR, json).into_response(),
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
                    return AdminResponse::error(&format!("Failed to load user stats: {}", e))
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
}

fn default_count() -> u32 {
    50
}

async fn admin_list_files(
    auth: Nip98Auth,
    Query(params): Query<AdminListFilesQuery>,
    AxumState(state): AxumState<Arc<AppState>>,
) -> AdminResponse<PagedResult<AdminNip94File>> {
    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
    let server_count = params.count.clamp(1, 5_000);

    let user = match state.db.get_user(&pubkey_vec).await {
        Ok(user) => user,
        Err(_) => return AdminResponse::error("User not found"),
    };

    if !user.is_admin {
        return AdminResponse::error("User is not an admin");
    }
    match state.db
        .list_all_files(params.page * server_count, server_count, params.mime_type)
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
    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
    let server_count = params.count.clamp(1, 5_000);

    let user = match state.db.get_user(&pubkey_vec).await {
        Ok(user) => user,
        Err(_) => return AdminResponse::error("User not found"),
    };

    if !user.is_admin {
        return AdminResponse::error("User is not an admin");
    }

    match state.db.list_reports(params.page * server_count, server_count).await {
        Ok((reports, total_count)) => AdminResponse::success(PagedResult {
            count: reports.len() as u32,
            page: params.page,
            total: total_count as u32,
            files: reports,
        }),
        Err(e) => AdminResponse::error(&format!("Could not list reports: {}", e)),
    }
}

async fn admin_acknowledge_report(
    auth: Nip98Auth,
    Path(report_id): Path<u64>,
    AxumState(state): AxumState<Arc<AppState>>,
) -> AdminResponse<()> {
    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();

    let user = match state.db.get_user(&pubkey_vec).await {
        Ok(user) => user,
        Err(_) => return AdminResponse::error("User not found"),
    };

    if !user.is_admin {
        return AdminResponse::error("User is not an admin");
    }

    match state.db.mark_report_reviewed(report_id).await {
        Ok(()) => AdminResponse::success(()),
        Err(e) => AdminResponse::error(&format!("Could not acknowledge report: {}", e)),
    }
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
    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();

    // Check if the requesting user is an admin
    let admin_user = match state.db.get_user(&pubkey_vec).await {
        Ok(user) => user,
        Err(_) => return AdminResponse::error("User not found"),
    };

    if !admin_user.is_admin {
        return AdminResponse::error("User is not an admin");
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
    let (files, total_files) = match state.db.list_files(&target_pubkey, page * count, count).await {
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

            let payments = state.db
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
    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();

    // Check if the requesting user is an admin
    let admin_user = match state.db.get_user(&pubkey_vec).await {
        Ok(user) => user,
        Err(_) => return AdminResponse::error("User not found"),
    };

    if !admin_user.is_admin {
        return AdminResponse::error("User is not an admin");
    }

    // Parse target user pubkey
    let target_pubkey = match hex::decode(&user_pubkey) {
        Ok(pk) => pk,
        Err(_) => return AdminResponse::error("Invalid pubkey format"),
    };

    // Get all file IDs for the target user
    let file_ids = match state.db.get_user_file_ids(&target_pubkey).await {
        Ok(ids) => ids,
        Err(e) => return AdminResponse::error(&format!("Failed to get user files: {}", e)),
    };

    let mut deleted_count = 0;
    let mut failed_count = 0;

    // Delete each file
    for file_id in file_ids {
        // Delete file ownership records
        if let Err(e) = state.db.delete_all_file_owner(&file_id).await {
            log::warn!(
                "Failed to delete file ownership for file {}: {}",
                hex::encode(&file_id),
                e
            );
            failed_count += 1;
            continue;
        }

        // Delete file record from database
        if let Err(e) = state.db.delete_file(&file_id).await {
            log::warn!(
                "Failed to delete file record for file {}: {}",
                hex::encode(&file_id),
                e
            );
            failed_count += 1;
            continue;
        }

        // Delete physical file
        if let Err(e) = tokio::fs::remove_file(state.fs.get(&file_id)).await {
            log::warn!(
                "Failed to delete physical file {}: {}",
                hex::encode(&file_id),
                e
            );
            // Don't increment failed_count here as the DB record is already deleted
        }

        deleted_count += 1;
    }

    if failed_count > 0 {
        AdminResponse::error(&format!(
            "Partially completed: {} files deleted, {} failed",
            deleted_count, failed_count
        ))
    } else {
        AdminResponse::success(())
    }
}

impl Database {
    pub async fn list_all_files(
        &self,
        offset: u32,
        limit: u32,
        mime_type: Option<String>,
    ) -> Result<(Vec<(FileUpload, Vec<User>)>, i64), Error> {
        let mut q = QueryBuilder::new("select u.* from uploads u ");
        if let Some(m) = mime_type {
            q.push("where INSTR(u.mime_type,");
            q.push_bind(m);
            q.push(") > 0");
        }
        q.push(" order by u.created desc limit ");
        q.push_bind(limit);
        q.push(" offset ");
        q.push_bind(offset);

        let results: Vec<FileUpload> = q.build_query_as().fetch_all(&self.pool).await?;
        let count: i64 = sqlx::query("select count(u.id) from uploads u")
            .fetch_one(&self.pool)
            .await?
            .try_get(0)?;

        let mut res = Vec::with_capacity(results.len());
        for upload in results.into_iter() {
            let upd = self.get_file_owners(&upload.id).await?;
            res.push((upload, upd));
        }
        Ok((res, count))
    }
}
