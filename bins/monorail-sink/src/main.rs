//! Host-side sink (ADRs 0004/0006/0009/0010/0011): consumes telemetry from
//! JetStream into DuckDB, runs predictions and plan generation, pushes plans
//! over the command plane, and serves the HTTP API + Leptos UI bundle.

mod consumer;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use clap::Parser;
use monorail_api::AppState;
use monorail_stream::jetstream::{connect, ensure_stream};
use tokio::sync::broadcast;
use tower_http::services::ServeDir;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Config {
    /// Address for the HTTP API + UI.
    #[arg(long, env = "MONORAIL_LISTEN", default_value = "0.0.0.0:8080")]
    listen: String,

    /// NATS server URL.
    #[arg(
        long,
        env = "MONORAIL_NATS_URL",
        default_value = "nats://localhost:4222"
    )]
    nats_url: String,

    /// DuckDB database file.
    #[arg(long, env = "MONORAIL_DB_PATH", default_value = "monorail.duckdb")]
    db_path: PathBuf,

    /// Directory with the built Leptos UI bundle (`trunk build`); served at
    /// `/` when it exists.
    #[arg(long, env = "MONORAIL_UI_DIST", default_value = "web/monorail-ui/dist")]
    ui_dist: PathBuf,

    /// Concept2 Logbook access token (ADR 0013); sync endpoint answers 503
    /// without it.
    #[arg(long, env = "MONORAIL_LOGBOOK_TOKEN")]
    logbook_token: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Config::parse();
    tracing::info!(
        listen = %config.listen,
        nats_url = %config.nats_url,
        db_path = %config.db_path.display(),
        "monorail-sink starting"
    );

    let store = Arc::new(Mutex::new(monorail_store::Store::open(&config.db_path)?));
    let (client, js) = connect(&config.nats_url).await?;
    ensure_stream(&js).await?;

    let (live, _) = broadcast::channel(256);
    let consumer_store = Arc::clone(&store);
    let consumer_live = live.clone();
    let consume =
        tokio::spawn(async move { consumer::run(&js, consumer_store, consumer_live).await });

    let logbook = config
        .logbook_token
        .as_ref()
        .map(|token| Arc::new(monorail_logbook::LogbookClient::new(token.clone())));
    if logbook.is_some() {
        tracing::info!("logbook sync enabled");
    }
    let mut app = monorail_api::router(AppState::new(live, store, Some(client), logbook));
    if config.ui_dist.is_dir() {
        tracing::info!(dist = %config.ui_dist.display(), "serving UI bundle");
        app = app.fallback_service(ServeDir::new(&config.ui_dist));
    }
    let listener = tokio::net::TcpListener::bind(&config.listen).await?;
    tracing::info!("listening on http://{}", listener.local_addr()?);

    tokio::select! {
        result = axum::serve(listener, app) => result?,
        result = consume => result??,
        _ = tokio::signal::ctrl_c() => tracing::info!("shutting down"),
    }
    Ok(())
}
