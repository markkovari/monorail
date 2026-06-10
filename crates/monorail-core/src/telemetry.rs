//! Telemetry payloads published by the Pi (ADRs 0003/0004/0005).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::PlanId;

/// One completed stroke, published on `monorail.telemetry.<rower>.stroke`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrokeSample {
    pub stroke_number: u32,
    pub drive_time_ms: u32,
    pub recovery_time_ms: u32,
    pub stroke_rate_spm: f32,
    pub power_watts: f32,
    /// Pace in seconds per 500 m at the end of this stroke.
    pub split_s_per_500m: f32,
    /// Cumulative distance since workout start.
    pub distance_m: f64,
    /// Drive length reported by the PM5, when available.
    pub drive_length_m: Option<f32>,
}

/// PM5 stroke-cycle phase, from the monitor's stroke state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrokePhase {
    Idle,
    Drive,
    Dwell,
    Recovery,
}

/// Monitor snapshot from the fast poll loop (~10 Hz), published on
/// `monorail.telemetry.<rower>.monitor`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorSample {
    pub elapsed_s: f64,
    pub distance_m: f64,
    pub split_s_per_500m: f32,
    pub stroke_rate_spm: f32,
    pub power_watts: f32,
    pub heart_rate_bpm: Option<u16>,
    pub phase: StrokePhase,
}

/// Workout lifecycle events, published on `monorail.workout.<rower>.event`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum WorkoutEvent {
    Started {
        ts: DateTime<Utc>,
        /// Set when the session executes a pushed plan (ADR 0010 stamps it).
        plan_id: Option<PlanId>,
    },
    IntervalBoundary {
        ts: DateTime<Utc>,
        interval_index: u32,
    },
    Ended {
        ts: DateTime<Utc>,
        summary: WorkoutSummary,
    },
}

/// Totals as reported by the PM5 at workout end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkoutSummary {
    pub duration_s: f64,
    pub distance_m: f64,
    pub avg_split_s_per_500m: f32,
    pub avg_stroke_rate_spm: f32,
    pub avg_power_watts: f32,
    pub stroke_count: u32,
    pub avg_heart_rate_bpm: Option<u16>,
}
