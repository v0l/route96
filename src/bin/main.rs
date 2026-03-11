use std::net::{IpAddr, SocketAddr};

use anyhow::Error;
use axum::{
    Router,
    routing::{get, head},
};
use clap::Parser;
use config::Config;
#[cfg(feature = "payments")]
use fedimint_tonic_lnd::lnrpc::GetInfoRequest;
use log::info;
#[cfg(feature = "analytics")]
use route96::analytics::AnalyticsLayer;
#[cfg(feature = "analytics")]
use route96::analytics::plausible::PlausibleAnalytics;
use route96::background::start_background_tasks;
use route96::cors::cors_layer;
use route96::db::Database;
use route96::file_stats::FileStatsTracker;
use route96::filesystem::FileStore;
use route96::routes;
use route96::settings::Settings;
use route96::whitelist::Whitelist;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
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
    let file_stats = FileStatsTracker::new();

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
        .route("/docs.md", get(routes::docs_md))
        .route("/SKILL.md", get(routes::skill_md))
        .route("/{sha256}", head(routes::head_blob).get(routes::get_blob));

    #[cfg(feature = "media-compression")]
    {
        app = app.route("/thumb/{sha256}", get(routes::get_blob_thumb));
    }

    // Add admin routes
    app = app.merge(routes::admin_routes());

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
            app = app.merge(routes::payment::payment_routes());
        }
    }

    // Add state
    let mut app = app.with_state(Arc::new(routes::AppState {
        fs: fs.clone(),
        db: db.clone(),
        settings: settings.clone(),
        wl: wl.clone(),
        file_stats: file_stats.clone(),
        #[cfg(feature = "payments")]
        lnd: lnd.clone(),
    }));

    // Add middleware layers
    app = app.layer(cors_layer());
    app = app.layer(RequestBodyLimitLayer::new(
        settings.max_upload_bytes as usize,
    ));

    #[cfg(feature = "analytics")]
    {
        if settings.plausible_url.is_some() {
            app = app.layer(AnalyticsLayer::new(PlausibleAnalytics::new(&settings)));
        }
    }

    let shutdown = CancellationToken::new();
    let mut jh = start_background_tasks(
        db.clone(),
        fs.clone(),
        settings.clone(),
        shutdown.clone(),
        file_stats.clone(),
        #[cfg(feature = "payments")]
        lnd.clone(),
    );
    if let Some(path) = settings.whitelist_file.clone() {
        let wh = wl.start_file_watcher(path, shutdown.clone());
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

    shutdown.cancel();

    for j in jh {
        j.await?;
    }
    Ok(())
}
