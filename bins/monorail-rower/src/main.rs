//! Pi-side publisher (ADRs 0003/0004/0010): polls a telemetry source and
//! publishes enveloped telemetry to NATS JetStream.
//!
//! Source is currently the deterministic [`source::FakePm5`]; the CSAFE/HID
//! source replaces it behind the same `SourceEvent` vocabulary.
//!
//! This binary must never transitively depend on DuckDB (ADR 0002); CI
//! enforces it (`cargo tree -p monorail-rower -i duckdb` must fail).

mod command;
mod source;

use chrono::Utc;
use clap::Parser;
use monorail_core::telemetry::WorkoutEvent;
use monorail_core::wire::{Envelope, WIRE_VERSION};
use monorail_core::{RowerId, SessionId};
use monorail_stream::jetstream::{connect, ensure_stream, TelemetryPublisher};
use serde::de::DeserializeOwned;
use serde::Serialize;
use uuid::Uuid;

use source::{FakePm5, SourceEvent};

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

    /// Fake-source session length in seconds.
    #[arg(long, env = "MONORAIL_FAKE_DURATION_S", default_value_t = 120)]
    fake_duration_s: u32,

    /// Fake-source target split (seconds per 500 m).
    #[arg(long, env = "MONORAIL_FAKE_SPLIT_S", default_value_t = 120.0)]
    fake_split_s: f32,

    /// Fake-source target stroke rate (strokes per minute).
    #[arg(long, env = "MONORAIL_FAKE_SPM", default_value_t = 20.0)]
    fake_spm: f32,

    /// Wait for a pushed plan (ADR 0010) and run the workout with its
    /// targets instead of starting immediately.
    #[arg(long, env = "MONORAIL_WAIT_FOR_PLAN", default_value_t = false)]
    wait_for_plan: bool,
}

/// Stamps envelopes with the session id and a monotonic sequence.
struct Session {
    id: SessionId,
    seq: u64,
}

impl Session {
    fn new() -> Self {
        Self {
            id: SessionId(Uuid::new_v4()),
            seq: 0,
        }
    }

    fn envelope<T: Serialize + DeserializeOwned>(&mut self, payload: T) -> Envelope<T> {
        let envelope = Envelope {
            v: WIRE_VERSION,
            session_id: self.id,
            seq: self.seq,
            ts: Utc::now(),
            payload,
        };
        self.seq += 1;
        envelope
    }
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
        "monorail-rower starting (fake PM5 source)"
    );

    let (client, js) = connect(&config.nats_url).await?;
    ensure_stream(&js).await?;

    // A pushed plan overrides the fake source's targets and duration, and is
    // stamped into the session's events so telemetry joins back to it.
    let (split_s, spm, duration_s, plan_id) = if config.wait_for_plan {
        let applied = command::wait_for_plan(&client, &rower_id).await?;
        (
            applied.target_split_s,
            applied.target_spm,
            applied.duration_s,
            Some(applied.plan.plan_id),
        )
    } else {
        (
            config.fake_split_s,
            config.fake_spm,
            config.fake_duration_s,
            None,
        )
    };

    let publisher = TelemetryPublisher::new(js, rower_id);
    let mut session = Session::new();
    let mut fake = FakePm5::new(split_s, spm);
    let dt_s = 1.0 / config.poll_hz as f64;
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs_f64(dt_s));

    tracing::info!(session_id = %session.id, ?plan_id, split_s, spm, "workout started");
    publisher
        .publish_workout_event(&session.envelope(WorkoutEvent::Started {
            ts: Utc::now(),
            plan_id,
        }))
        .await?;

    let total_ticks = duration_s as f64 * config.poll_hz as f64;
    let total_ticks = total_ticks as u64;
    for _ in 0..total_ticks {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("interrupted, ending workout early");
                break;
            }
        }
        for event in fake.advance(dt_s) {
            match event {
                SourceEvent::Monitor(sample) => {
                    publisher.publish_monitor(&session.envelope(sample)).await?
                }
                SourceEvent::Stroke(sample) => {
                    tracing::debug!(stroke = sample.stroke_number, "stroke");
                    publisher.publish_stroke(&session.envelope(sample)).await?
                }
            }
        }
    }

    let summary = fake.summary();
    tracing::info!(
        distance_m = summary.distance_m,
        strokes = summary.stroke_count,
        "workout ended"
    );
    publisher
        .publish_workout_event(&session.envelope(WorkoutEvent::Ended {
            ts: Utc::now(),
            summary,
        }))
        .await?;

    Ok(())
}
