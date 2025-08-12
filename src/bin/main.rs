use std::net::{IpAddr, SocketAddr};

use anyhow::Error;
use clap::Parser;
use config::Config;
#[cfg(feature = "payments")]
use fedimint_tonic_lnd::lnrpc::GetInfoRequest;
use log::{error, info};
use rocket::config::Ident;
use rocket::data::{ByteUnit, Limits};
use rocket::routes;
use rocket::shield::Shield;
#[cfg(feature = "analytics")]
use route96::analytics::plausible::PlausibleAnalytics;
#[cfg(feature = "analytics")]
use route96::analytics::AnalyticsFairing;
use route96::background::start_background_tasks;
use route96::cors::CORS;
use route96::db::Database;
use route96::filesystem::FileStore;
use route96::routes;
use route96::routes::{get_blob, head_blob, root};
use route96::settings::Settings;
use route96::whitelist::DynamicWhitelist;
use tokio::sync::broadcast;

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
        .add_source(config::Environment::with_prefix("APP"))
        .build()?;

    let settings: Settings = builder.try_deserialize()?;

    let db = Database::new(&settings.database).await?;

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

    let fs = FileStore::new(settings.clone());
    
    // Initialize dynamic whitelist if configured
    let dynamic_whitelist = if let Some(config) = &settings.dynamic_whitelist {
        info!("Initializing dynamic whitelist with program: {}", config.user_exit_program.display());
        Some(DynamicWhitelist::new(config.clone()))
    } else {
        info!("Dynamic whitelist not configured, using static whitelist only");
        None
    };

    let mut rocket = rocket::Rocket::custom(config)
        .manage(fs.clone())
        .manage(settings.clone())
        .manage(db.clone())
        .attach(CORS)
        .attach(Shield::new()) // disable
        .mount(
            "/",
            routes![
                root,
                get_blob,
                head_blob
            ],
        )
        .mount("/admin", routes::admin_routes());

    // Always manage the dynamic whitelist state, even if it's None
    // This allows the routes to expect the state parameter
    rocket = rocket.manage(dynamic_whitelist);

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
    #[cfg(feature = "media-compression")]
    {
        rocket = rocket.mount("/", routes![routes::get_blob_thumb]);
    }
    #[cfg(feature = "payments")]
    let lnd = {
        if let Some(lnd) = settings.payments.as_ref().map(|p| &p.lnd) {
            let lnd = fedimint_tonic_lnd::connect(
                lnd.endpoint.clone(),
                lnd.tls.clone(),
                lnd.macaroon.clone(),
            )
            .await?;

            let info = {
                let mut lnd = lnd.clone();
                lnd.lightning().get_info(GetInfoRequest::default()).await?
            };

            info!(
                "LND connected: {} v{}",
                info.get_ref().alias,
                info.get_ref().version
            );
            rocket = rocket
                .manage(lnd.clone())
                .mount("/", routes::payment::routes());
            Some(lnd)
        } else {
            None
        }
    };

    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    #[cfg(feature = "payments")]
    let jh = start_background_tasks(db, fs, shutdown_rx, lnd);
    #[cfg(not(feature = "payments"))]
    let jh = start_background_tasks(db, fs, shutdown_rx);

    if let Err(e) = rocket.launch().await {
        error!("Rocker error {}", e);
    }
    shutdown_tx
        .send(())
        .expect("Failed to send shutdown signal");

    for j in jh {
        j.await?;
    }
    Ok(())
}
