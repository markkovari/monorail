//! Workout plan generation and compliance scoring (ADR 0009).
//!
//! Generation = templates × personalization: curated templates keyed by
//! zone/goal shape, then a feasibility pass against fitted athlete models
//! (`monorail-predict::FeasibilityJudge`). Compliance scoring joins a plan's
//! segments against recorded telemetry by `plan_id` after the session.

use monorail_core::plan::{
    Band, Extent, Feasibility, Segment, SegmentIntent, WorkoutGoal, WorkoutPlan,
};
use monorail_core::{PlanId, RowerId};
use monorail_predict::FeasibilityJudge;
use uuid::Uuid;

/// Tolerances applied around segment targets.
const SPLIT_TOLERANCE_S: f32 = 2.0;
const SPM_TOLERANCE: f32 = 1.0;

/// Generate a plan for a goal.
///
/// Currently implements the built-in steady-state template
/// (25% build / 50% core / 25% push). The template library as data files
/// (RON/JSON) replaces this hardcoding per ADR 0009.
pub fn generate_plan(
    rower_id: RowerId,
    goal: WorkoutGoal,
    judge: Option<&dyn FeasibilityJudge>,
) -> WorkoutPlan {
    let segments = match goal.extent {
        Extent::Time { seconds } => steady_time_segments(&goal, seconds),
        // Distance goals: single core segment until distance templates land.
        Extent::Distance { .. } => vec![Segment {
            extent: goal.extent,
            split_band: Band::around(goal.target_split_s, SPLIT_TOLERANCE_S),
            spm_band: Band::around(goal.target_spm as f32, SPM_TOLERANCE),
            intent: SegmentIntent::Core,
        }],
    };

    let feasibility = match judge {
        None => Feasibility::Unchecked,
        Some(judge) => check_feasibility(&goal, judge),
    };

    WorkoutPlan {
        plan_id: PlanId(Uuid::new_v4()),
        rower_id,
        goal,
        segments,
        feasibility,
    }
}

/// Steady-state template: 25% build (easier split, lower rate), 50% core at
/// goal, 25% push (slightly faster, slightly higher rate).
fn steady_time_segments(goal: &WorkoutGoal, total_s: u32) -> Vec<Segment> {
    let build_s = total_s / 4;
    let push_s = total_s / 4;
    let core_s = total_s - build_s - push_s;
    let spm = goal.target_spm as f32;

    vec![
        Segment {
            extent: Extent::Time { seconds: build_s },
            split_band: Band::around(goal.target_split_s + 5.0, SPLIT_TOLERANCE_S),
            spm_band: Band::around(spm - 2.0, SPM_TOLERANCE),
            intent: SegmentIntent::Build,
        },
        Segment {
            extent: Extent::Time { seconds: core_s },
            split_band: Band::around(goal.target_split_s, SPLIT_TOLERANCE_S),
            spm_band: Band::around(spm, SPM_TOLERANCE),
            intent: SegmentIntent::Core,
        },
        Segment {
            extent: Extent::Time { seconds: push_s },
            split_band: Band::around(goal.target_split_s - 2.0, SPLIT_TOLERANCE_S),
            spm_band: Band::around(spm + 2.0, SPM_TOLERANCE),
            intent: SegmentIntent::Push,
        },
    ]
}

