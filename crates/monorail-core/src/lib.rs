//! Domain and wire types for monorail.
//!
//! Everything that crosses a process boundary (NATS payloads, API bodies,
//! DuckDB rows) is defined here, per ADR 0005. This crate must stay free of
//! I/O dependencies — `serde`, `uuid`, `chrono` only — so it compiles for
//! the Pi target and for wasm32 (the Leptos UI) alike.

pub mod api;
pub mod ids;
pub mod metrics;
pub mod plan;
pub mod telemetry;
pub mod wire;

pub use ids::{PlanId, RowerId, SessionId};
