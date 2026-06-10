//! Pi-side publisher (ADRs 0003/0004/0010): polls the PM5 over USB HID,
//! publishes telemetry to JetStream, and handles plan/control commands.
//!
//! This binary must never transitively depend on DuckDB (ADR 0002); CI
//! enforces it (`cargo tree -p monorail-rower -i duckdb` must fail).

use clap::Parser;
use monorail_core::RowerId;

/// Configuration, sourced from flags or the systemd environment file
/// (`/etc/monorail/rower.env`, ADR 0008).
#[derive(Debug, Parser)]
#[command(version, about)]
struct Config {
    /// NATS server URL.
    #[arg(
        long,
        env = "MONORAIL_NATS_URL",
        default_value = "nats://localhost:4222"
    )]
    nats_url: String,

    /// Identifier for this erg/Pi pairing (lowercase, digits, dashes).
    #[arg(long, env = "MONORAIL_ROWER_ID", default_value = "erg-1")]
    rower_id: String,

    /// Fast-loop poll rate for monitor snapshots, Hz.
    #[arg(long, env = "MONORAIL_POLL_HZ", default_value_t = 10)]
    poll_hz: u32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Config::parse();
    let rower_id = RowerId::new(&config.rower_id)
        .ok_or_else(|| anyhow::anyhow!("invalid rower id {:?}", config.rower_id))?;

    tracing::info!(
        rower_id = %rower_id,
        nats_url = %config.nats_url,
        poll_hz = config.poll_hz,
        "monorail-rower starting"
    );

    // TODO wiring order:
    // 1. PM5 discovery + reconnect loop (monorail_pm5::transport)
    // 2. NATS connect + JetStream publish with dedup ids (monorail_stream)
    // 3. fast/slow poll loops -> Envelope<MonitorSample>/StrokeSample
    // 4. command subscriber: program PM5, ack/nack (ADR 0010)
    anyhow::bail!("not yet implemented: PM5 polling and publishing");
}
