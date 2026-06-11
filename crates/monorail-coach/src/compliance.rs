//! Post-session compliance scoring (ADR 0009): how well recorded telemetry
//! tracked the plan's segment targets.
//!
//! Segments are walked in order; each claims a window of the session by its
//! extent (time segments by `elapsed_s`, distance segments by `distance_m`),
//! and every monitor sample falling in the window is checked against the
//! segment's split/SPM bands.

use monorail_core::plan::{ComplianceReport, Extent, SegmentCompliance, WorkoutPlan};
use monorail_core::telemetry::MonitorSample;
use monorail_core::SessionId;

/// Score a session's monitor samples against its plan.
///
/// Samples must be in capture order. Samples beyond the planned extent (the
/// athlete kept rowing) are ignored; an unreached segment scores zero with
/// `sample_count = 0`.
pub fn score_compliance(
    plan: &WorkoutPlan,
    session_id: SessionId,
    samples: &[MonitorSample],
) -> ComplianceReport {
    let mut segments = Vec::with_capacity(plan.segments.len());
    let mut window_start_time = 0.0_f64;
    let mut window_start_dist = 0.0_f64;

    for (index, segment) in plan.segments.iter().enumerate() {
        let (in_window, window_end_time, window_end_dist): (Vec<&MonitorSample>, f64, f64) =
            match segment.extent {
                Extent::Time { seconds } => {
                    let end = window_start_time + seconds as f64;
                    let picked = samples
                        .iter()
                        .filter(|s| s.elapsed_s >= window_start_time && s.elapsed_s < end)
                        .collect();
                    (picked, end, window_start_dist)
                }
                Extent::Distance { meters } => {
                    let end = window_start_dist + meters as f64;
                    let picked = samples
                        .iter()
                        .filter(|s| s.distance_m >= window_start_dist && s.distance_m < end)
                        .collect();
                    (picked, window_start_time, end)
                }
            };

        let count = in_window.len() as u32;
        let fraction = |hit: usize| {
            if count == 0 {
                0.0
            } else {
                hit as f32 / count as f32
            }
        };
        let split_hits = in_window
            .iter()
            .filter(|s| segment.split_band.contains(s.split_s_per_500m))
            .count();
        let spm_hits = in_window
            .iter()
            .filter(|s| segment.spm_band.contains(s.stroke_rate_spm))
            .count();

        segments.push(SegmentCompliance {
            segment_index: index as u32,
            intent: segment.intent,
            sample_count: count,
            split_in_band: fraction(split_hits),
            spm_in_band: fraction(spm_hits),
        });

        // A distance segment also consumes the time its samples spanned (and
        // vice versa), so mixed plans keep both cursors moving.
        window_start_time = in_window
            .iter()
            .map(|s| s.elapsed_s)
            .fold(window_end_time, f64::max);
        window_start_dist = in_window
            .iter()
            .map(|s| s.distance_m)
            .fold(window_end_dist, f64::max);
    }

    let total: u32 = segments.iter().map(|s| s.sample_count).sum();
    let weighted = |pick: fn(&SegmentCompliance) -> f32| {
        if total == 0 {
            0.0
        } else {
            segments
                .iter()
                .map(|s| pick(s) * s.sample_count as f32)
                .sum::<f32>()
                / total as f32
        }
    };

    ComplianceReport {
        plan_id: plan.plan_id,
        session_id,
        overall_split_in_band: weighted(|s| s.split_in_band),
        overall_spm_in_band: weighted(|s| s.spm_in_band),
        segments,
    }
}

#[cfg(test)]
mod tests {
    use monorail_core::plan::{Band, Feasibility, Segment, SegmentIntent, WorkoutGoal, Zone};
    use monorail_core::telemetry::StrokePhase;
    use monorail_core::{PlanId, RowerId};
    use uuid::Uuid;

    use super::*;

    fn sample(elapsed_s: f64, split: f32, spm: f32) -> MonitorSample {
        MonitorSample {
            elapsed_s,
            distance_m: elapsed_s * 4.0,
            split_s_per_500m: split,
            stroke_rate_spm: spm,
            power_watts: 200.0,
            heart_rate_bpm: None,
            phase: StrokePhase::Drive,
        }
    }

