//! NATS subject schema and JetStream configuration (ADRs 0004/0010).
//!
//! Subjects are built here and nowhere else, so the hierarchy has a single
//! point of truth.

pub mod commands;
pub mod jetstream;
pub mod subjects;

/// JetStream stream capturing durable telemetry.
pub const STREAM_NAME: &str = "MONORAIL";

/// Subjects captured by the [`STREAM_NAME`] stream.
///
/// Deliberately telemetry + workout events only: the command plane
/// (`monorail.command.>`, `monorail.status.>`) uses core NATS request/reply
/// and must NOT be persisted — replaying a stale "program workout" command
/// is wrong (ADR 0010).
pub const STREAM_SUBJECTS: &[&str] = &["monorail.telemetry.>", "monorail.workout.>"];
