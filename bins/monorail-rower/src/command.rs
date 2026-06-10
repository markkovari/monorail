//! Pi-side command handler (ADR 0010): subscribes to this rower's plan
//! subject and answers request/reply commands.
//!
//! With the fake source there is no PM5 to program, so a received plan is
//! honestly acked as `Programmed::Approximate` and its core-segment targets
//! are applied to the simulator instead. The real CSAFE write path replaces
//! the apply step; the subscribe/validate/reply structure stays.

use async_nats::Client;
use futures::StreamExt;
use monorail_core::plan::{Extent, SegmentIntent, WorkoutPlan};
use monorail_core::wire::{Command, CommandReply, NackReason, Programmed};
use monorail_core::RowerId;
use monorail_stream::subjects;

/// Targets extracted from a plan for the fake source.
#[derive(Debug, Clone, PartialEq)]
pub struct AppliedPlan {
    pub plan: WorkoutPlan,
    pub target_split_s: f32,
    pub target_spm: f32,
    pub duration_s: u32,
}

/// Pull the core segment's midpoints (or the goal's targets) out of a plan.
pub fn apply_targets(plan: &WorkoutPlan) -> AppliedPlan {
    let core = plan
        .segments
        .iter()
        .find(|s| s.intent == SegmentIntent::Core);
    let (split, spm) = core
        .map(|s| {
            (
                (s.split_band.low + s.split_band.high) / 2.0,
                (s.spm_band.low + s.spm_band.high) / 2.0,
            )
        })
        .unwrap_or((plan.goal.target_split_s, plan.goal.target_spm as f32));
    let duration_s = match plan.total_seconds() {
        Some(total) if total > 0 => total,
        // Distance segments or no segments: derive from the goal.
        _ => match plan.goal.extent {
            Extent::Time { seconds } => seconds,
            // Rough conversion for distance goals.
            Extent::Distance { meters } => {
                (meters as f32 / 500.0 * plan.goal.target_split_s) as u32
            }
        },
    };
    AppliedPlan {
        plan: plan.clone(),
        target_split_s: split,
        target_spm: spm,
        duration_s,
    }
}

/// Wait for one valid `ProgramWorkout` command, ack it, return the applied
/// targets. Invalid or mismatched commands are nacked and waiting continues.
pub async fn wait_for_plan(client: &Client, rower: &RowerId) -> anyhow::Result<AppliedPlan> {
    let subject = subjects::command_plan(rower);
    let mut subscription = client.subscribe(subject.clone()).await?;
    tracing::info!(%subject, "waiting for plan push");

    while let Some(message) = subscription.next().await {
        let Some(reply_to) = message.reply.clone() else {
            tracing::warn!("command without reply subject, ignoring");
            continue;
        };

        let reply = match serde_json::from_slice::<Command>(&message.payload) {
            Ok(Command::ProgramWorkout { plan }) if plan.rower_id == *rower => {
                let applied = apply_targets(&plan);
                let reply = CommandReply::Ack {
                    plan_id: Some(plan.plan_id),
                    programmed: Some(Programmed::Approximate {
                        reason: "fake PM5: core-segment targets applied to simulator".to_string(),
                    }),
                };
                client
                    .publish(reply_to, serde_json::to_vec(&reply)?.into())
                    .await?;
                tracing::info!(plan_id = %applied.plan.plan_id, "plan accepted");
                return Ok(applied);
            }
            Ok(Command::ProgramWorkout { plan }) => {
                tracing::warn!(got = %plan.rower_id, expected = %rower, "plan for wrong rower");
                CommandReply::Nack {
                    reason: NackReason::InvalidCommand,
                    detail: Some(format!("plan addressed to {}", plan.rower_id)),
                }
            }
            Ok(Command::ClearWorkout) => CommandReply::Ack {
                plan_id: None,
                programmed: None,
            },
            Err(error) => CommandReply::Nack {
                reason: NackReason::InvalidCommand,
                detail: Some(error.to_string()),
            },
        };
        client
            .publish(reply_to, serde_json::to_vec(&reply)?.into())
            .await?;
    }
    anyhow::bail!("command subscription ended without a plan")
}

#[cfg(test)]
mod tests {
    use monorail_core::plan::{Band, Feasibility, Segment, WorkoutGoal, Zone};
    use monorail_core::PlanId;
    use uuid::Uuid;

    use super::*;

    fn plan_with_core() -> WorkoutPlan {
        WorkoutPlan {
            plan_id: PlanId(Uuid::from_u128(1)),
            rower_id: RowerId::new("erg-1").unwrap(),
            goal: WorkoutGoal {
                zone: Zone::Ut2,
                extent: Extent::Time { seconds: 2400 },
                target_split_s: 120.0,
                target_spm: 20,
                hr_cap_bpm: None,
            },
            segments: vec![
                Segment {
                    extent: Extent::Time { seconds: 600 },
                    split_band: Band::around(125.0, 2.0),
                    spm_band: Band::around(18.0, 1.0),
                    intent: SegmentIntent::Build,
                },
                Segment {
                    extent: Extent::Time { seconds: 1800 },
                    split_band: Band::around(120.0, 2.0),
                    spm_band: Band::around(20.0, 1.0),
                    intent: SegmentIntent::Core,
                },
            ],
            feasibility: Feasibility::Unchecked,
        }
    }

    #[test]
    fn applies_core_segment_midpoints() {
        let applied = apply_targets(&plan_with_core());
        assert_eq!(applied.target_split_s, 120.0);
        assert_eq!(applied.target_spm, 20.0);
        assert_eq!(applied.duration_s, 2400);
    }

    #[test]
    fn falls_back_to_goal_targets_without_core_segment() {
        let mut plan = plan_with_core();
        plan.segments.clear();
        let applied = apply_targets(&plan);
        assert_eq!(applied.target_split_s, 120.0);
        assert_eq!(applied.target_spm, 20.0);
        // No segments: duration comes from the goal extent.
        assert_eq!(applied.duration_s, 2400);
    }
}
