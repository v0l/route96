use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, RwLock};

use anyhow::Error;
use axum::{
    Router,
    routing::{get, head},
};
use clap::Parser;
use config::Config;
#[cfg(feature = "payments")]
use fedimint_tonic_lnd::lnrpc::GetInfoRequest;
use log::{info, warn};
use std::time::Duration;
#[cfg(feature = "analytics")]
use route96::analytics::AnalyticsLayer;
#[cfg(feature = "analytics")]
use route96::analytics::plausible::PlausibleAnalytics;
use route96::background::start_background_tasks;
use route96::config_watcher::{build_settings, watch_config};
use route96::cors::cors_layer;
use route96::db::Database;
use route96::file_stats::FileStatsTracker;
use route96::filesystem::FileStore;
use route96::routes;
use route96::settings::{Settings, WhitelistMode};
use route96::whitelist::Whitelist;
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

    let config_path = args.config.as_deref().unwrap_or("config.yaml").to_string();

    // ── Step 1: load initial settings from file + env only (no DB yet) ──────
    let builder = Config::builder()
        .add_source(config::File::with_name(&config_path))
        .add_source(config::Environment::with_prefix("APP"))
        .build()?;

    let initial_settings: Settings = builder.try_deserialize()?;

    let db = Database::new(&initial_settings.database).await?;

    info!("Running DB migration");
    db.migrate().await?;

    // ── Step 2: rebuild settings with DB overrides applied ──────────────────
    let settings = build_settings(&config_path, &db).await?;

    let addr: SocketAddr = match &settings.listen {
        Some(i) => i.parse()?,
        None => SocketAddr::new(IpAddr::from([0, 0, 0, 0]), 8000),
    };

    let fs = FileStore::new(settings.clone());
    let file_stats = FileStatsTracker::new();

    // Wrap settings and whitelist in Arc<RwLock<>> so the watcher can
    // hot-reload them without restarting the server.
    let live_settings: Arc<RwLock<Settings>> = Arc::new(RwLock::new(settings.clone()));
    let live_wl: Arc<RwLock<Whitelist>> = Arc::new(RwLock::new(
        Whitelist::from_mode(settings.whitelist.as_ref(), Some(&db)),
    ));

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
        settings: live_settings.clone(),
        wl: live_wl.clone(),
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
    if let Some(WhitelistMode::File(path)) = settings.whitelist.clone() {
        jh.spawn(Whitelist::watch_file(live_wl.clone(), path, shutdown.clone()));
    }

    // Start the config hot-reload watcher.  It rebuilds both Settings and
    // Whitelist on every change so runtime mode switches (e.g. enabling the
    // whitelist) take effect without a restart.
    jh.spawn(watch_config(
        config_path,
        db.clone(),
        live_settings.clone(),
        live_wl.clone(),
        shutdown.clone(),
    ));

    info!("Starting server on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Wait for Ctrl+C, then cancel everything.
    let shutdown_signal = shutdown.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        info!("Shutdown signal received");
        shutdown_signal.cancel();
    });

    // Run until the shutdown token is cancelled.  Once cancelled, axum enters
    // graceful-drain mode: it stops accepting new connections and waits for
    // existing ones to close.  We give that drain up to 5 s; after that we
    // abandon any lingering keep-alive connections.
    let serve = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown.clone().cancelled_owned());

    tokio::select! {
        res = serve => { res.ok(); }
        _ = async {
            shutdown.cancelled().await;
            tokio::time::sleep(Duration::from_secs(5)).await;
        } => {
            warn!("Graceful HTTP shutdown timed out; abandoning open connections");
        }
    }

    // Give background tasks up to 5 s to finish cleanly (e.g. final stats
    // flush). After that, force-exit — CPU-bound work like model loading
    // cannot be interrupted and should not hold up shutdown.
    if tokio::time::timeout(Duration::from_secs(5), async move {
        while jh.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        warn!("Background tasks did not finish in time; forcing exit");
    }

    std::process::exit(0);
}
