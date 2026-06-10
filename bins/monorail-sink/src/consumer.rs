//! JetStream consumer loop: pulls telemetry, ingests into DuckDB, acks only
//! after the write succeeds (at-least-once into the store, deduplicated by
//! `(session_id, seq)` — ADRs 0004/0006).

use async_nats::jetstream::context::Context;
use futures::StreamExt;
use monorail_core::telemetry::{MonitorSample, StrokeSample, WorkoutEvent};
use monorail_core::wire::Envelope;
use monorail_store::Store;
use monorail_stream::jetstream::{ensure_pull_consumer, rower_from_subject, TelemetryKind};

/// Durable consumer name; one per sink (single-writer rule, ADR 0006).
pub const DURABLE_NAME: &str = "sink";

/// Run the consume loop until the stream ends or an unrecoverable error.
pub async fn run(js: &Context, store: Store) -> anyhow::Result<()> {
    let consumer = ensure_pull_consumer(js, DURABLE_NAME).await?;
    let mut messages = consumer.messages().await?;

    tracing::info!(durable = DURABLE_NAME, "consuming telemetry");
    while let Some(message) = messages.next().await {
        let message = match message {
            Ok(message) => message,
            Err(error) => {
                tracing::warn!(%error, "message pull failed, continuing");
                continue;
            }
        };

        match handle(&store, &message) {
            Ok(()) => {
                if let Err(error) = message.ack().await {
                    tracing::warn!(%error, "ack failed; message will redeliver");
                }
            }
            Err(error) => {
                // No ack: JetStream redelivers; (session_id, seq) dedup in
                // the store makes the retry safe.
                tracing::error!(%error, subject = %message.subject, "ingest failed");
            }
        }
    }
    Ok(())
}

/// Parse by subject kind and ingest. Insert outcome is logged at debug;
/// `fresh = false` means the dedup key already existed (redelivery).
fn handle(store: &Store, message: &async_nats::jetstream::Message) -> anyhow::Result<()> {
    let subject = message.subject.as_str();
    let kind = TelemetryKind::from_subject(subject)
        .ok_or_else(|| anyhow::anyhow!("unclassifiable subject {subject}"))?;
    let rower = rower_from_subject(subject)
        .ok_or_else(|| anyhow::anyhow!("no rower token in subject {subject}"))?;

    let fresh = match kind {
        TelemetryKind::Monitor => {
            let env: Envelope<MonitorSample> = serde_json::from_slice(&message.payload)?;
            store.ingest_monitor(rower, &env)?
        }
        TelemetryKind::Stroke => {
            let env: Envelope<StrokeSample> = serde_json::from_slice(&message.payload)?;
            store.ingest_stroke(rower, &env)?
        }
        TelemetryKind::WorkoutEvent => {
            let env: Envelope<WorkoutEvent> = serde_json::from_slice(&message.payload)?;
            tracing::info!(session = %env.session_id, event = ?env.payload, "workout event");
            store.ingest_workout_event(rower, &env)?
        }
    };
    tracing::debug!(subject, kind = kind.as_str(), fresh, "ingested");
    Ok(())
}
