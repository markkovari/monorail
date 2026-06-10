//! JetStream wiring: stream provisioning, telemetry publishing with dedup
//! headers, durable pull consumers (ADR 0004).

use std::time::Duration;

use async_nats::jetstream::consumer::pull::Config as PullConfig;
use async_nats::jetstream::consumer::Consumer;
use async_nats::jetstream::context::Context;
use async_nats::jetstream::stream::{Config as StreamConfig, RetentionPolicy, StorageType, Stream};
use monorail_core::telemetry::{MonitorSample, StrokeSample, WorkoutEvent};
use monorail_core::wire::Envelope;
use monorail_core::RowerId;
use serde::de::DeserializeOwned;
use serde::Serialize;
use thiserror::Error;

use crate::{subjects, STREAM_NAME, STREAM_SUBJECTS};

/// JetStream deduplication window; `Nats-Msg-Id` repeats inside it are
/// dropped server-side.
pub const DEDUP_WINDOW: Duration = Duration::from_secs(120);

#[derive(Debug, Error)]
pub enum StreamError {
    #[error("connect failed: {0}")]
    Connect(#[from] async_nats::ConnectError),
    #[error("stream create failed: {0}")]
    CreateStream(#[from] async_nats::jetstream::context::CreateStreamError),
    #[error("consumer create failed: {0}")]
    CreateConsumer(#[from] async_nats::jetstream::stream::ConsumerError),
    #[error("get stream failed: {0}")]
    GetStream(#[from] async_nats::jetstream::context::GetStreamError),
    #[error("publish failed: {0}")]
    Publish(#[from] async_nats::jetstream::context::PublishError),
    #[error("serialize failed: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Connect to NATS, returning the core client (command plane, ADR 0010) and
/// a JetStream context over it (telemetry, ADR 0004).
pub async fn connect(url: &str) -> Result<(async_nats::Client, Context), StreamError> {
    let client = async_nats::connect(url).await?;
    let context = async_nats::jetstream::new(client.clone());
    Ok((client, context))
}

/// Create or update the durable telemetry stream (ADR 0004): telemetry +
/// workout subjects only, file storage, limits retention.
pub async fn ensure_stream(js: &Context) -> Result<Stream, StreamError> {
    let stream = js
        .get_or_create_stream(StreamConfig {
            name: STREAM_NAME.to_string(),
            subjects: STREAM_SUBJECTS.iter().map(|s| s.to_string()).collect(),
            storage: StorageType::File,
            retention: RetentionPolicy::Limits,
            duplicate_window: DEDUP_WINDOW,
            ..Default::default()
        })
        .await?;
    Ok(stream)
}

/// Durable pull consumer over the telemetry stream.
pub async fn ensure_pull_consumer(
    js: &Context,
    durable: &str,
) -> Result<Consumer<PullConfig>, StreamError> {
    let stream = js.get_stream(STREAM_NAME).await?;
    let consumer = stream
        .get_or_create_consumer(
            durable,
            PullConfig {
                durable_name: Some(durable.to_string()),
                ..Default::default()
            },
        )
        .await?;
    Ok(consumer)
}

/// Publishes enveloped telemetry for one rower, with `Nats-Msg-Id` set from
/// the envelope's `(session_id, seq)` so redelivery after retries dedups.
pub struct TelemetryPublisher {
    js: Context,
    rower: RowerId,
}

impl TelemetryPublisher {
    pub fn new(js: Context, rower: RowerId) -> Self {
        Self { js, rower }
    }

    pub fn rower(&self) -> &RowerId {
        &self.rower
    }

    pub async fn publish_monitor(
        &self,
        envelope: &Envelope<MonitorSample>,
    ) -> Result<(), StreamError> {
        self.publish(subjects::telemetry_monitor(&self.rower), envelope)
            .await
    }

    pub async fn publish_stroke(
        &self,
        envelope: &Envelope<StrokeSample>,
    ) -> Result<(), StreamError> {
        self.publish(subjects::telemetry_stroke(&self.rower), envelope)
            .await
    }

    pub async fn publish_workout_event(
        &self,
        envelope: &Envelope<WorkoutEvent>,
    ) -> Result<(), StreamError> {
        self.publish(subjects::workout_event(&self.rower), envelope)
            .await
    }

    async fn publish<T: Serialize + DeserializeOwned>(
        &self,
        subject: String,
        envelope: &Envelope<T>,
    ) -> Result<(), StreamError> {
        let mut headers = async_nats::HeaderMap::new();
        headers.insert("Nats-Msg-Id", envelope.dedup_id().as_str());
        let payload = serde_json::to_vec(envelope)?;
        self.js
            .publish_with_headers(subject, headers, payload.into())
            .await?
            // Await the JetStream ack: publish is durable once this returns.
            .await?;
        Ok(())
    }
}

/// Kind of telemetry carried by a message, classified from its subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryKind {
    Monitor,
    Stroke,
    WorkoutEvent,
}

impl TelemetryKind {
    /// Classify a subject from the telemetry stream.
    pub fn from_subject(subject: &str) -> Option<Self> {
        if !subject.starts_with("monorail.") {
            return None;
        }
        match subject.rsplit('.').next() {
            Some("monitor") => Some(Self::Monitor),
            Some("stroke") => Some(Self::Stroke),
            Some("event") => Some(Self::WorkoutEvent),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Monitor => "monitor",
            Self::Stroke => "stroke",
            Self::WorkoutEvent => "workout_event",
        }
    }
}

/// Extract the rower id token from a telemetry/workout subject.
pub fn rower_from_subject(subject: &str) -> Option<&str> {
    let mut parts = subject.split('.');
    let (root, _category, rower) = (parts.next()?, parts.next()?, parts.next()?);
    (root == "monorail").then_some(rower)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_subjects() {
        let rower = RowerId::new("erg-1").unwrap();
        assert_eq!(
            TelemetryKind::from_subject(&subjects::telemetry_monitor(&rower)),
            Some(TelemetryKind::Monitor)
        );
        assert_eq!(
            TelemetryKind::from_subject(&subjects::telemetry_stroke(&rower)),
            Some(TelemetryKind::Stroke)
        );
        assert_eq!(
            TelemetryKind::from_subject(&subjects::workout_event(&rower)),
            Some(TelemetryKind::WorkoutEvent)
        );
        assert_eq!(TelemetryKind::from_subject("other.thing.x"), None);
    }

    #[test]
    fn extracts_rower_token() {
        let rower = RowerId::new("erg-1").unwrap();
        assert_eq!(
            rower_from_subject(&subjects::telemetry_monitor(&rower)),
            Some("erg-1")
        );
        assert_eq!(rower_from_subject("bogus"), None);
    }
}
