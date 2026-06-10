//! Subject builders. `RowerId` is validated at construction (token-safe),
//! so interpolation here cannot produce malformed subjects.

use monorail_core::RowerId;

/// `monorail.telemetry.<rower>.stroke` — per-stroke samples (JetStream).
pub fn telemetry_stroke(rower: &RowerId) -> String {
    format!("monorail.telemetry.{rower}.stroke")
}

/// `monorail.telemetry.<rower>.monitor` — ~10 Hz snapshots (JetStream).
pub fn telemetry_monitor(rower: &RowerId) -> String {
    format!("monorail.telemetry.{rower}.monitor")
}

/// `monorail.workout.<rower>.event` — lifecycle events (JetStream).
pub fn workout_event(rower: &RowerId) -> String {
    format!("monorail.workout.{rower}.event")
}

/// `monorail.command.<rower>.plan` — plan delivery, request/reply (no JS).
pub fn command_plan(rower: &RowerId) -> String {
    format!("monorail.command.{rower}.plan")
}

/// `monorail.command.<rower>.control` — cancel/clear, request/reply (no JS).
pub fn command_control(rower: &RowerId) -> String {
    format!("monorail.command.{rower}.control")
}

/// `monorail.status.<rower>` — Pi liveness/PM5 state (no JS).
pub fn status(rower: &RowerId) -> String {
    format!("monorail.status.{rower}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::STREAM_SUBJECTS;

    fn rower() -> RowerId {
        RowerId::new("erg-1").unwrap()
    }

    #[test]
    fn subjects_match_adr_hierarchy() {
        assert_eq!(
            telemetry_stroke(&rower()),
            "monorail.telemetry.erg-1.stroke"
        );
        assert_eq!(
            telemetry_monitor(&rower()),
            "monorail.telemetry.erg-1.monitor"
        );
        assert_eq!(workout_event(&rower()), "monorail.workout.erg-1.event");
        assert_eq!(command_plan(&rower()), "monorail.command.erg-1.plan");
        assert_eq!(command_control(&rower()), "monorail.command.erg-1.control");
        assert_eq!(status(&rower()), "monorail.status.erg-1");
    }

    #[test]
    fn command_subjects_are_outside_stream_capture() {
        // Commands must never be captured by JetStream (ADR 0010).
        let captured = |subject: &str| {
            STREAM_SUBJECTS
                .iter()
                .any(|prefix| subject.starts_with(prefix.trim_end_matches('>')))
        };
        assert!(captured(&telemetry_stroke(&rower())));
        assert!(captured(&workout_event(&rower())));
        assert!(!captured(&command_plan(&rower())));
        assert!(!captured(&status(&rower())));
    }
}
