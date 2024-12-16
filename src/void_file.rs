use chrono::{DateTime, Utc};
use sqlx::FromRow;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(FromRow)]
pub struct VoidFile {
    #[sqlx(rename = "Id")]
    pub id: Uuid,
    #[sqlx(rename = "Name")]
    pub name: Option<String>,
    #[sqlx(rename = "Size")]
    pub size: i64,
    #[sqlx(rename = "Uploaded")]
    pub uploaded: DateTime<Utc>,
    #[sqlx(rename = "Description")]
    pub description: Option<String>,
    #[sqlx(rename = "MimeType")]
    pub mime_type: String,
    #[sqlx(rename = "Digest")]
    pub digest: String,
    #[sqlx(rename = "MediaDimensions")]
    pub media_dimensions: Option<String>,
    #[sqlx(rename = "Email")]
    pub email: String,
}

impl VoidFile {
    pub fn map_to_path(id: &Uuid) -> PathBuf {
        let id_str = id.as_hyphenated().to_string();
        PathBuf::new()
            .join("files-v2/")
            .join(&id_str[..2])
            .join(&id_str[2..4])
            .join(&id_str)
    }
}
