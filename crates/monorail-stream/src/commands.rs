//! Command plane (ADR 0010): core NATS request/reply, deliberately not
//! JetStream — a command is only meaningful against the current PM5 state,
//! so durability is explicitly unwanted.

use std::time::Duration;

use async_nats::Client;
use monorail_core::plan::WorkoutPlan;
use monorail_core::wire::{Command, CommandReply};
use thiserror::Error;

use crate::subjects;

/// How long the server waits for the Pi to ack/nack a command.
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("serialize failed: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("request failed: {0}")]
    Request(#[from] async_nats::RequestError),
    #[error("timed out after {COMMAND_TIMEOUT:?} (rower offline?)")]
    Timeout,
}

/// Push a plan to its rower's Pi and await the ack/nack.
pub async fn push_plan(client: &Client, plan: &WorkoutPlan) -> Result<CommandReply, CommandError> {
    let subject = subjects::command_plan(&plan.rower_id);
    let payload = serde_json::to_vec(&Command::ProgramWorkout { plan: plan.clone() })?;

    let response = tokio::time::timeout(COMMAND_TIMEOUT, client.request(subject, payload.into()))
        .await
        .map_err(|_| CommandError::Timeout)??;
    Ok(serde_json::from_slice(&response.payload)?)
}
