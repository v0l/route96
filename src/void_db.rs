use chrono::{DateTime, Utc};
use sqlx::FromRow;
use sqlx_postgres::{PgPool, PgPoolOptions};
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
    pub fn map_to_path(&self) -> PathBuf {
        let id_str = self.id.as_hyphenated().to_string();
        PathBuf::new()
            .join("files-v2/")
            .join(&id_str[..2])
            .join(&id_str[2..4])
            .join(&id_str)
    }
}

pub struct VoidCatDb {
    pub pool: PgPool,
}

impl VoidCatDb {
    pub async fn connect(conn: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(50)
            .connect(conn)
            .await?;
        Ok(Self { pool })
    }

    pub async fn list_files(&self, page: usize) -> Result<Vec<VoidFile>, sqlx::Error> {
        let page_size = 100;
        sqlx::query_as(format!("select f.\"Id\", f.\"Name\", CAST(f.\"Size\" as BIGINT) \"Size\", f.\"Uploaded\", f.\"Description\", f.\"MimeType\", f.\"Digest\", f.\"MediaDimensions\", u.\"Email\"
from \"Files\" f, \"UserFiles\" uf, \"Users\" u
where f.\"Id\" = uf.\"FileId\"
and uf.\"UserId\" = u.\"Id\"
and u.\"AuthType\" = 4\
offset {} limit {}", page * page_size, page_size).as_str())
            .fetch_all(&self.pool)
            .await
    }

    pub async fn get_digest(&self, file_id: &Uuid) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar("select f.\"Digest\" from \"Files\" f where f.\"Id\" = $1")
            .bind(file_id)
            .fetch_optional(&self.pool)
            .await
    }
}
