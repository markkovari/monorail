//! JetStream consumer loop: pulls telemetry, ingests into DuckDB, fans the
//! raw envelope out to live SSE subscribers, acks only after the write
//! succeeds (at-least-once into the store, deduplicated by
//! `(session_id, seq)` — ADRs 0004/0006/0011).

use std::sync::{Arc, Mutex};

use async_nats::jetstream::context::Context;
use futures::StreamExt;
use monorail_api::LiveEvent;
use monorail_core::telemetry::{MonitorSample, StrokeSample, WorkoutEvent};
use monorail_core::wire::Envelope;
use monorail_store::Store;
use monorail_stream::jetstream::{ensure_pull_consumer, rower_from_subject, TelemetryKind};
use tokio::sync::broadcast;

/// Durable consumer name; one per sink (single-writer rule, ADR 0006).
pub const DURABLE_NAME: &str = "sink";

/// Run the consume loop until the stream ends or an unrecoverable error.
pub async fn run(
    js: &Context,
    store: Arc<Mutex<Store>>,
    live: broadcast::Sender<LiveEvent>,
) -> anyhow::Result<()> {
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

        match handle(&store, &live, &message) {
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

/// Parse by subject kind, ingest, then fan out to live subscribers.
/// Fan-out only for fresh rows: a redelivered duplicate was already seen.
fn handle(
    store: &Mutex<Store>,
    live: &broadcast::Sender<LiveEvent>,
    message: &async_nats::jetstream::Message,
) -> anyhow::Result<()> {
    let subject = message.subject.as_str();
    let kind = TelemetryKind::from_subject(subject)
        .ok_or_else(|| anyhow::anyhow!("unclassifiable subject {subject}"))?;
    let rower = rower_from_subject(subject)
        .ok_or_else(|| anyhow::anyhow!("no rower token in subject {subject}"))?;

    let store = store
        .lock()
        .map_err(|_| anyhow::anyhow!("store mutex poisoned"))?;
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
    drop(store);

    if fresh {
        // Send error just means no dashboard is watching.
        let _ = live.send(LiveEvent {
            rower: rower.to_string(),
            kind: kind.as_str(),
            payload: String::from_utf8_lossy(&message.payload).into_owned(),
        });
    }
    tracing::debug!(subject, kind = kind.as_str(), fresh, "ingested");
    Ok(())
}
