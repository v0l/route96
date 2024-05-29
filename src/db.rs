use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{Error, Executor, FromRow, Row};
use sqlx::migrate::MigrateError;

#[derive(Clone, FromRow, Default, Serialize)]
pub struct FileUpload {
    pub id: Vec<u8>,
    pub name: String,
    pub size: u64,
    pub mime_type: String,
    pub created: DateTime<Utc>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub blur_hash: Option<String>,

    #[sqlx(skip)]
    pub labels: Vec<FileLabel>,
}

#[derive(Clone, FromRow)]
pub struct User {
    pub id: u64,
    pub pubkey: Vec<u8>,
    pub created: DateTime<Utc>,
}

#[derive(Clone, FromRow, Serialize)]
pub struct FileLabel {
    pub file: Vec<u8>,
    pub label: String,
    pub created: DateTime<Utc>,
    pub model: String,
}

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

#[derive(Clone)]
pub struct Database {
    pool: sqlx::pool::Pool<sqlx::mysql::MySql>,
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

    pub async fn get_user_id(&self, pubkey: &Vec<u8>) -> Result<u64, Error> {
        sqlx::query("select id from users where pubkey = ?")
            .bind(pubkey)
            .fetch_one(&self.pool)
            .await?
            .try_get(0)
    }

    pub async fn add_file(&self, file: &FileUpload, user_id: u64) -> Result<(), Error> {
        let mut tx = self.pool.begin().await?;
        let q = sqlx::query("insert ignore into uploads(id,name,size,mime_type,blur_hash,width,height) values(?,?,?,?,?,?,?)")
            .bind(&file.id)
            .bind(&file.name)
            .bind(file.size)
            .bind(&file.mime_type)
            .bind(&file.blur_hash)
            .bind(file.width)
            .bind(file.height);
        tx.execute(q).await?;

        let q2 = sqlx::query("insert into user_uploads(file,user_id) values(?,?)")
            .bind(&file.id)
            .bind(user_id);
        tx.execute(q2).await?;

        for lbl in &file.labels {
            let q3 = sqlx::query("insert into upload_labels(file,label,model) values(?,?,?)")
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
        sqlx::query_as("select users.* from users, user_uploads where user.id = user_uploads.user_id and user_uploads.file = ?")
            .bind(file)
            .fetch_all(&self.pool)
            .await
    }

    pub async fn get_file_labels(&self, file: &Vec<u8>) -> Result<Vec<FileLabel>, Error> {
        sqlx::query_as("select upload_labels.* from uploads, upload_labels where uploads.id = ? and uploads.id = upload_labels.file")
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

    pub async fn delete_file(&self, file: &Vec<u8>) -> Result<(), Error> {
        sqlx::query("delete from uploads where id = ?")
            .bind(file)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_files(&self, pubkey: &Vec<u8>, offset: u32, limit: u32) -> Result<(Vec<FileUpload>, i64), Error> {
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
            and user_uploads.file = uploads.id \
            order by uploads.created desc")
            .bind(pubkey)
            .fetch_one(&self.pool)
            .await?
            .try_get(0)?;

        Ok((results, count))
    }
}
