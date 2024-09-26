use std::net::{IpAddr, SocketAddr};

use anyhow::Error;
use config::Config;
use log::{error, info};
use rocket::config::Ident;
use rocket::data::{ByteUnit, Limits};
use rocket::routes;
use rocket::shield::Shield;

use route96::cors::CORS;
use route96::db::Database;
use route96::filesystem::FileStore;
use route96::routes;
use route96::routes::{get_blob, head_blob, root};
use route96::settings::Settings;
use route96::webhook::Webhook;

#[rocket::main]
async fn main() -> Result<(), Error> {
    pretty_env_logger::init();

    let builder = Config::builder()
        .add_source(config::File::with_name("config.toml"))
        .add_source(config::Environment::with_prefix("APP"))
        .build()?;

    let settings: Settings = builder.try_deserialize()?;

    let db = Database::new(&settings.database).await?;

    info!("Running DB migration");
    db.migrate().await?;

    let mut config = rocket::Config::default();
    let ip: SocketAddr = match &settings.listen {
        Some(i) => i.parse().unwrap(),
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
        .mount("/", routes![root, get_blob, head_blob]);

    #[cfg(feature = "blossom")]
    {
        rocket = rocket.mount("/", routes::blossom_routes());
    }
    #[cfg(feature = "nip96")]
    {
        rocket = rocket.mount("/", routes::nip96_routes());
    }
    if let Err(e) = rocket.launch().await {
        error!("Rocker error {}", e);
        Err(Error::from(e))
    } else {
        Ok(())
    }
}
