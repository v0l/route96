#![deny(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]

use anyhow::{Context, Error};
use clap::Parser;
use config::Config;
use log::{error, info};
use nostr_sdk::prelude::Keys;
use rocket::config::Ident;
use rocket::data::{ByteUnit, Limits};
use rocket::shield::Shield;
use rocket::{routes, Build, Rocket};
use route96::background::start_background_tasks;
use route96::cors::CORS;
use route96::db::Database;
use route96::filesystem::FileStore;
use route96::nip29::init_nip29_client;
use route96::routes;
use route96::routes::{get_blob_route, head_blob, health_check, root};
use route96::settings::{Settings, StorageBackendType};
#[cfg(feature = "s3-storage")]
use route96::spaces::SpacesStore;
use route96::storage::StorageBackend;
use std::env;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    #[arg(long)]
    pub config: Option<String>,
}

#[rocket::main]
async fn main() -> Result<(), Error> {
    pretty_env_logger::init();

    let args: Args = Args::parse();

    let builder = Config::builder()
        .add_source(config::File::with_name(if let Some(ref c) = args.config {
            c.as_str()
        } else {
            "config.yaml"
        }))
        .add_source(
            config::Environment::with_prefix("APP")
                .separator("__")
                .try_parsing(true),
        )
        .build()?;

    let settings: Settings = builder
        .try_deserialize()
        .context("Failed to deserialize settings")?;

    let db = Database::new(&settings.database)
        .await
        .context("Failed to initialize database connection")?;

    info!("Running DB migration");
    db.migrate().await.context("Database migration failed")?;

    // Get the temp dir path from std::env
    let temp_dir = env::temp_dir();
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .context(format!("Failed to create temp directory: {:?}", temp_dir))?;
    info!("Using temporary directory: {:?}", temp_dir);

    let storage: Arc<dyn StorageBackend> = match settings.storage_type {
        StorageBackendType::FileSystem => {
            info!("Using FileSystem storage backend.");
            let fs_settings = settings.filesystem.clone().context(
                "FileSystem storage type selected, but [filesystem] settings are missing",
            )?;
            // Pass the temp_dir obtained from env
            let fs = FileStore::new(&fs_settings).expect("Failed to initialize FileStore");
            Arc::new(fs)
        }
        #[cfg(feature = "s3-storage")]
        StorageBackendType::S3 => {
            info!("Using S3 storage backend.");
            let s3_settings = settings
                .s3
                .clone()
                .context("S3 storage type selected, but [s3] settings are missing")?;
            // Pass only settings to SpacesStore::new
            let spaces = SpacesStore::new(&s3_settings)
                .await
                .context("Failed to initialize S3 storage client")?;
            Arc::new(spaces)
        }
    };

    // Parse NIP-29 relay keys
    let nip29_keys = Keys::parse(&settings.nip29_relay.private_key)
        .context("Failed to parse NIP-29 private key from settings")?;

    // Initialize NIP-29 client
    let (nip29_client, _nip29_join_handle) = init_nip29_client(&settings, nip29_keys)
        .await
        .context("Failed to initialize NIP-29 client")?;

    let mut config = rocket::Config::default();
    let ip: SocketAddr = match &settings.listen {
        Some(i) => i.parse()?,
        None => SocketAddr::new(IpAddr::from([0, 0, 0, 0]), 8000),
    };
    config.address = ip.ip();
    config.port = ip.port();

    let upload_limit = ByteUnit::from(settings.max_upload_bytes);
    config.limits = Limits::new()
        .limit("file", upload_limit)
        .limit("data-form", upload_limit)
        .limit("form", upload_limit);
    config.ident = Ident::try_new("route96").unwrap();

    let mut rocket: Rocket<Build> = rocket::custom(config)
        .manage(storage.clone())
        .manage(settings.clone())
        .manage(db.clone())
        .manage(nip29_client);

    rocket = rocket
        .attach(CORS)
        .attach(Shield::default())
        .mount(
            "/",
            routes![
                root,
                get_blob_route,
                head_blob,
                routes::void_cat_redirect,
                routes::void_cat_redirect_head,
                health_check
            ],
        )
        .mount("/admin", routes::admin::admin_routes());

    {
        println!("DEBUG: Mounting blossom routes...");
        rocket = rocket.mount("/", routes::blossom_routes());
    }
    #[cfg(feature = "media-compression")]
    {
        rocket = rocket.mount("/", routes![routes::get_blob_thumb]);
    }

    let jh = start_background_tasks(Arc::new(db), storage, &settings).await;

    if let Err(e) = rocket.launch().await {
        error!("Rocket error: {}", e);
        for j in jh {
            let _ = j.await?;
        }
        Err(Error::from(e).context("Rocket server failed to launch"))
    } else {
        for j in jh {
            let _ = j.await?;
        }
        Ok(())
    }
}
