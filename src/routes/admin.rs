use crate::auth::nip98::Nip98Auth;
use crate::db::{Database, FileUpload, User, Report};
use crate::routes::{Nip94Event, PagedResult};
use crate::settings::Settings;
use rocket::serde::json::Json;
use rocket::serde::Serialize;
use rocket::{routes, Responder, Route, State};
use sqlx::{Error, QueryBuilder, Row};

pub fn admin_routes() -> Vec<Route> {
    routes![admin_list_files, admin_get_self, admin_list_reports, admin_acknowledge_report]
}

#[derive(Serialize, Default)]
#[serde(crate = "rocket::serde")]
struct AdminResponseBase<T> {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

#[derive(Responder)]
enum AdminResponse<T> {
    #[response(status = 500)]
    GenericError(Json<AdminResponseBase<T>>),

    #[response(status = 200)]
    Ok(Json<AdminResponseBase<T>>),
}

impl<T> AdminResponse<T> {
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

#[rocket::get("/self")]
async fn admin_get_self(auth: Nip98Auth, db: &State<Database>, settings: &State<Settings>) -> AdminResponse<SelfUser> {
    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
    match db.get_user(&pubkey_vec).await {
        Ok(user) => {
            let s = match db.get_user_stats(user.id).await {
                Ok(r) => r,
                Err(e) => {
                    return AdminResponse::error(&format!("Failed to load user stats: {}", e))
                }
            };

            #[cfg(feature = "payments")]
            let (free_quota, total_available_quota) = {
                let free_quota = settings.payments.as_ref()
                    .and_then(|p| p.free_quota_bytes)
                    .unwrap_or(104857600);
                let mut total_available = free_quota;
                
                // Add paid quota if still valid
                if let Some(paid_until) = &user.paid_until {
                    if *paid_until > chrono::Utc::now() {
                        total_available += user.paid_size;
                    }
                }
                
                (free_quota, total_available)
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

#[rocket::get("/files?<page>&<count>&<mime_type>")]
async fn admin_list_files(
    auth: Nip98Auth,
    page: u32,
    count: u32,
    mime_type: Option<String>,
    db: &State<Database>,
    settings: &State<Settings>,
) -> AdminResponse<PagedResult<AdminNip94File>> {
    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
    let server_count = count.clamp(1, 5_000);

    let user = match db.get_user(&pubkey_vec).await {
        Ok(user) => user,
        Err(_) => return AdminResponse::error("User not found"),
    };

    if !user.is_admin {
        return AdminResponse::error("User is not an admin");
    }
    match db
        .list_all_files(page * server_count, server_count, mime_type)
        .await
    {
        Ok((files, count)) => AdminResponse::success(PagedResult {
            count: files.len() as u32,
            page,
            total: count as u32,
            files: files
                .into_iter()
                .map(|f| AdminNip94File {
                    inner: Nip94Event::from_upload(settings, &f.0),
                    uploader: f.1.into_iter().map(|u| hex::encode(&u.pubkey)).collect(),
                })
                .collect(),
        }),
        Err(e) => AdminResponse::error(&format!("Could not list files: {}", e)),
    }
}

#[rocket::get("/reports?<page>&<count>")]
async fn admin_list_reports(
    auth: Nip98Auth,
    page: u32,
    count: u32,
    db: &State<Database>,
) -> AdminResponse<PagedResult<Report>> {
    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
    let server_count = count.clamp(1, 5_000);

    let user = match db.get_user(&pubkey_vec).await {
        Ok(user) => user,
        Err(_) => return AdminResponse::error("User not found"),
    };

    if !user.is_admin {
        return AdminResponse::error("User is not an admin");
    }

    match db.list_reports(page * server_count, server_count).await {
        Ok((reports, total_count)) => AdminResponse::success(PagedResult {
            count: reports.len() as u32,
            page,
            total: total_count as u32,
            files: reports,
        }),
        Err(e) => AdminResponse::error(&format!("Could not list reports: {}", e)),
    }
}

#[rocket::delete("/reports/<report_id>")]
async fn admin_acknowledge_report(
    auth: Nip98Auth,
    report_id: u64,
    db: &State<Database>,
) -> AdminResponse<()> {
    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();

    let user = match db.get_user(&pubkey_vec).await {
        Ok(user) => user,
        Err(_) => return AdminResponse::error("User not found"),
    };

    if !user.is_admin {
        return AdminResponse::error("User is not an admin");
    }

    match db.delete_report(report_id).await {
        Ok(()) => AdminResponse::success(()),
        Err(e) => AdminResponse::error(&format!("Could not acknowledge report: {}", e)),
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
            q.push("where u.mime_type = ");
            q.push_bind(m);
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
