//! Host-side sink (ADRs 0004/0006/0009/0010/0011): consumes telemetry from
//! JetStream into DuckDB, runs predictions and plan generation, pushes plans
//! over the command plane, and serves the HTTP API + Leptos UI bundle.

use std::path::PathBuf;

use clap::Parser;

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

    // TODO wiring order:
    // 1. open store (migrations) — single read-write connection (ADR 0006)
    // 2. JetStream durable pull consumer -> idempotent ingest
    // 3. SSE fan-out broadcast channel from the consumer
    // 4. command-plane client for plan pushes (ADR 0010)
    // 5. serve UI bundle as static files next to the API
    let app = monorail_api::router();
    let listener = tokio::net::TcpListener::bind(&config.listen).await?;
    tracing::info!("listening on http://{}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}
