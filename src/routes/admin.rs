use crate::auth::nip98::Nip98Auth;
use crate::db::Database;
use crate::routes::{Nip94Event, PagedResult};
use crate::settings::Settings;
use rocket::response::Responder;
use rocket::serde::json::Json;
use rocket::serde::Serialize;
use rocket::{routes, Route, State};

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

#[derive(Serialize)]
pub struct AdminNip94File {
    #[serde(flatten)]
    pub inner: Nip94Event,
    pub uploader: Vec<String>,
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
                file_count: s.file_count as u64,
                total_size: s.total_size as u64,
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
        .list_all_files_with_owners((page * server_count) as i32, server_count as i32, mime_type)
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
