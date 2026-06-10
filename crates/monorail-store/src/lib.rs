//! DuckDB aggregation store (ADR 0006).
//!
//! This crate is the only one allowed to depend on `duckdb` or contain SQL.
//! It owns:
//! - schema + numbered migrations (applied at startup, tracked in
//!   `schema_migrations`);
//! - idempotent ingestion keyed on `(session_id, seq)`;
//! - canonical aggregation queries exposed as typed functions, so
//!   `monorail-predict`/`monorail-coach` consume feature frames, not SQL.
//!
//! Raw tables (`stroke`, `monitor_sample`, `workout_event`) are append-only;
//! derived tables (`session_summary`, `interval_split`, `daily_load`,
//! `plan*`) are always rebuildable from raw.

use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("migration failed at version {0}")]
    Migration(u32),
    #[error("database error: {0}")]
    Database(String),
}

/// Configuration for opening the store.
#[derive(Debug, Clone)]
pub struct StoreConfig {
    /// Path to the DuckDB database file.
    pub db_path: PathBuf,
}

/// Numbered migrations, applied in order at startup.
/// First entries land with the ingest implementation.
pub const MIGRATIONS: &[(u32, &str)] = &[];
