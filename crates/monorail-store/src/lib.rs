//! DuckDB aggregation store (ADR 0006).
//!
//! This crate is the only one allowed to depend on `duckdb` or contain SQL.
//! Raw tables are append-only and ingested idempotently keyed on
//! `(session_id, seq)`, so JetStream's at-least-once delivery is safe to
//! replay. Single-writer rule: exactly one process opens the database
//! read-write (the sink).

mod migrations;

use std::path::Path;

use duckdb::{params, Connection};
use monorail_core::telemetry::{MonitorSample, StrokeSample, WorkoutEvent};
use monorail_core::wire::Envelope;
use thiserror::Error;

pub use migrations::MIGRATIONS;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Database(#[from] duckdb::Error),
    #[error("serialize error: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Handle to the DuckDB database. Not `Sync`: keep it on one writer thread
/// or behind a mutex (single-writer rule, ADR 0006).
pub struct Store {
    conn: Connection,
}

/// One row of the per-session overview query.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionSummaryRow {
    pub session_id: String,
    pub rower_id: String,
    pub started_at: String,
    pub monitor_samples: u64,
    pub strokes: u64,
    pub last_distance_m: Option<f64>,
}

impl Store {
    /// Open (creating if needed) and migrate the database at `path`.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        Self::from_connection(Connection::open(path)?)
    }

    /// In-memory database for tests.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self, StoreError> {
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    /// Apply pending migrations in order, tracked in `schema_migrations`.
    fn migrate(&self) -> Result<(), StoreError> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version    INTEGER PRIMARY KEY,
                applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
            );",
        )?;
        let current: u32 = self.conn.query_row(
            "SELECT coalesce(max(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?;

        for (version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v > current) {
            tracing::info!(version, "applying migration");
            self.conn.execute_batch(&format!(
                "BEGIN;
                 {sql}
                 INSERT INTO schema_migrations (version) VALUES ({version});
                 COMMIT;"
            ))?;
        }
        Ok(())
    }

    /// Highest applied migration version.
    pub fn schema_version(&self) -> Result<u32, StoreError> {
        Ok(self.conn.query_row(
            "SELECT coalesce(max(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?)
    }

    /// Idempotent: re-ingesting the same `(session_id, seq)` is a no-op.
    /// Returns whether a row was actually written.
    pub fn ingest_monitor(
        &self,
        rower_id: &str,
        env: &Envelope<MonitorSample>,
    ) -> Result<bool, StoreError> {
        let s = &env.payload;
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO monitor_sample
             (session_id, seq, rower_id, ts, elapsed_s, distance_m,
              split_s_per_500m, stroke_rate_spm, power_watts, heart_rate_bpm, phase)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                env.session_id.to_string(),
                env.seq,
                rower_id,
                env.ts.to_rfc3339(),
                s.elapsed_s,
                s.distance_m,
                s.split_s_per_500m,
                s.stroke_rate_spm,
                s.power_watts,
                s.heart_rate_bpm,
                format!("{:?}", s.phase).to_lowercase(),
            ],
        )?;
        Ok(inserted > 0)
    }

    /// Idempotent stroke ingest; see [`Store::ingest_monitor`].
    pub fn ingest_stroke(
        &self,
        rower_id: &str,
        env: &Envelope<StrokeSample>,
    ) -> Result<bool, StoreError> {
        let s = &env.payload;
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO stroke
             (session_id, seq, rower_id, ts, stroke_number, drive_time_ms,
              recovery_time_ms, stroke_rate_spm, power_watts, split_s_per_500m,
              distance_m, drive_length_m)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                env.session_id.to_string(),
                env.seq,
                rower_id,
                env.ts.to_rfc3339(),
                s.stroke_number,
                s.drive_time_ms,
                s.recovery_time_ms,
                s.stroke_rate_spm,
                s.power_watts,
                s.split_s_per_500m,
                s.distance_m,
                s.drive_length_m,
            ],
        )?;
        Ok(inserted > 0)
    }

    /// Idempotent workout-event ingest; payload stored lossless as JSON.
    pub fn ingest_workout_event(
        &self,
        rower_id: &str,
        env: &Envelope<WorkoutEvent>,
    ) -> Result<bool, StoreError> {
        let event_type = match &env.payload {
            WorkoutEvent::Started { .. } => "started",
            WorkoutEvent::IntervalBoundary { .. } => "interval_boundary",
            WorkoutEvent::Ended { .. } => "ended",
        };
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO workout_event
             (session_id, seq, rower_id, ts, event_type, payload)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![
                env.session_id.to_string(),
                env.seq,
                rower_id,
                env.ts.to_rfc3339(),
                event_type,
                serde_json::to_string(&env.payload)?,
            ],
        )?;
        Ok(inserted > 0)
    }

    /// Per-session overview, newest first.
    pub fn session_summaries(&self) -> Result<Vec<SessionSummaryRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT
                 e.session_id,
                 e.rower_id,
                 CAST(min(e.ts) AS TEXT)                          AS started_at,
                 (SELECT count(*) FROM monitor_sample m
                   WHERE m.session_id = e.session_id)             AS monitor_samples,
                 (SELECT count(*) FROM stroke s
                   WHERE s.session_id = e.session_id)             AS strokes,
                 (SELECT max(m.distance_m) FROM monitor_sample m
                   WHERE m.session_id = e.session_id)             AS last_distance_m
             FROM workout_event e
             GROUP BY e.session_id, e.rower_id
             ORDER BY min(e.ts) DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SessionSummaryRow {
                    session_id: row.get(0)?,
                    rower_id: row.get(1)?,
                    started_at: row.get(2)?,
                    monitor_samples: row.get(3)?,
                    strokes: row.get(4)?,
                    last_distance_m: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use monorail_core::telemetry::StrokePhase;
    use monorail_core::wire::WIRE_VERSION;
    use monorail_core::SessionId;
    use uuid::Uuid;

    use super::*;

    fn monitor_env(seq: u64) -> Envelope<MonitorSample> {
        Envelope {
            v: WIRE_VERSION,
            session_id: SessionId(Uuid::from_u128(1)),
            seq,
            ts: Utc.with_ymd_and_hms(2026, 6, 10, 6, 30, 0).unwrap(),
            payload: MonitorSample {
                elapsed_s: seq as f64 / 10.0,
                distance_m: seq as f64 * 0.4,
                split_s_per_500m: 120.0,
                stroke_rate_spm: 20.0,
                power_watts: 203.0,
                heart_rate_bpm: Some(140),
                phase: StrokePhase::Drive,
            },
        }
    }

    fn event_env(seq: u64) -> Envelope<WorkoutEvent> {
        Envelope {
            v: WIRE_VERSION,
            session_id: SessionId(Uuid::from_u128(1)),
            seq,
            ts: Utc.with_ymd_and_hms(2026, 6, 10, 6, 29, 55).unwrap(),
            payload: WorkoutEvent::Started {
                ts: Utc.with_ymd_and_hms(2026, 6, 10, 6, 29, 55).unwrap(),
                plan_id: None,
            },
        }
    }

    #[test]
    fn migrations_apply_and_are_idempotent() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.schema_version().unwrap(), 1);
        // Re-running on the same connection is a no-op.
        store.migrate().unwrap();
        assert_eq!(store.schema_version().unwrap(), 1);
    }

    #[test]
    fn double_ingest_is_deduplicated() {
        let store = Store::open_in_memory().unwrap();
        let env = monitor_env(7);
        assert!(store.ingest_monitor("erg-1", &env).unwrap());
        // Redelivery of the same (session_id, seq): no second row.
        assert!(!store.ingest_monitor("erg-1", &env).unwrap());

        let count: u64 = store
            .conn
            .query_row("SELECT count(*) FROM monitor_sample", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn workout_event_payload_round_trips() {
        let store = Store::open_in_memory().unwrap();
        let env = event_env(0);
        assert!(store.ingest_workout_event("erg-1", &env).unwrap());

        let (event_type, payload): (String, String) = store
            .conn
            .query_row(
                "SELECT event_type, payload FROM workout_event WHERE seq = 0",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(event_type, "started");
        let parsed: WorkoutEvent = serde_json::from_str(&payload).unwrap();
        assert_eq!(parsed, env.payload);
    }

    #[test]
    fn session_summaries_aggregate_per_session() {
        let store = Store::open_in_memory().unwrap();
        store.ingest_workout_event("erg-1", &event_env(0)).unwrap();
        for seq in 1..=5 {
            store.ingest_monitor("erg-1", &monitor_env(seq)).unwrap();
        }

        let summaries = store.session_summaries().unwrap();
        assert_eq!(summaries.len(), 1);
        let row = &summaries[0];
        assert_eq!(row.rower_id, "erg-1");
        assert_eq!(row.monitor_samples, 5);
        assert_eq!(row.strokes, 0);
        assert_eq!(row.last_distance_m, Some(2.0));
    }
}
