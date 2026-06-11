//! DuckDB aggregation store (ADR 0006).
//!
//! This crate is the only one allowed to depend on `duckdb` or contain SQL.
//! Raw tables are append-only and ingested idempotently keyed on
//! `(session_id, seq)`, so JetStream's at-least-once delivery is safe to
//! replay. Single-writer rule: exactly one process opens the database
//! read-write (the sink).

mod migrations;

use std::path::Path;

use chrono::{DateTime, Utc};
use duckdb::{params, Connection, OptionalExt};
use monorail_core::metrics::{self, AthleteProfile};
use monorail_core::plan::{ComplianceReport, WorkoutPlan};
use monorail_core::telemetry::{MonitorSample, StrokePhase, StrokeSample, WorkoutEvent};
use monorail_core::wire::Envelope;
use monorail_core::{PlanId, SessionId};
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

/// One row of the plan overview query.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlanRow {
    pub plan_id: String,
    pub rower_id: String,
    pub created_at: String,
    pub status: String,
}

/// One stored per-segment compliance row.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ComplianceRow {
    pub plan_id: String,
    pub segment_index: u32,
    pub intent: String,
    pub sample_count: u32,
    pub split_in_band: f32,
    pub spm_in_band: f32,
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
    pub duration_s: Option<f64>,
    pub avg_power_watts: Option<f64>,
    /// Calories as a PM5 would show them (175 lb reference, ADR 0012).
    pub kcal_pm: Option<f64>,
    /// Weight-adjusted calories; `null` until an athlete weight is set.
    pub kcal_adjusted: Option<f64>,
}

