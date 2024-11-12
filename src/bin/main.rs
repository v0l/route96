use std::net::{IpAddr, SocketAddr};

use anyhow::Error;
use clap::Parser;
use config::Config;
use log::{error, info};
use rocket::config::Ident;
use rocket::data::{ByteUnit, Limits};
use rocket::routes;
use rocket::shield::Shield;
#[cfg(feature = "analytics")]
use route96::analytics::plausible::PlausibleAnalytics;
#[cfg(feature = "analytics")]
use route96::analytics::AnalyticsFairing;
use route96::cors::CORS;
use route96::db::Database;
use route96::filesystem::FileStore;
use route96::routes;
use route96::routes::{get_blob, head_blob, root};
use route96::settings::Settings;
#[cfg(feature = "void-cat-redirects")]
use route96::void_db::VoidCatDb;
use route96::webhook::Webhook;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
}

#[rocket::main]
async fn main() -> Result<(), Error> {
    pretty_env_logger::init();

    let builder = Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("APP"))
        .build()?;

    let settings: Settings = builder.try_deserialize()?;

    let db = Database::new(&settings.database).await?;

    let _args: Args = Args::parse();
    
    info!("Running DB migration");
    db.migrate().await?;

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

    let mut rocket = rocket::Rocket::custom(config)
        .manage(FileStore::new(settings.clone()))
        .manage(settings.clone())
        .manage(db.clone())
        .manage(
            settings
                .webhook_url
                .as_ref()
                .map(|w| Webhook::new(w.clone())),
        )
        .attach(CORS)
        .attach(Shield::new()) // disable
        .mount("/", routes![root, get_blob, head_blob])
        .mount("/admin", routes::admin_routes());

    #[cfg(feature = "analytics")]
    {
        if settings.plausible_url.is_some() {
            rocket = rocket.attach(AnalyticsFairing::new(PlausibleAnalytics::new(&settings)))
        }
    }
    #[cfg(feature = "blossom")]
    {
        rocket = rocket.mount("/", routes::blossom_routes());
    }
    #[cfg(feature = "nip96")]
    {
        rocket = rocket.mount("/", routes::nip96_routes());
    }
    #[cfg(feature = "void-cat-redirects")]
    {
        if let Some(conn) = settings.void_cat_database {
            let vdb = VoidCatDb::connect(&conn).await?;
            rocket = rocket
                .mount("/", routes![routes::void_cat_redirect])
                .manage(vdb);
        }
    }
    if let Err(e) = rocket.launch().await {
        error!("Rocker error {}", e);
        Err(Error::from(e))
    } else {
        Ok(())
    }
}
