use anyhow::Error;
use chrono::{DateTime, Utc};
use clap::Parser;
use config::Config;
use log::{info, warn};
use route96::db::{Database, FileUpload};
use route96::filesystem::FileStore;
use route96::settings::Settings;
use sqlx::FromRow;
use sqlx_postgres::{PgPool, Postgres};
use std::path::PathBuf;
use tokio::fs::File;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Database connection string for void.cat DB
    #[arg(long)]
    pub database: String,

    /// Path to filestore on void.cat
    #[arg(long)]
    pub data_path: String,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    pretty_env_logger::init();

    let builder = Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("APP"))
        .build()?;

    let settings: Settings = builder.try_deserialize()?;

    let db = Database::new(&settings.database).await?;
    let fs = FileStore::new(settings.clone());

    let args: Args = Args::parse();

    let db_void = VoidCatDb::connect(&args.database).await?;

    let mut page = 0;
    loop {
        let files = db_void.list_files(page).await?;
        if files.len() == 0 {
            break;
        }
        for f in files {
            if let Err(e) = migrate_file(&f, &db, &fs, &args).await {
                warn!("Failed to migrate file: {}, {}", &f.id, e);
            }
        }
        page += 1;
    }
    Ok(())
}

async fn migrate_file(
    f: &VoidFile,
    db: &Database,
    fs: &FileStore,
    args: &Args,
) -> Result<(), Error> {
    let pubkey_vec = hex::decode(&f.email)?;
    let id_vec = hex::decode(&f.digest)?;

    // copy file
    let src_path = PathBuf::new().join(&args.data_path).join(f.map_to_path());
    let dst_path = fs.map_path(&id_vec);
    if src_path.exists() && !dst_path.exists() {
        info!(
            "Copying file: {} from {} => {}",
            &f.id,
            src_path.to_str().unwrap(),
            dst_path.to_str().unwrap()
        );
        tokio::fs::copy(src_path, dst_path).await?;
    } else if dst_path.exists() {
        info!("File already exists {}, continuing...", &f.id);
    } else {
        anyhow::bail!("Source file not found {}", src_path.to_str().unwrap());
    }
    let uid = db.upsert_user(&pubkey_vec).await?;
    info!("Mapped user {} => {}", &f.email, uid);

    let md: Option<Vec<&str>> = match &f.media_dimensions {
        Some(s) => Some(s.split("x").collect()),
        _ => None,
    };
    let fu = FileUpload {
        id: id_vec,
        name: match &f.name {
            Some(n) => n.to_string(),
            None => "".to_string(),
        },
        size: f.size as u64,
        mime_type: f.mime_type.clone(),
        created: f.uploaded,
        width: match &md {
            Some(s) => Some(s[0].parse::<u32>()?),
            None => None,
        },
        height: match &md {
            Some(s) => Some(s[1].parse::<u32>()?),
            None => None,
        },
        blur_hash: None,
        alt: f.description.clone(),
    };
    db.add_file(&fu, uid).await?;
    Ok(())
}

#[derive(FromRow)]
struct VoidFile {
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
    fn map_to_path(&self) -> PathBuf {
        let id_str = self.id.as_hyphenated().to_string();
        PathBuf::new()
            .join("files-v2/")
            .join(&id_str[..2])
            .join(&id_str[2..4])
            .join(&id_str)
    }
}

struct VoidCatDb {
    pub pool: PgPool,
}

impl VoidCatDb {
    async fn connect(conn: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPool::connect(conn).await?;
        Ok(Self { pool })
    }

    async fn list_files(&self, page: usize) -> Result<Vec<VoidFile>, sqlx::Error> {
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
}
