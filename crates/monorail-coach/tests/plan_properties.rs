//! Property tests for plan generation invariants (ADR 0009).

use monorail_coach::generate_plan;
use monorail_core::plan::{Extent, SegmentIntent, WorkoutGoal, Zone};
use monorail_core::RowerId;
use proptest::prelude::*;

fn rower() -> RowerId {
    RowerId::new("erg-1").unwrap()
}

fn time_goal(seconds: u32, split: f32, spm: u8) -> WorkoutGoal {
    WorkoutGoal {
        zone: Zone::Ut2,
        extent: Extent::Time { seconds },
        target_split_s: split,
        target_spm: spm,
        hr_cap_bpm: None,
    }
}

proptest! {
    #[test]
    fn time_plans_preserve_duration_and_shape(
        seconds in 60u32..=14_400,
        split in 90.0f32..=180.0,
        spm in 16u8..=36,
    ) {
        let plan = generate_plan(rower(), time_goal(seconds, split, spm), None);

        // Durations sum exactly to the goal.
        prop_assert_eq!(plan.total_seconds(), Some(seconds));

        // Shape is build -> core -> push.
        let intents: Vec<_> = plan.segments.iter().map(|s| s.intent).collect();
        prop_assert_eq!(
            intents,
            vec![SegmentIntent::Build, SegmentIntent::Core, SegmentIntent::Push]
        );

        // Build eases off the target split, push goes beyond the core.
        let (build, core, push) = (&plan.segments[0], &plan.segments[1], &plan.segments[2]);
        prop_assert!(build.split_band.low > core.split_band.low);
        prop_assert!(push.split_band.high < core.split_band.high);
        prop_assert!(build.spm_band.high < core.spm_band.high);
        prop_assert!(push.spm_band.low > core.spm_band.low);

        // All bands well-formed.
        for segment in &plan.segments {
            prop_assert!(segment.split_band.low <= segment.split_band.high);
            prop_assert!(segment.spm_band.low <= segment.spm_band.high);
        }
    }

    #[test]
    fn distance_plans_are_single_core_segment(
        meters in 500u32..=42_195,
        split in 90.0f32..=180.0,
        spm in 16u8..=36,
    ) {
        let goal = WorkoutGoal {
            zone: Zone::Ut2,
            extent: Extent::Distance { meters },
            target_split_s: split,
            target_spm: spm,
            hr_cap_bpm: None,
        };
        let plan = generate_plan(rower(), goal.clone(), None);

        prop_assert_eq!(plan.segments.len(), 1);
        prop_assert_eq!(plan.segments[0].intent, SegmentIntent::Core);
        prop_assert_eq!(plan.segments[0].extent, goal.extent);
        prop_assert!(plan.segments[0].split_band.contains(split));
    }
}
