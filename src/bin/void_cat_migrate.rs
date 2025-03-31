use anyhow::Error;
use clap::Parser;
use config::Config;
use log::{info, warn};
use nostr::bitcoin::base58;
use route96::db::{Database, FileUpload};
use route96::filesystem::FileStore;
use route96::settings::Settings;
use route96::void_db::VoidCatDb;
use route96::void_file::VoidFile;
use std::path::PathBuf;
use tokio::io::{AsyncWriteExt, BufWriter};

#[derive(Debug, Clone, clap::ValueEnum)]
enum ArgOperation {
    Migrate,
    ExportNginxRedirects,
}

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Database connection string for void.cat DB
    #[arg(long)]
    pub database: String,

    /// Path to filestore on void.cat
    #[arg(long)]
    pub data_path: String,

    #[arg(long)]
    pub operation: ArgOperation,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();

    let builder = Config::builder()
        .add_source(config::File::with_name("config.yaml"))
        .add_source(config::Environment::with_prefix("APP"))
        .build()?;

    let settings: Settings = builder.try_deserialize()?;

    let db = Database::new(&settings.database).await?;
    let fs = FileStore::new(settings.clone());

    let args: Args = Args::parse();

    let db_void = VoidCatDb::connect(&args.database).await?;

    match args.operation {
        ArgOperation::Migrate => {
            let mut page = 0;
            loop {
                let files = db_void.list_files(page).await?;
                if files.is_empty() {
                    break;
                }
                for f in files {
                    if let Err(e) = migrate_file(&f, &db, &fs, &args).await {
                        warn!("Failed to migrate file: {}, {}", &f.id, e);
                    }
                }
                page += 1;
            }
        }
        ArgOperation::ExportNginxRedirects => {
            let path: PathBuf = args.data_path.parse()?;
            let conf_path = &path.join("nginx.conf");
            info!("Writing redirects to {}", conf_path.to_str().unwrap());
            let mut fout = BufWriter::new(tokio::fs::File::create(conf_path).await?);
            let mut page = 0;
            loop {
                let files = db_void.list_files(page).await?;
                if files.is_empty() {
                    break;
                }
                for f in files {
                    let legacy_id = base58::encode(f.id.to_bytes_le().as_slice());
                    let redirect = format!("location ^\\/d\\/{}(?:\\.\\w+)?$ {{\n\treturn 301 https://nostr.download/{};\n}}\n", &legacy_id, &f.digest);
                    fout.write_all(redirect.as_bytes()).await?;
                }
                page += 1;
            }
        }
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
    let src_path = PathBuf::new()
        .join(&args.data_path)
        .join(VoidFile::map_to_path(&f.id));
    let dst_path = fs.get(&id_vec);
    if src_path.exists() && !dst_path.exists() {
        info!(
            "Copying file: {} from {} => {}",
            &f.id,
            src_path.to_str().unwrap(),
            dst_path.to_str().unwrap()
        );

        tokio::fs::create_dir_all(dst_path.parent().unwrap()).await?;
        tokio::fs::copy(src_path, dst_path).await?;
    } else if dst_path.exists() {
        info!("File already exists {}, continuing...", &f.id);
    } else {
        anyhow::bail!("Source file not found {}", src_path.to_str().unwrap());
    }
    let uid = db.upsert_user(&pubkey_vec).await?;
    info!("Mapped user {} => {}", &f.email, uid);

    let md: Option<Vec<&str>> = f.media_dimensions.as_ref().map(|s| s.split("x").collect());
    let fu = FileUpload {
        id: id_vec,
        name: f.name.clone(),
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
        duration: None,
        bitrate: None,
    };
    db.add_file(&fu, Some(uid)).await?;
    Ok(())
}
