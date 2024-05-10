use std::net::{IpAddr, SocketAddr};

use anyhow::Error;
use config::Config;
use log::{error, info};
use rocket::routes;

use crate::cors::CORS;
use crate::db::Database;
use crate::filesystem::FileStore;
use crate::routes::{get_blob, head_blob, root};
use crate::settings::Settings;

mod auth;
mod blob;
mod cors;
mod db;
mod filesystem;
mod routes;
mod settings;
mod processing;

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
        None => SocketAddr::new(IpAddr::from([0, 0, 0, 0]), 8000)
    };
    config.address = ip.ip();
    config.port = ip.port();

    let rocket = rocket::Rocket::custom(config)
        .manage(FileStore::new(settings.clone()))
        .manage(settings.clone())
        .manage(db.clone())
        .attach(CORS)
        .mount("/", routes::blossom_routes())
        .mount("/", routes::nip96_routes())
        .mount("/", routes![root, get_blob, head_blob])
        .launch()
        .await;

    if let Err(e) = rocket {
        error!("Rocker error {}", e);
        Err(Error::from(e))
    } else {
        Ok(())
    }
}
