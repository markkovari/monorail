//! HTTP API response rows (ADR 0011): produced by the sink's queries,
//! consumed verbatim by the Leptos UI. They live here — not in the store —
//! because anything crossing the wire belongs to core (ADR 0005), and the
//! UI must never depend on DuckDB-bearing crates.

use serde::{Deserialize, Serialize};

/// One row of the per-session overview (`GET /api/v1/sessions`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummaryRow {
    pub session_id: String,
    pub rower_id: String,
    pub started_at: String,
    pub monitor_samples: u64,
    pub strokes: u64,
    pub last_distance_m: Option<f64>,
    pub duration_s: Option<f64>,
    pub avg_power_watts: Option<f64>,
    /// Calories as a PM5 would show them (175 lb reference, ADR 0012).
    pub kcal_pm: Option<f64>,
    /// Weight-adjusted calories; `null` until an athlete weight is set.
    pub kcal_adjusted: Option<f64>,
}

/// One row of the plan overview (`GET /api/v1/plans`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanRow {
    pub plan_id: String,
    pub rower_id: String,
    pub created_at: String,
    pub status: String,
}

/// One stored per-segment compliance row
/// (`GET /api/v1/sessions/{id}/compliance`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComplianceRow {
    pub plan_id: String,
    pub segment_index: u32,
    pub intent: String,
    pub sample_count: u32,
    pub split_in_band: f32,
    pub spm_in_band: f32,
}

/// One imported Concept2 Logbook result (`GET /api/v1/logbook`, ADR 0013).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogbookRow {
    pub id: u64,
    pub date: String,
    pub distance_m: Option<f64>,
    pub duration_s: Option<f64>,
    pub calories_total: Option<u32>,
    pub stroke_rate: Option<u32>,
    pub raw: String,
}
