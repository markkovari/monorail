//! Message envelope and command types (ADRs 0005/0010).

use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{PlanId, SessionId};

/// Current envelope schema version. Bump only on breaking change; additive
/// fields are non-breaking because consumers ignore unknown fields.
pub const WIRE_VERSION: u16 = 1;

/// Envelope wrapping every telemetry payload on the wire (ADR 0005).
///
/// `(session_id, seq)` is the system-wide idempotency key: it feeds the
/// JetStream `Nats-Msg-Id` dedup header and the DuckDB primary key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub v: u16,
    pub session_id: SessionId,
    /// Monotonic per-session sequence number.
    pub seq: u64,
    /// Capture timestamp (Pi clock, NTP-synced).
    pub ts: DateTime<Utc>,
    pub payload: T,
}

impl<T: Serialize + DeserializeOwned> Envelope<T> {
    /// Value for the `Nats-Msg-Id` header used by JetStream deduplication.
    pub fn dedup_id(&self) -> String {
        format!("{}-{}", self.session_id, self.seq)
    }
}

/// Server → Pi commands on `monorail.command.<rower>.*` (ADR 0010).
/// Core NATS request/reply, deliberately not persisted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Program the attached PM5 with this plan.
    ProgramWorkout { plan: crate::plan::WorkoutPlan },
    /// Clear any programmed-but-unstarted workout.
    ClearWorkout,
}

/// Fidelity actually achieved when mapping a plan onto the PM5 (ADR 0010).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "fidelity", rename_all = "snake_case")]
pub enum Programmed {
    /// Plan represented exactly as variable intervals.
    Exact,
    /// Fell back to a single time/distance workout; segments advisory.
    Approximate { reason: String },
}

/// Pi → server reply to a [`Command`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum CommandReply {
    Ack {
        plan_id: Option<PlanId>,
        programmed: Option<Programmed>,
    },
    Nack {
        reason: NackReason,
        detail: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NackReason {
    Pm5Offline,
    Pm5Busy,
    PlanDoesNotFit,
    InvalidCommand,
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use uuid::Uuid;

    use super::*;
    use crate::telemetry::{MonitorSample, StrokePhase};

    fn sample_envelope() -> Envelope<MonitorSample> {
        Envelope {
            v: WIRE_VERSION,
            session_id: SessionId(Uuid::from_u128(0xfeed_beef)),
            seq: 42,
            ts: Utc.with_ymd_and_hms(2026, 6, 10, 6, 30, 0).unwrap(),
            payload: MonitorSample {
                elapsed_s: 90.5,
                distance_m: 412.0,
                split_s_per_500m: 120.2,
                stroke_rate_spm: 20.0,
                power_watts: 185.0,
                heart_rate_bpm: Some(142),
                phase: StrokePhase::Drive,
            },
        }
    }

    #[test]
    fn envelope_round_trips_through_json() {
        let env = sample_envelope();
        let json = serde_json::to_string(&env).unwrap();
        let back: Envelope<MonitorSample> = serde_json::from_str(&json).unwrap();
        assert_eq!(env, back);
    }

    #[test]
    fn consumers_tolerate_unknown_fields() {
        let mut value = serde_json::to_value(sample_envelope()).unwrap();
        value["added_in_v2"] = serde_json::json!("ignored");
        value["payload"]["future_metric"] = serde_json::json!(1.0);
        let parsed: Result<Envelope<MonitorSample>, _> = serde_json::from_value(value);
        assert!(parsed.is_ok());
    }

    #[test]
    fn dedup_id_is_session_and_seq() {
        let env = sample_envelope();
        assert_eq!(
            env.dedup_id(),
            format!("{}-42", env.session_id)
        );
    }
}
