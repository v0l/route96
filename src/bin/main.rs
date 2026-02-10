use std::net::{IpAddr, SocketAddr};

use anyhow::Error;
use axum::{
    Router,
    routing::{get, head, post, put, delete},
};
use clap::Parser;
use config::Config;
#[cfg(feature = "payments")]
use fedimint_tonic_lnd::lnrpc::GetInfoRequest;
use log::{error, info};
#[cfg(feature = "analytics")]
use route96::analytics::plausible::PlausibleAnalytics;
#[cfg(feature = "analytics")]
use route96::analytics::AnalyticsLayer;
use route96::background::start_background_tasks;
use route96::cors::cors_layer;
use route96::db::Database;
use route96::filesystem::FileStore;
use route96::routes;
use route96::settings::Settings;
use route96::whitelist::Whitelist;
use tokio::sync::broadcast;
use tower_http::limit::RequestBodyLimitLayer;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    #[arg(long)]
    pub config: Option<String>,
}

#[tokio::main]
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

    let addr: SocketAddr = match &settings.listen {
        Some(i) => i.parse()?,
        None => SocketAddr::new(IpAddr::from([0, 0, 0, 0]), 8000),
    };

    let fs = FileStore::new(settings.clone());
    let wl = Whitelist::new(settings.whitelist.clone());

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
            Some(lnd)
        } else {
            None
        }
    };

    // Build the router
    let mut app = Router::new()
        .route("/", get(routes::root))
        .route("/:sha256", get(routes::get_blob))
        .route("/:sha256", head(routes::head_blob));

    #[cfg(feature = "media-compression")]
    {
        app = app.route("/thumb/:sha256", get(routes::get_blob_thumb));
    }

    // Add admin routes
    app = app.nest("/admin", routes::admin_routes());

    // Add blossom routes
    #[cfg(feature = "blossom")]
    {
        app = app.merge(routes::blossom_routes());
    }

    // Add nip96 routes
    #[cfg(feature = "nip96")]
    {
        app = app.merge(routes::nip96_routes());
    }

    // Add payment routes
    #[cfg(feature = "payments")]
    {
        if lnd.is_some() {
            app = app.merge(routes::payment::routes());
        }
    }

    // Add state
    let mut app = app
        .with_state(routes::AppState {
            fs: fs.clone(),
            db: db.clone(),
            settings: settings.clone(),
            wl: wl.clone(),
            #[cfg(feature = "payments")]
            lnd: lnd.clone(),
        });

    // Add middleware layers
    app = app.layer(cors_layer());
    app = app.layer(RequestBodyLimitLayer::new(settings.max_upload_bytes as usize));

    #[cfg(feature = "analytics")]
    {
        if settings.plausible_url.is_some() {
            app = app.layer(AnalyticsLayer::new(PlausibleAnalytics::new(&settings)));
        }
    }

    let (shutdown_tx, shutdown_rx) = broadcast::channel(1);
    #[cfg(not(feature = "payments"))]
    let lnd = None;
    let mut jh = start_background_tasks(db, fs, shutdown_rx.resubscribe(), lnd);
    if let Some(path) = settings.whitelist_file.clone() {
        let wh = wl.start_file_watcher(path, shutdown_rx.resubscribe());
        jh.push(wh);
    }

    info!("Starting server on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::signal::ctrl_c().await.ok();
            info!("Shutdown signal received");
        })
        .await?;

    shutdown_tx
        .send(())
        .expect("Failed to send shutdown signal");

    for j in jh {
        j.await?;
    }
    Ok(())
}
