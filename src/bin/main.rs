use anyhow::{bail, Error};
use clap::Parser;
use config::Config;
use log::{error, info};
use rocket::config::Ident;
use rocket::data::{ByteUnit, Limits};
use rocket::routes;
use rocket::shield::Shield;
use route96::background::start_background_tasks;
use route96::cors::CORS;
use route96::db::Database;
use route96::filesystem::FileStore;
use route96::nip29::init_nip29_client;
use route96::routes;
use route96::routes::{get_blob_route, head_blob, root};
use route96::settings::Settings;
use std::net::{IpAddr, SocketAddr};

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

    let settings: Settings = builder.try_deserialize()?;

    let db = Database::new_with_settings(&settings.database, &settings.storage_dir).await?;

    info!("Running DB migration");
    db.migrate().await?;

    // Initialize the NIP-29 client if configured
    let nip29_client = match init_nip29_client(&settings).await {
        Ok(client) => client,
        Err(e) => {
            bail!("Failed to initialize NIP-29 client: {}", e);
        }
    };

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

    let fs = FileStore::new(settings.clone());
    let mut rocket = rocket::Rocket::custom(config)
        .manage(fs.clone())
        .manage(settings.clone())
        .manage(db.clone());

    rocket = rocket.manage(nip29_client);

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
                routes::void_cat_redirect_head
            ],
        )
        .mount("/admin", routes::admin_routes());

    #[cfg(feature = "blossom")]
    {
        rocket = rocket.mount("/", routes::blossom_routes());
    }
    #[cfg(feature = "media-compression")]
    {
        rocket = rocket.mount("/", routes![routes::get_blob_thumb]);
    }

    let jh = start_background_tasks(db, fs).await;

    if let Err(e) = rocket.launch().await {
        error!("Rocker error {}", e);
        for j in jh {
            let _ = j.await?;
        }
        Err(Error::from(e))
    } else {
        for j in jh {
            let _ = j.await?;
        }
        Ok(())
    }
}
