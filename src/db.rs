use sqlx::migrate::MigrateError;
use sqlx::{Error, Row};

#[derive(Clone, sqlx::FromRow)]
pub struct FileUpload {
    pub id: Vec<u8>,
    pub user_id: u64,
    pub name: String,
    pub size: u64,
    pub created: u64,
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

    pub async fn add_user(&self, pubkey: Vec<u8>) -> Result<u32, Error> {
        let res = sqlx::query("insert into users(pubkey) values(?) returning id")
            .bind(pubkey)
            .fetch_one(&self.pool)
            .await?;
        res.try_get(0)
    }

    pub async fn add_file(&self, file: FileUpload) -> Result<(), Error> {
        sqlx::query("insert into uploads(id,user_id,name,size) values(?,?,?,?)")
            .bind(&file.id)
            .bind(&file.user_id)
            .bind(&file.name)
            .bind(&file.size)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_files(&self, pubkey: &Vec<u8>) -> Result<Vec<FileUpload>, Error> {
        let results: Vec<FileUpload> = sqlx::query_as("select * from uploads where user_id = ?")
            .bind(&pubkey)
            .fetch_all(&self.pool)
            .await?;
        Ok(results)
    }
}