    fn two_segment_plan() -> WorkoutPlan {
        WorkoutPlan {
            plan_id: PlanId(Uuid::from_u128(1)),
            rower_id: RowerId::new("erg-1").unwrap(),
            goal: WorkoutGoal {
                zone: Zone::Ut2,
                extent: Extent::Time { seconds: 120 },
                target_split_s: 120.0,
                target_spm: 20,
                hr_cap_bpm: None,
            },
            segments: vec![
                Segment {
                    extent: Extent::Time { seconds: 60 },
                    split_band: Band::around(125.0, 2.0),
                    spm_band: Band::around(18.0, 1.0),
                    intent: SegmentIntent::Build,
                },
                Segment {
                    extent: Extent::Time { seconds: 60 },
                    split_band: Band::around(120.0, 2.0),
                    spm_band: Band::around(20.0, 1.0),
                    intent: SegmentIntent::Core,
                },
            ],
            feasibility: Feasibility::Unchecked,
        }
    }

    fn session() -> SessionId {
        SessionId(Uuid::from_u128(9))
    }

    #[test]
    fn perfect_execution_scores_full() {
        let plan = two_segment_plan();
        let samples: Vec<MonitorSample> = (0..120)
            .map(|t| {
                if t < 60 {
                    sample(t as f64, 125.0, 18.0)
                } else {
                    sample(t as f64, 120.0, 20.0)
                }
            })
            .collect();

        let report = score_compliance(&plan, session(), &samples);
        assert_eq!(report.segments.len(), 2);
        for seg in &report.segments {
            assert_eq!(seg.sample_count, 60);
            assert_eq!(seg.split_in_band, 1.0);
            assert_eq!(seg.spm_in_band, 1.0);
        }
        assert_eq!(report.overall_split_in_band, 1.0);
    }

    #[test]
    fn samples_score_against_their_own_segment_window() {
        let plan = two_segment_plan();
        // Athlete rows the CORE pace from the start: out of band during the
        // build segment, in band during core.
        let samples: Vec<MonitorSample> = (0..120).map(|t| sample(t as f64, 120.0, 20.0)).collect();

        let report = score_compliance(&plan, session(), &samples);
        assert_eq!(report.segments[0].split_in_band, 0.0);
        assert_eq!(report.segments[1].split_in_band, 1.0);
        assert!((report.overall_split_in_band - 0.5).abs() < 0.01);
    }

    #[test]
    fn short_session_leaves_unreached_segment_empty() {
        let plan = two_segment_plan();
        // Stopped after 30 s.
        let samples: Vec<MonitorSample> = (0..30).map(|t| sample(t as f64, 125.0, 18.0)).collect();

        let report = score_compliance(&plan, session(), &samples);
        assert_eq!(report.segments[0].sample_count, 30);
        assert_eq!(report.segments[1].sample_count, 0);
        assert_eq!(report.segments[1].split_in_band, 0.0);
    }

    #[test]
    fn distance_segment_scores_by_meters() {
        let mut plan = two_segment_plan();
        plan.segments = vec![Segment {
            extent: Extent::Distance { meters: 200 },
            split_band: Band::around(120.0, 2.0),
            spm_band: Band::around(20.0, 1.0),
            intent: SegmentIntent::Core,
        }];
        // 4 m/s ⇒ 200 m reached at t = 50; later samples are out of extent.
        let samples: Vec<MonitorSample> = (0..80).map(|t| sample(t as f64, 120.0, 20.0)).collect();

        let report = score_compliance(&plan, session(), &samples);
        assert_eq!(report.segments[0].sample_count, 50);
        assert_eq!(report.segments[0].split_in_band, 1.0);
    }

    #[test]
    fn empty_session_scores_zero_not_nan() {
        let report = score_compliance(&two_segment_plan(), session(), &[]);
        assert_eq!(report.overall_split_in_band, 0.0);
        assert_eq!(report.overall_spm_in_band, 0.0);
    }
}
