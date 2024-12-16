use crate::void_file::VoidFile;
use sqlx_postgres::{PgPool, PgPoolOptions};
use uuid::Uuid;

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