fn check_feasibility(goal: &WorkoutGoal, judge: &dyn FeasibilityJudge) -> Feasibility {
    let duration_s = match goal.extent {
        Extent::Time { seconds } => seconds as f64,
        // Rough duration estimate from target pace.
        Extent::Distance { meters } => meters as f64 / 500.0 * goal.target_split_s as f64,
    };
    match judge.sustainable_split_s(duration_s) {
        None => Feasibility::Unchecked,
        Some(sustainable) if goal.target_split_s as f64 >= sustainable => Feasibility::Ok,
        Some(sustainable) => Feasibility::Warning {
            reason: format!(
                "target split {:.1}s/500m is faster than predicted sustainable {:.1}s/500m for {:.0}s",
                goal.target_split_s, sustainable, duration_s
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use monorail_core::plan::Zone;

    use super::*;

    fn ut2_goal() -> WorkoutGoal {
        WorkoutGoal {
            zone: Zone::Ut2,
            extent: Extent::Time { seconds: 40 * 60 },
            target_split_s: 120.0,
            target_spm: 20,
            hr_cap_bpm: None,
        }
    }

    fn rower() -> RowerId {
        RowerId::new("erg-1").unwrap()
    }

    #[test]
    fn steady_template_splits_40min_into_10_20_10() {
        let plan = generate_plan(rower(), ut2_goal(), None);
        let durations: Vec<u32> = plan
            .segments
            .iter()
            .map(|s| match s.extent {
                Extent::Time { seconds } => seconds,
                _ => panic!("time goal must yield time segments"),
            })
            .collect();
        assert_eq!(durations, vec![600, 1200, 600]);
        assert_eq!(plan.total_seconds(), Some(2400));
        assert_eq!(plan.feasibility, Feasibility::Unchecked);

        let intents: Vec<_> = plan.segments.iter().map(|s| s.intent).collect();
        assert_eq!(
            intents,
            vec![
                SegmentIntent::Build,
                SegmentIntent::Core,
                SegmentIntent::Push
            ]
        );
        // Build eases off the target, push goes beyond it.
        assert!(plan.segments[0].split_band.low > plan.segments[1].split_band.low);
        assert!(plan.segments[2].split_band.high < plan.segments[1].split_band.high);
    }

    struct FixedJudge(f64);
    impl FeasibilityJudge for FixedJudge {
        fn sustainable_split_s(&self, _duration_s: f64) -> Option<f64> {
            Some(self.0)
        }
    }

    /// Records the duration the generator asked about.
    struct RecordingJudge {
        asked_duration_s: std::cell::Cell<f64>,
    }
    impl FeasibilityJudge for RecordingJudge {
        fn sustainable_split_s(&self, duration_s: f64) -> Option<f64> {
            self.asked_duration_s.set(duration_s);
            Some(0.0) // never sustainable; value irrelevant here
        }
    }

    /// Judge with no opinion (e.g. not enough history to fit a model).
    struct SilentJudge;
    impl FeasibilityJudge for SilentJudge {
        fn sustainable_split_s(&self, _duration_s: f64) -> Option<f64> {
            None
        }
    }

    fn assert_bands_well_formed(plan: &WorkoutPlan) {
        for segment in &plan.segments {
            assert!(
                segment.split_band.low <= segment.split_band.high,
                "split band inverted: {:?}",
                segment.split_band
            );
            assert!(
                segment.spm_band.low <= segment.spm_band.high,
                "spm band inverted: {:?}",
                segment.spm_band
            );
        }
    }

    #[test]
    fn unsustainable_goal_yields_warning_not_rejection() {
        // Athlete can only sustain 125s/500m; goal asks for 120.
        let plan = generate_plan(rower(), ut2_goal(), Some(&FixedJudge(125.0)));
        assert!(matches!(plan.feasibility, Feasibility::Warning { .. }));
        // Goal preserved as requested.
        assert_eq!(plan.goal.target_split_s, 120.0);
    }

    #[test]
    fn sustainable_goal_is_ok() {
        let plan = generate_plan(rower(), ut2_goal(), Some(&FixedJudge(115.0)));
        assert_eq!(plan.feasibility, Feasibility::Ok);
    }

    #[test]
    fn split_exactly_at_sustainable_is_ok() {
        // Boundary of the >= comparison: equal means feasible.
        let plan = generate_plan(rower(), ut2_goal(), Some(&FixedJudge(120.0)));
        assert_eq!(plan.feasibility, Feasibility::Ok);
    }

    #[test]
    fn judge_without_opinion_leaves_plan_unchecked() {
        let plan = generate_plan(rower(), ut2_goal(), Some(&SilentJudge));
        assert_eq!(plan.feasibility, Feasibility::Unchecked);
    }

    #[test]
    fn warning_message_names_both_splits() {
        let plan = generate_plan(rower(), ut2_goal(), Some(&FixedJudge(125.0)));
        let Feasibility::Warning { reason } = &plan.feasibility else {
            panic!("expected warning, got {:?}", plan.feasibility);
        };
        assert!(reason.contains("120.0"), "target split missing: {reason}");
        assert!(
            reason.contains("125.0"),
            "sustainable split missing: {reason}"
        );
    }

    #[test]
    fn odd_duration_segments_sum_exactly() {
        // 41 minutes: total/4 truncates, core absorbs the remainder.
        let goal = WorkoutGoal {
            extent: Extent::Time { seconds: 41 * 60 },
            ..ut2_goal()
        };
        let plan = generate_plan(rower(), goal, None);
        assert_eq!(plan.total_seconds(), Some(41 * 60));
        assert_bands_well_formed(&plan);
    }

    #[test]
    fn degenerate_durations_do_not_panic() {
        for seconds in [0, 1, 2, 3] {
            let goal = WorkoutGoal {
                extent: Extent::Time { seconds },
                ..ut2_goal()
            };
            let plan = generate_plan(rower(), goal, None);
            assert_eq!(plan.total_seconds(), Some(seconds));
            assert_bands_well_formed(&plan);
        }
    }

    #[test]
    fn distance_goal_yields_single_core_segment_with_goal_extent() {
        let goal = WorkoutGoal {
            extent: Extent::Distance { meters: 10_000 },
            ..ut2_goal()
        };
        let plan = generate_plan(rower(), goal.clone(), None);
        assert_eq!(plan.segments.len(), 1);
        let segment = &plan.segments[0];
        assert_eq!(segment.intent, SegmentIntent::Core);
        assert_eq!(segment.extent, goal.extent);
        assert!(segment.split_band.contains(goal.target_split_s));
        assert!(segment.spm_band.contains(goal.target_spm as f32));
        assert_bands_well_formed(&plan);
    }

    #[test]
    fn time_goal_feasibility_judged_over_goal_duration() {
        let judge = RecordingJudge {
            asked_duration_s: std::cell::Cell::new(f64::NAN),
        };
        generate_plan(rower(), ut2_goal(), Some(&judge));
        assert_eq!(judge.asked_duration_s.get(), 2400.0);
    }

    #[test]
    fn distance_goal_feasibility_uses_pace_estimated_duration() {
        let judge = RecordingJudge {
            asked_duration_s: std::cell::Cell::new(f64::NAN),
        };
        let goal = WorkoutGoal {
            extent: Extent::Distance { meters: 10_000 },
            ..ut2_goal()
        };
        generate_plan(rower(), goal, Some(&judge));
        // 10000 m / 500 m * 120 s = 2400 s
        assert_eq!(judge.asked_duration_s.get(), 2400.0);
    }
}
