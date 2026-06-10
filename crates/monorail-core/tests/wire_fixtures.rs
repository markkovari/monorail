//! Schema-compatibility tests against checked-in wire fixtures (ADR 0005).
//!
//! The fixtures in `tests/fixtures/` are the de-facto schema documentation.
//! Each test deserializes a fixture into its typed struct (backward read
//! compatibility) and re-serializes it, comparing as `serde_json::Value`
//! (field-order-insensitive). A failure here means the wire schema changed:
//! either fix the regression or consciously bump `WIRE_VERSION` and update
//! the fixture.

use serde::de::DeserializeOwned;
use serde::Serialize;

use monorail_core::plan::WorkoutPlan;
use monorail_core::telemetry::{MonitorSample, StrokeSample, WorkoutEvent};
use monorail_core::wire::{Command, CommandReply, Envelope};

fn check_fixture<T: Serialize + DeserializeOwned>(name: &str) {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let expected: serde_json::Value = serde_json::from_str(&raw).expect("fixture is valid JSON");

    let typed: T = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("fixture {name} no longer deserializes: {e}"));
    let actual = serde_json::to_value(&typed).expect("re-serialize");

    assert_eq!(
        actual, expected,
        "wire schema drifted for fixture {name}; fix the regression or bump WIRE_VERSION and update the fixture"
    );
}

#[test]
fn envelope_monitor_sample() {
    check_fixture::<Envelope<MonitorSample>>("envelope_monitor_sample");
}

#[test]
fn envelope_stroke_sample() {
    check_fixture::<Envelope<StrokeSample>>("envelope_stroke_sample");
}

#[test]
fn workout_event_started() {
    check_fixture::<WorkoutEvent>("workout_event_started");
}

#[test]
fn workout_event_ended() {
    check_fixture::<WorkoutEvent>("workout_event_ended");
}

#[test]
fn workout_plan() {
    check_fixture::<WorkoutPlan>("workout_plan");
}

#[test]
fn command_program_workout() {
    check_fixture::<Command>("command_program_workout");
}

#[test]
fn command_reply_nack() {
    check_fixture::<CommandReply>("command_reply_nack");
}