/// One imported Concept2 Logbook result (ADR 0013).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LogbookRow {
    pub id: u64,
    pub date: String,
    pub distance_m: Option<f64>,
    pub duration_s: Option<f64>,
    pub calories_total: Option<u32>,
    pub stroke_rate: Option<u32>,
    pub raw: String,
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

    /// Persist a generated plan with its lifecycle status (ADR 0009).
    pub fn save_plan(
        &self,
        plan: &WorkoutPlan,
        status: &str,
        created_at: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO plan (plan_id, rower_id, created_at, status, body)
             VALUES (?, ?, ?, ?, ?)",
            params![
                plan.plan_id.to_string(),
                plan.rower_id.as_str(),
                created_at.to_rfc3339(),
                status,
                serde_json::to_string(plan)?,
            ],
        )?;
        Ok(())
    }

    pub fn get_plan(&self, plan_id: PlanId) -> Result<Option<WorkoutPlan>, StoreError> {
        let body: Option<String> = self
            .conn
            .query_row(
                "SELECT body FROM plan WHERE plan_id = ?",
                params![plan_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(body.map(|b| serde_json::from_str(&b)).transpose()?)
    }

    pub fn set_plan_status(&self, plan_id: PlanId, status: &str) -> Result<bool, StoreError> {
        let updated = self.conn.execute(
            "UPDATE plan SET status = ? WHERE plan_id = ?",
            params![status, plan_id.to_string()],
        )?;
        Ok(updated > 0)
    }

    /// Plan overview rows, newest first.
    pub fn list_plans(&self) -> Result<Vec<PlanRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT plan_id, rower_id, CAST(created_at AS TEXT), status
             FROM plan ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(PlanRow {
                    plan_id: row.get(0)?,
                    rower_id: row.get(1)?,
                    created_at: row.get(2)?,
                    status: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Per-session overview, newest first. Calories are derived at query
    /// time (ADRs 0006/0012): PM-reference always, weight-adjusted only
    /// when an athlete weight is set.
    pub fn session_summaries(&self) -> Result<Vec<SessionSummaryRow>, StoreError> {
        let weight_kg = self.get_athlete()?.map(|a| a.weight_kg as f64);
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
                   WHERE m.session_id = e.session_id)             AS last_distance_m,
                 (SELECT max(m.elapsed_s) FROM monitor_sample m
                   WHERE m.session_id = e.session_id)             AS duration_s,
                 (SELECT avg(m.power_watts) FROM monitor_sample m
                   WHERE m.session_id = e.session_id)             AS avg_power_watts
             FROM workout_event e
             GROUP BY e.session_id, e.rower_id
             ORDER BY min(e.ts) DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                let duration_s: Option<f64> = row.get(6)?;
                let avg_power_watts: Option<f64> = row.get(7)?;
                let kcal = |per_hr: fn(f64) -> f64| {
                    Some(metrics::workout_kcal(per_hr(avg_power_watts?), duration_s?))
                };
                Ok(SessionSummaryRow {
                    session_id: row.get(0)?,
                    rower_id: row.get(1)?,
                    started_at: row.get(2)?,
                    monitor_samples: row.get(3)?,
                    strokes: row.get(4)?,
                    last_distance_m: row.get(5)?,
                    duration_s,
                    avg_power_watts,
                    kcal_pm: kcal(metrics::pm_kcal_per_hr),
                    kcal_adjusted: weight_kg.and_then(|kg| {
                        Some(metrics::workout_kcal(
                            metrics::weight_adjusted_kcal_per_hr(avg_power_watts?, kg),
                            duration_s?,
                        ))
                    }),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Athlete profile (single-row table, ADR 0012).
    pub fn get_athlete(&self) -> Result<Option<AthleteProfile>, StoreError> {
        let weight: Option<f32> = self
            .conn
            .query_row("SELECT weight_kg FROM athlete WHERE id = 1", [], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(weight.map(|weight_kg| AthleteProfile { weight_kg }))
    }

    pub fn set_athlete(
        &self,
        profile: AthleteProfile,
        updated_at: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO athlete (id, weight_kg, updated_at) VALUES (1, ?, ?)",
            params![profile.weight_kg, updated_at.to_rfc3339()],
        )?;
        Ok(())
    }

    /// Idempotent upsert of imported Logbook results (ADR 0013).
    /// Returns how many were newly inserted.
    pub fn upsert_logbook_results(&self, results: &[LogbookRow]) -> Result<u64, StoreError> {
        let mut inserted = 0;
        for r in results {
            inserted += self.conn.execute(
                "INSERT OR IGNORE INTO logbook_result
                 (id, date, distance_m, duration_s, calories_total, stroke_rate, raw)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                params![
                    r.id,
                    r.date,
                    r.distance_m,
                    r.duration_s,
                    r.calories_total,
                    r.stroke_rate,
                    r.raw,
                ],
            )? as u64;
        }
        Ok(inserted)
    }

    /// Imported Logbook results, newest first.
    pub fn list_logbook_results(&self) -> Result<Vec<LogbookRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, date, distance_m, duration_s, calories_total, stroke_rate, raw
             FROM logbook_result ORDER BY date DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(LogbookRow {
                    id: row.get(0)?,
                    date: row.get(1)?,
                    distance_m: row.get(2)?,
                    duration_s: row.get(3)?,
                    calories_total: row.get(4)?,
                    stroke_rate: row.get(5)?,
                    raw: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Monitor samples for a session, in sequence order, reconstructed as
    /// wire types for compliance scoring.
    pub fn session_monitor_samples(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<MonitorSample>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT elapsed_s, distance_m, split_s_per_500m, stroke_rate_spm,
                    power_watts, heart_rate_bpm, phase
             FROM monitor_sample WHERE session_id = ? ORDER BY seq",
        )?;
        let rows = stmt
            .query_map(params![session_id.to_string()], |row| {
                let phase: String = row.get(6)?;
                Ok(MonitorSample {
                    elapsed_s: row.get(0)?,
                    distance_m: row.get(1)?,
                    split_s_per_500m: row.get(2)?,
                    stroke_rate_spm: row.get(3)?,
                    power_watts: row.get(4)?,
                    heart_rate_bpm: row.get(5)?,
                    phase: match phase.as_str() {
                        "drive" => StrokePhase::Drive,
                        "dwell" => StrokePhase::Dwell,
                        "recovery" => StrokePhase::Recovery,
                        _ => StrokePhase::Idle,
                    },
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The plan a session executed, if its `started` event carries one.
    pub fn session_plan_id(&self, session_id: SessionId) -> Result<Option<PlanId>, StoreError> {
        let payload: Option<String> = self
            .conn
            .query_row(
                "SELECT payload FROM workout_event
                 WHERE session_id = ? AND event_type = 'started'",
                params![session_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let Some(payload) = payload else {
            return Ok(None);
        };
        let event: WorkoutEvent = serde_json::from_str(&payload)?;
        Ok(match event {
            WorkoutEvent::Started { plan_id, .. } => plan_id,
            _ => None,
        })
    }

    /// Persist a compliance report (idempotent on re-score).
    pub fn save_compliance(&self, report: &ComplianceReport) -> Result<(), StoreError> {
        for seg in &report.segments {
            self.conn.execute(
                "INSERT OR REPLACE INTO plan_compliance
                 (plan_id, session_id, segment_index, intent, sample_count,
                  split_in_band, spm_in_band)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                params![
                    report.plan_id.to_string(),
                    report.session_id.to_string(),
                    seg.segment_index,
                    serde_json::to_string(&seg.intent)?.trim_matches('"'),
                    seg.sample_count,
                    seg.split_in_band,
                    seg.spm_in_band,
                ],
            )?;
        }
        Ok(())
    }

    /// Stored compliance rows for a session (empty when unscored/unplanned).
    pub fn get_compliance(&self, session_id: SessionId) -> Result<Vec<ComplianceRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT plan_id, segment_index, intent, sample_count,
                    split_in_band, spm_in_band
             FROM plan_compliance WHERE session_id = ? ORDER BY segment_index",
        )?;
        let rows = stmt
            .query_map(params![session_id.to_string()], |row| {
                Ok(ComplianceRow {
                    plan_id: row.get(0)?,
                    segment_index: row.get(1)?,
                    intent: row.get(2)?,
                    sample_count: row.get(3)?,
                    split_in_band: row.get(4)?,
                    spm_in_band: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Per-session `(duration_s, avg_power_w)` efforts for critical-power
    /// fitting (ADR 0007).
    pub fn session_efforts(&self) -> Result<Vec<(f64, f64)>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT max(elapsed_s), avg(power_watts)
             FROM monitor_sample GROUP BY session_id
             HAVING max(elapsed_s) > 0",
        )?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
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
        let latest = MIGRATIONS.last().unwrap().0;
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.schema_version().unwrap(), latest);
        // Re-running on the same connection is a no-op.
        store.migrate().unwrap();
        assert_eq!(store.schema_version().unwrap(), latest);
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
    fn plan_save_get_status_round_trip() {
        use monorail_core::plan::{Extent, Feasibility, WorkoutGoal, Zone};
        use monorail_core::RowerId;

        let store = Store::open_in_memory().unwrap();
        let plan = WorkoutPlan {
            plan_id: PlanId(Uuid::from_u128(0xab)),
            rower_id: RowerId::new("erg-1").unwrap(),
            goal: WorkoutGoal {
                zone: Zone::Ut2,
                extent: Extent::Time { seconds: 2400 },
                target_split_s: 120.0,
                target_spm: 20,
                hr_cap_bpm: None,
            },
            segments: vec![],
            feasibility: Feasibility::Unchecked,
        };
        let created = Utc.with_ymd_and_hms(2026, 6, 10, 7, 0, 0).unwrap();

        store.save_plan(&plan, "recommended", created).unwrap();
        assert_eq!(store.get_plan(plan.plan_id).unwrap(), Some(plan.clone()));
        assert_eq!(store.get_plan(PlanId(Uuid::from_u128(0xff))).unwrap(), None);

        assert!(store.set_plan_status(plan.plan_id, "scheduled").unwrap());
        let rows = store.list_plans().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, "scheduled");
        assert_eq!(rows[0].rower_id, "erg-1");
    }

    #[test]
    fn session_round_trip_for_scoring_inputs() {
        use monorail_core::plan::{ComplianceReport, SegmentCompliance, SegmentIntent};

        let store = Store::open_in_memory().unwrap();
        let session = SessionId(Uuid::from_u128(1));
        let plan_id = PlanId(Uuid::from_u128(0xab));

        // Started event carrying the plan id.
        let mut started = event_env(0);
        started.payload = WorkoutEvent::Started {
            ts: Utc.with_ymd_and_hms(2026, 6, 10, 6, 29, 55).unwrap(),
            plan_id: Some(plan_id),
        };
        store.ingest_workout_event("erg-1", &started).unwrap();
        for seq in 1..=3 {
            store.ingest_monitor("erg-1", &monitor_env(seq)).unwrap();
        }

        assert_eq!(store.session_plan_id(session).unwrap(), Some(plan_id));
        assert_eq!(
            store
                .session_plan_id(SessionId(Uuid::from_u128(0xff)))
                .unwrap(),
            None
        );

        let samples = store.session_monitor_samples(session).unwrap();
        assert_eq!(samples.len(), 3);
        assert_eq!(samples[0], monitor_env(1).payload);

        // Efforts query sees the session.
        let efforts = store.session_efforts().unwrap();
        assert_eq!(efforts.len(), 1);
        assert!((efforts[0].1 - 203.0).abs() < 0.01);

        // Compliance save is idempotent (re-score replaces).
        let report = ComplianceReport {
            plan_id,
            session_id: session,
            segments: vec![SegmentCompliance {
                segment_index: 0,
                intent: SegmentIntent::Core,
                sample_count: 3,
                split_in_band: 1.0,
                spm_in_band: 0.5,
            }],
            overall_split_in_band: 1.0,
            overall_spm_in_band: 0.5,
        };
        store.save_compliance(&report).unwrap();
        store.save_compliance(&report).unwrap();
        let rows = store.get_compliance(session).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].intent, "core");
        assert_eq!(rows[0].split_in_band, 1.0);
    }

    #[test]
    fn athlete_profile_round_trips_and_unlocks_adjusted_calories() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.get_athlete().unwrap(), None);

        store.ingest_workout_event("erg-1", &event_env(0)).unwrap();
        for seq in 1..=5 {
            store.ingest_monitor("erg-1", &monitor_env(seq)).unwrap();
        }

        // No weight set: PM calories present, adjusted honestly null.
        let row = &store.session_summaries().unwrap()[0];
        assert!(row.kcal_pm.unwrap() > 0.0);
        assert_eq!(row.kcal_adjusted, None);

        let now = Utc.with_ymd_and_hms(2026, 6, 11, 8, 0, 0).unwrap();
        store
            .set_athlete(AthleteProfile { weight_kg: 90.0 }, now)
            .unwrap();
        assert_eq!(
            store.get_athlete().unwrap(),
            Some(AthleteProfile { weight_kg: 90.0 })
        );
        // Upsert replaces, not duplicates.
        store
            .set_athlete(AthleteProfile { weight_kg: 88.0 }, now)
            .unwrap();
        assert_eq!(
            store.get_athlete().unwrap(),
            Some(AthleteProfile { weight_kg: 88.0 })
        );

        // 88 kg > 79.4 kg reference ⇒ adjusted burns more than PM shows.
        let row = &store.session_summaries().unwrap()[0];
        assert!(row.kcal_adjusted.unwrap() > row.kcal_pm.unwrap());
    }

    #[test]
    fn logbook_upsert_is_idempotent() {
        let store = Store::open_in_memory().unwrap();
        let rows = vec![LogbookRow {
            id: 99,
            date: "2026-06-10 18:00:00".into(),
            distance_m: Some(10_000.0),
            duration_s: Some(2400.0),
            calories_total: Some(700),
            stroke_rate: Some(20),
            raw: "{}".into(),
        }];
        assert_eq!(store.upsert_logbook_results(&rows).unwrap(), 1);
        assert_eq!(store.upsert_logbook_results(&rows).unwrap(), 0);
        let stored = store.list_logbook_results().unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].calories_total, Some(700));
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
