//! JetStream consumer loop: pulls telemetry, hands each message to a
//! handler, acks only after the handler succeeds (at-least-once, ADR 0004).

use async_nats::jetstream::context::Context;
use futures::StreamExt;
use monorail_core::telemetry::{MonitorSample, StrokeSample, WorkoutEvent};
use monorail_core::wire::Envelope;
use monorail_stream::jetstream::{ensure_pull_consumer, TelemetryKind};

/// Durable consumer name; one per sink (single-writer rule, ADR 0006).
pub const DURABLE_NAME: &str = "sink";

/// Run the consume loop until the stream ends or an unrecoverable error.
pub async fn run(js: &Context) -> anyhow::Result<()> {
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

        match handle(&message) {
            Ok(()) => {
                if let Err(error) = message.ack().await {
                    tracing::warn!(%error, "ack failed; message will redeliver");
                }
            }
            Err(error) => {
                // No ack: JetStream redelivers. Dedup by (session_id, seq)
                // makes the retry safe once the store lands (ADR 0006).
                tracing::error!(%error, subject = %message.subject, "handler failed");
            }
        }
    }
    Ok(())
}

/// Phase-1 handler: parse and log. The DuckDB ingest (ADR 0006) replaces the
/// logging body; the parse-by-kind structure stays.
fn handle(message: &async_nats::jetstream::Message) -> anyhow::Result<()> {
    let subject = message.subject.as_str();
    let kind = TelemetryKind::from_subject(subject)
        .ok_or_else(|| anyhow::anyhow!("unclassifiable subject {subject}"))?;

    match kind {
        TelemetryKind::Monitor => {
            let env: Envelope<MonitorSample> = serde_json::from_slice(&message.payload)?;
            tracing::info!(
                session = %env.session_id,
                seq = env.seq,
                elapsed_s = format!("{:.1}", env.payload.elapsed_s),
                distance_m = format!("{:.1}", env.payload.distance_m),
                split = format!("{:.1}", env.payload.split_s_per_500m),
                spm = format!("{:.1}", env.payload.stroke_rate_spm),
                watts = format!("{:.0}", env.payload.power_watts),
                "monitor"
            );
        }
        TelemetryKind::Stroke => {
            let env: Envelope<StrokeSample> = serde_json::from_slice(&message.payload)?;
            tracing::info!(
                session = %env.session_id,
                seq = env.seq,
                stroke = env.payload.stroke_number,
                watts = format!("{:.0}", env.payload.power_watts),
                "stroke"
            );
        }
        TelemetryKind::WorkoutEvent => {
            let env: Envelope<WorkoutEvent> = serde_json::from_slice(&message.payload)?;
            tracing::info!(session = %env.session_id, seq = env.seq, event = ?env.payload, "workout");
        }
    }
    Ok(())
}
