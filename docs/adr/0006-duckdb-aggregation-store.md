# ADR 0006: DuckDB as Aggregation Store and System of Record

## Status

Accepted

## Context

Consumed telemetry needs persistent storage supporting analytical queries:
rolling aggregates per workout, pace/power distributions, training-load
features for prediction. Workload shape: append-heavy ingest in bursts
(during/after workouts), read-heavy analytical scans (window functions,
aggregations over months of strokes), single writer, embedded — no DB server
to babysit.

Alternatives:

- **SQLite**: superb embedded OLTP, weak at wide analytical scans and window-
  function-heavy feature extraction over millions of stroke rows.
- **Postgres/Timescale**: capable but a running service with upgrades and
  backups; oversized for a single-user training log.
- **Parquet files + DataFusion**: viable, but loses easy upserts/idempotent
  ingest; more moving parts in application code.

DuckDB is embedded (no service), columnar (made for exactly these scans),
speaks SQL with full window functions, exports Parquet for interchange, and
has a maintained Rust crate (`duckdb`, bundled build). It runs in the sink
process on the aggregation host — never on the Pi (ADR 0002).

## Decision

- **DuckDB** is the aggregation store and the **system of record** (JetStream
  is a durable buffer with bounded retention, not the archive — ADR 0004).
- Access is confined to `monorail-store`, which owns:
  - schema + versioned migrations (numbered SQL files applied at startup,
    tracked in a `schema_migrations` table);
  - ingestion (idempotent inserts keyed on `(session_id, seq)` —
    `INSERT ... ON CONFLICT DO NOTHING` — making at-least-once delivery
    safe);
  - canonical aggregation queries (per-session summaries, per-interval
    splits, training-load features) exposed as typed Rust functions, so
    `monorail-predict` consumes feature frames, not SQL strings.
- Layout: raw append tables (`stroke`, `monitor_sample`, `workout_event`)
  plus derived tables/views (`session_summary`, `interval_split`,
  `daily_load`) recomputed from raw — raw data is never mutated, derived
  data is always rebuildable.
- Single-writer rule: only the sink process opens the database read-write.
  Ad-hoc analysis uses read-only connections or Parquet exports.
- Backup: periodic `EXPORT DATABASE` / Parquet snapshots; catastrophic loss
  is additionally recoverable by JetStream replay within retention.

## Consequences

- No database service to operate; the analytics engine lives inside the sink
  binary.
- Analytical queries over the full stroke history stay fast without manual
  pre-aggregation gymnastics.
- DuckDB's single-writer model is a real constraint; respected by design
  (one sink). A second writer would force an architecture change and a new
  ADR.
- `duckdb` crate's bundled C++ build makes sink compile times chunky —
  confined to `monorail-store` so Pi-target and core-crate builds never pay
  it.
