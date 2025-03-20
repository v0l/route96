use crate::filesystem::NewFileResult;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::migrate::MigrateError;
use sqlx::{Error, Executor, FromRow, Row};

#[derive(Clone, FromRow, Default, Serialize)]
pub struct FileUpload {
    /// SHA-256 hash of the file
    #[serde(with = "hex")]
    pub id: Vec<u8>,
    /// Filename
    pub name: Option<String>,
    /// Size in bytes
    pub size: u64,
    /// MIME type
    pub mime_type: String,
    /// When the upload was created
    pub created: DateTime<Utc>,
    /// Width of the media in pixels
    pub width: Option<u32>,
    /// Height of the media in pixels
    pub height: Option<u32>,
    /// Blurhash of the media
    pub blur_hash: Option<String>,
    /// Alt text of the media
    pub alt: Option<String>,
    /// Duration of media in seconds
    pub duration: Option<f32>,
    /// Average bitrate in bits/s
    pub bitrate: Option<u32>,
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
            name: None,
            size: value.size,
            mime_type: value.mime_type.clone(),
            created: Utc::now(),
            width: value.width,
            height: value.height,
            blur_hash: value.blur_hash.clone(),
            alt: None,
            duration: value.duration,
            bitrate: value.bitrate,
            h_tag: None,
            #[cfg(feature = "labels")]
            labels: value.labels.clone(),
        }
    }
}
#[derive(Clone, FromRow, Serialize)]
pub struct User {
    pub id: u64,
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
    pub file_count: u64,
    pub total_size: u64,
}

#[derive(Clone)]
pub struct Database {
    pub(crate) pool: sqlx::pool::Pool<sqlx::mysql::MySql>,
}

impl Database {
    pub async fn new(conn: &str) -> Result<Self, Error> {
        let db = sqlx::mysql::MySqlPool::connect(conn).await?;
        Ok(Self { pool: db })
    }

    pub async fn migrate(&self) -> Result<(), MigrateError> {
        sqlx::migrate!("./migrations/").run(&self.pool).await
    }

    pub async fn upsert_user(&self, pubkey: &Vec<u8>) -> Result<u64, Error> {
        let res = sqlx::query("insert ignore into users(pubkey) values(?) returning id")
            .bind(pubkey)
            .fetch_optional(&self.pool)
            .await?;
        match res {
            None => sqlx::query("select id from users where pubkey = ?")
                .bind(pubkey)
                .fetch_one(&self.pool)
                .await?
                .try_get(0),
            Some(res) => res.try_get(0),
        }
    }

    pub async fn get_user(&self, pubkey: &Vec<u8>) -> Result<User, Error> {
        sqlx::query_as("select * from users where pubkey = ?")
            .bind(pubkey)
            .fetch_one(&self.pool)
            .await
    }

    pub async fn get_user_stats(&self, id: u64) -> Result<UserStats, Error> {
        sqlx::query_as(
            "select cast(count(user_uploads.file) as unsigned integer) as file_count, \
        cast(sum(uploads.size) as unsigned integer) as total_size \
        from user_uploads,uploads \
        where user_uploads.user_id = ? \
        and user_uploads.file = uploads.id",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
    }

    pub async fn get_user_id(&self, pubkey: &Vec<u8>) -> Result<u64, Error> {
        sqlx::query("select id from users where pubkey = ?")
            .bind(pubkey)
            .fetch_one(&self.pool)
            .await?
            .try_get(0)
    }

    pub async fn add_file(&self, file: &FileUpload, user_id: Option<u64>) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;
        let q = sqlx::query("insert ignore into \
        uploads(id,name,size,mime_type,blur_hash,width,height,alt,created,duration,bitrate,h_tag) values(?,?,?,?,?,?,?,?,?,?,?,?)")
            .bind(&file.id)
            .bind(&file.name)
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
            let q2 = sqlx::query("insert ignore into user_uploads(file,user_id) values(?,?)")
                .bind(&file.id)
                .bind(user_id);

            tx.execute(q2).await?;
        }

        #[cfg(feature = "labels")]
        for lbl in &file.labels {
            let q3 =
                sqlx::query("insert ignore into upload_labels(file,label,model) values(?,?,?)")
                    .bind(&file.id)
                    .bind(&lbl.label)
                    .bind(&lbl.model);
            tx.execute(q3).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn get_file(&self, file: &Vec<u8>) -> Result<Option<FileUpload>, Error> {
        sqlx::query_as("select * from uploads where id = ?")
            .bind(file)
            .fetch_optional(&self.pool)
            .await
    }

    pub async fn get_file_owners(&self, file: &Vec<u8>) -> Result<Vec<User>, Error> {
        sqlx::query_as(
            "select users.* from users, user_uploads \
        where users.id = user_uploads.user_id \
        and user_uploads.file = ?",
        )
        .bind(file)
        .fetch_all(&self.pool)
        .await
    }

    #[cfg(feature = "labels")]
    pub async fn get_file_labels(&self, file: &Vec<u8>) -> Result<Vec<FileLabel>, Error> {
        sqlx::query_as(
            "select upload_labels.* from uploads, upload_labels \
        where uploads.id = ? and uploads.id = upload_labels.file",
        )
        .bind(file)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn delete_file_owner(&self, file: &Vec<u8>, owner: u64) -> Result<(), Error> {
        sqlx::query("delete from user_uploads where file = ? and user_id = ?")
            .bind(file)
            .bind(owner)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_all_file_owner(&self, file: &Vec<u8>) -> Result<(), Error> {
        sqlx::query("delete from user_uploads where file = ?")
            .bind(file)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_file(&self, file: &Vec<u8>) -> Result<(), Error> {
        sqlx::query("delete from uploads where id = ?")
            .bind(file)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_files(
        &self,
        pubkey: &Vec<u8>,
        offset: u32,
        limit: u32,
    ) -> Result<(Vec<FileUpload>, i64), Error> {
        let results: Vec<FileUpload> = sqlx::query_as(
            "select uploads.* from uploads, users, user_uploads \
            where users.pubkey = ? \
            and users.id = user_uploads.user_id \
            and user_uploads.file = uploads.id \
            order by uploads.created desc \
            limit ? offset ?",
        )
        .bind(pubkey)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        let count: i64 = sqlx::query(
            "select count(uploads.id) from uploads, users, user_uploads \
            where users.pubkey = ? \
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
        sqlx::query_scalar("select h_tag from uploads where id = ?")
            .bind(file)
            .fetch_optional(&self.pool)
            .await
    }
}
