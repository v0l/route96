use crate::filesystem::NewFileResult;
use chrono::{DateTime, Utc};
use hex;
use serde::Serialize;
use sqlx::migrate::MigrateError;
use sqlx::{Error, Executor, FromRow, Row};

#[derive(Clone, FromRow, Default, Serialize)]
pub struct FileUpload {
    /// SHA-256 hash of the file
    #[serde(with = "hex")]
    pub id: Vec<u8>,
    /// Size in bytes
    pub size: i64,
    /// MIME type
    pub mime_type: String,
    /// When the upload was created
    pub created: DateTime<Utc>,
    /// Width of the media in pixels
    pub width: Option<i32>,
    /// Height of the media in pixels
    pub height: Option<i32>,
    /// Blurhash of the media
    pub blur_hash: Option<String>,
    /// Alt text of the media
    pub alt: Option<String>,
    /// Duration of media in seconds
    pub duration: Option<f32>,
    /// Average bitrate in bits/s
    pub bitrate: Option<i32>,
    /// NIP-29 group ID (h tag)
    pub h_tag: Option<String>,

    #[sqlx(skip)]
    #[cfg(feature = "labels")]
    pub labels: Vec<FileLabel>,
}

impl From<&NewFileResult> for FileUpload {
    fn from(value: &NewFileResult) -> Self {
        Self {
            id: value.id.clone(),
            size: value.size as i64,
            mime_type: value.mime_type.clone(),
            created: Utc::now(),
            width: value.width.map(|w| w as i32),
            height: value.height.map(|h| h as i32),
            blur_hash: value.blur_hash.clone(),
            alt: None,
            duration: value.duration,
            bitrate: value.bitrate.map(|b| b as i32),
            h_tag: None,
            #[cfg(feature = "labels")]
            labels: value.labels.clone(),
        }
    }
}

#[derive(Clone, FromRow, Serialize)]
pub struct User {
    pub id: i64,
    #[serde(with = "hex")]
    pub pubkey: Vec<u8>,
    pub created: DateTime<Utc>,
    pub is_admin: bool,
}

#[cfg(feature = "labels")]
#[derive(Clone, FromRow, Serialize)]
pub struct FileLabel {
    pub file: Vec<u8>,
    pub label: String,
    pub created: DateTime<Utc>,
    pub model: String,
}

#[cfg(feature = "labels")]
impl FileLabel {
    pub fn new(label: String, model: String) -> Self {
        Self {
            file: vec![],
            label,
            created: Utc::now(),
            model,
        }
    }
}

#[derive(Clone, FromRow, Serialize)]
pub struct UserStats {
    pub file_count: i64,
    pub total_size: i64,
}

#[derive(Clone)]
pub struct Database {
    pub pool: sqlx::pool::Pool<sqlx::postgres::Postgres>,
}

impl Database {
    pub async fn new(conn: &str) -> Result<Self, Error> {
        let pool = sqlx::postgres::PgPool::connect(conn).await?;
        Ok(Self { pool })
    }

    pub async fn migrate(&self) -> Result<(), MigrateError> {
        sqlx::migrate!("./migrations/").run(&self.pool).await
    }

    pub async fn upsert_user(&self, pubkey: &Vec<u8>) -> Result<i64, Error> {
        let res = sqlx::query(
            "insert into users(pubkey, is_admin) values($1, DEFAULT) on conflict (pubkey) do nothing returning id",
        )
        .bind(pubkey)
        .fetch_optional(&self.pool)
        .await?;
        match res {
            None => sqlx::query("select id from users where pubkey = $1")
                .bind(pubkey)
                .fetch_one(&self.pool)
                .await?
                .try_get(0),
            Some(res) => res.try_get(0),
        }
    }

