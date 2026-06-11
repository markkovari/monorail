//! Workout goals, plans, and segments (ADR 0009).

use serde::{Deserialize, Serialize};

use crate::{PlanId, RowerId};

/// Standard rowing intensity zones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Zone {
    /// Utilization 2 — light aerobic, ~55-70% max HR.
    Ut2,
    /// Utilization 1 — moderate aerobic.
    Ut1,
    /// Anaerobic threshold.
    At,
    /// Oxygen transportation.
    Tr,
    /// Anaerobic.
    An,
}

/// A workout's size: rowed for time or for distance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Extent {
    Time { seconds: u32 },
    Distance { meters: u32 },
}

/// What the athlete asked for; input to plan generation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkoutGoal {
    pub zone: Zone,
    pub extent: Extent,
    /// Target pace, seconds per 500 m.
    pub target_split_s: f32,
    pub target_spm: u8,
    /// Optional heart-rate ceiling.
    pub hr_cap_bpm: Option<u16>,
}

/// Display label conveying what a segment is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentIntent {
    Warmup,
    Build,
    Core,
    Push,
    Recover,
    Cooldown,
}

/// Inclusive target band; `low`/`high` in the unit of the field it targets.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Band {
    pub low: f32,
    pub high: f32,
}

impl Band {
    pub fn around(center: f32, tolerance: f32) -> Self {
        Self {
            low: center - tolerance,
            high: center + tolerance,
        }
    }

    pub fn contains(&self, value: f32) -> bool {
        (self.low..=self.high).contains(&value)
    }
}

/// One planned slice of a workout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub extent: Extent,
    /// Seconds per 500 m.
    pub split_band: Band,
    /// Strokes per minute. Advisory only — the PM5 cannot enforce stroke
    /// rate (ADR 0010); adherence is measured in compliance scoring.
    pub spm_band: Band,
    pub intent: SegmentIntent,
}

/// Plan feasibility as judged against the athlete's fitted models.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "feasibility", rename_all = "snake_case")]
pub enum Feasibility {
    /// Not yet checked against athlete models.
    Unchecked,
    Ok,
    /// Goal kept as requested but flagged as beyond predicted capability.
    Warning {
        reason: String,
    },
}

/// API request body for plan generation (`POST /api/v1/plans`, ADR 0011).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanRequest {
    pub rower_id: RowerId,
    pub goal: WorkoutGoal,
}

/// How one executed segment matched its plan targets (ADR 0009).
/// In-band values are fractions of samples inside the target band, 0.0–1.0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SegmentCompliance {
    pub segment_index: u32,
    pub intent: SegmentIntent,
    pub sample_count: u32,
    pub split_in_band: f32,
    /// SPM adherence is ours to measure — the PM5 cannot enforce stroke
    /// rate (ADR 0010).
    pub spm_in_band: f32,
}

/// Post-session adherence of recorded telemetry to a plan (ADR 0009).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComplianceReport {
    pub plan_id: PlanId,
    pub session_id: crate::SessionId,
    pub segments: Vec<SegmentCompliance>,
    /// Sample-weighted averages across segments.
    pub overall_split_in_band: f32,
    pub overall_spm_in_band: f32,
}

/// A generated, pushable workout plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkoutPlan {
    pub plan_id: PlanId,
    pub rower_id: RowerId,
    pub goal: WorkoutGoal,
    pub segments: Vec<Segment>,
    pub feasibility: Feasibility,
}

impl WorkoutPlan {
    /// Total planned time, if every segment is time-based.
    pub fn total_seconds(&self) -> Option<u32> {
        self.segments
            .iter()
            .map(|s| match s.extent {
                Extent::Time { seconds } => Some(seconds),
                Extent::Distance { .. } => None,
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_contains_is_inclusive() {
        let b = Band::around(120.0, 2.0);
        assert!(b.contains(118.0));
        assert!(b.contains(122.0));
        assert!(!b.contains(122.5));
    }
}
