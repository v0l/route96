use crate::auth::nip98::Nip98Auth;
use crate::db::{Database, FileUpload};
use crate::routes::{Nip94Event, PagedResult};
use crate::settings::Settings;
use rocket::serde::json::Json;
use rocket::serde::Serialize;
use rocket::{routes, Responder, Route, State};
use sqlx::{Error, Row};

pub fn admin_routes() -> Vec<Route> {
    routes![admin_list_files, admin_get_self]
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
}

#[rocket::get("/self")]
async fn admin_get_self(auth: Nip98Auth, db: &State<Database>) -> AdminResponse<SelfUser> {
    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
    match db.get_user(&pubkey_vec).await {
        Ok(user) => {
            let s = match db.get_user_stats(user.id).await {
                Ok(r) => r,
                Err(e) => {
                    return AdminResponse::error(&format!("Failed to load user stats: {}", e))
                }
            };
            AdminResponse::success(SelfUser {
                is_admin: user.is_admin,
                file_count: s.file_count,
                total_size: s.total_size,
            })
        }
        Err(_) => AdminResponse::error("User not found"),
    }
}

#[rocket::get("/files?<page>&<count>")]
async fn admin_list_files(
    auth: Nip98Auth,
    page: u32,
    count: u32,
    db: &State<Database>,
    settings: &State<Settings>,
) -> AdminResponse<PagedResult<Nip94Event>> {
    let pubkey_vec = auth.event.pubkey.to_bytes().to_vec();
    let server_count = count.clamp(1, 5_000);

    let user = match db.get_user(&pubkey_vec).await {
        Ok(user) => user,
        Err(_) => return AdminResponse::error("User not found"),
    };

    if !user.is_admin {
        return AdminResponse::error("User is not an admin");
    }
    match db.list_all_files(page * server_count, server_count).await {
        Ok((files, count)) => AdminResponse::success(PagedResult {
            count: files.len() as u32,
            page,
            total: count as u32,
            files: files
                .iter()
                .map(|f| Nip94Event::from_upload(settings, f))
                .collect(),
        }),
        Err(e) => AdminResponse::error(&format!("Could not list files: {}", e)),
    }
}

impl Database {
    pub async fn list_all_files(
        &self,
        offset: u32,
        limit: u32,
    ) -> Result<(Vec<FileUpload>, i64), Error> {
        let results: Vec<FileUpload> = sqlx::query_as(
            "select u.* \
            from uploads u \
            order by u.created desc \
            limit ? offset ?",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        let count: i64 = sqlx::query("select count(u.id) from uploads u")
            .fetch_one(&self.pool)
            .await?
            .try_get(0)?;
        Ok((results, count))
    }
}
