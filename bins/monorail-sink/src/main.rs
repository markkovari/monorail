//! Host-side sink (ADRs 0004/0006/0009/0010/0011): consumes telemetry from
//! JetStream into DuckDB, runs predictions and plan generation, pushes plans
//! over the command plane, and serves the HTTP API + Leptos UI bundle.

mod consumer;

use std::path::PathBuf;

use clap::Parser;
use monorail_stream::jetstream::{connect, ensure_stream};

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

    // TODO wiring still to land:
    // - SSE fan-out broadcast channel from the consumer (ADR 0011)
    // - command-plane client for plan pushes (ADR 0010)
    // - serve UI bundle as static files next to the API
    let store = monorail_store::Store::open(&config.db_path)?;
    let js = connect(&config.nats_url).await?;
    ensure_stream(&js).await?;

    let consume = tokio::spawn(async move { consumer::run(&js, store).await });

    let app = monorail_api::router();
    let listener = tokio::net::TcpListener::bind(&config.listen).await?;
    tracing::info!("listening on http://{}", listener.local_addr()?);

    tokio::select! {
        result = axum::serve(listener, app) => result?,
        result = consume => result??,
        _ = tokio::signal::ctrl_c() => tracing::info!("shutting down"),
    }
    Ok(())
}