    pub async fn get_user(&self, pubkey: &Vec<u8>) -> Result<User, Error> {
        sqlx::query_as("select * from users where pubkey = $1")
            .bind(pubkey)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn get_user_stats(&self, id: i64) -> Result<UserStats, Error> {
        sqlx::query_as(
            "select cast(count(user_uploads.file) as bigint) as file_count, \
        cast(sum(uploads.size) as bigint) as total_size \
        from user_uploads,uploads \
        where user_uploads.user_id = $1 \
        and user_uploads.file = uploads.id",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
    }

    pub async fn get_user_id(&self, pubkey: &Vec<u8>) -> Result<i64, Error> {
        sqlx::query("select id from users where pubkey = $1")
            .bind(pubkey)
            .fetch_one(&self.pool)
            .await?
            .try_get(0)
    }

    pub async fn add_file(&self, file: &FileUpload, user_id: Option<i64>) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;
        let q = sqlx::query("insert into uploads(id,size,mime_type,blur_hash,width,height,alt,created,duration,bitrate,h_tag) values($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) on conflict (id) do nothing")
            .bind(&file.id)
            .bind(file.size)
            .bind(&file.mime_type)
            .bind(&file.blur_hash)
            .bind(file.width)
            .bind(file.height)
            .bind(&file.alt)
            .bind(file.created)
            .bind(file.duration)
            .bind(file.bitrate)
            .bind(&file.h_tag);

        tx.execute(q).await?;

        if let Some(user_id) = user_id {
            let q2 = sqlx::query("insert into user_uploads(file,user_id) values($1,$2) on conflict (file, user_id) do nothing")
                .bind(&file.id)
                .bind(user_id);

            tx.execute(q2).await?;
        }

        #[cfg(feature = "labels")]
        for lbl in &file.labels {
            let q3 =
                sqlx::query("insert into upload_labels(file,label,model) values($1,$2,$3) on conflict (file, label) do nothing")
                    .bind(&file.id)
                    .bind(&lbl.label)
                    .bind(&lbl.model);
            tx.execute(q3).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_file(&self, file: &Vec<u8>) -> Result<Option<FileUpload>, Error> {
        sqlx::query_as("select * from uploads where id = $1")
            .bind(file)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn get_file_owners(&self, file: &Vec<u8>) -> Result<Vec<User>, Error> {
        sqlx::query_as(
            "select users.* from users, user_uploads \
        where users.id = user_uploads.user_id \
        and user_uploads.file = $1",
        )
        .bind(file)
        .fetch_all(&self.pool)
        .await
    }

    #[cfg(feature = "labels")]
    pub async fn get_file_labels(&self, file: &Vec<u8>) -> Result<Vec<FileLabel>, Error> {
        sqlx::query_as(
            "select upload_labels.* from uploads, upload_labels \
        where uploads.id = $1 and uploads.id = upload_labels.file",
        )
        .bind(file)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn delete_file_owner(&self, file: &Vec<u8>, owner: i64) -> Result<(), Error> {
        sqlx::query("delete from user_uploads where file = $1 and user_id = $2")
            .bind(file)
            .bind(owner)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_all_file_owner(&self, file: &Vec<u8>) -> Result<(), Error> {
        sqlx::query("delete from user_uploads where file = $1")
            .bind(file)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_file(&self, file: &Vec<u8>) -> Result<(), Error> {
        sqlx::query("delete from uploads where id = $1")
            .bind(file)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_files(
        &self,
        pubkey: &Vec<u8>,
        offset: i32,
        limit: i32,
    ) -> Result<(Vec<FileUpload>, i64), Error> {
        let results: Vec<FileUpload> = sqlx::query_as(
            "select uploads.* from uploads, users, user_uploads \
            where users.pubkey = $1 \
            and users.id = user_uploads.user_id \
            and user_uploads.file = uploads.id \
            order by uploads.created desc \
            limit $2 offset $3",
        )
        .bind(pubkey)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        let count: i64 = sqlx::query(
            "select count(uploads.id) from uploads, users, user_uploads \
            where users.pubkey = $1 \
            and users.id = user_uploads.user_id \
            and user_uploads.file = uploads.id",
        )
        .bind(pubkey)
        .fetch_one(&self.pool)
        .await?
        .try_get(0)?;

        Ok((results, count))
    }

    pub async fn get_file_h_tag(&self, file: &Vec<u8>) -> Result<Option<String>, Error> {
        sqlx::query_scalar("select h_tag from uploads where id = $1")
            .bind(file)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn list_all_files_with_owners(
        &self,
        offset: i32,
        limit: i32,
        mime_type: Option<String>,
    ) -> Result<(Vec<(FileUpload, Vec<User>)>, i64), Error> {
        let mut q = sqlx::QueryBuilder::new("select u.* from uploads u ");
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

    pub async fn list_all_files(&self, offset: i32, limit: i32) -> Result<Vec<FileUpload>, Error> {
        let mut q = sqlx::QueryBuilder::new(
            "select uploads.* from uploads order by uploads.created desc limit ",
        );
        q.push_bind(limit);
        q.push(" offset ");
        q.push_bind(offset);

        let results: Vec<FileUpload> = q.build_query_as().fetch_all(&self.pool).await?;
        Ok(results)
    }

    pub async fn list_files_by_mimetype(
        &self,
        offset: i32,
        limit: i32,
    ) -> Result<Vec<FileUpload>, Error> {
        let mut q = sqlx::QueryBuilder::new(
            "select uploads.* from uploads order by uploads.created desc limit ",
        );
        q.push_bind(limit);
        q.push(" offset ");
        q.push_bind(offset);

        let results: Vec<FileUpload> = q.build_query_as().fetch_all(&self.pool).await?;
        Ok(results)
    }
}
