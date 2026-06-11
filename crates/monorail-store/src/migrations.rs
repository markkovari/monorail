//! Numbered schema migrations (ADR 0006), applied in order at startup and
//! tracked in `schema_migrations`. Append-only: never edit a shipped
//! migration, add a new one.

pub const MIGRATIONS: &[(u32, &str)] = &[
    (
        1,
        r#"
    -- Raw append-only tables. (session_id, seq) is the system-wide
    -- idempotency key (ADRs 0004/0005); raw rows are never mutated.

    CREATE TABLE monitor_sample (
        session_id       TEXT NOT NULL,
        seq              BIGINT NOT NULL,
        rower_id         TEXT NOT NULL,
        ts               TIMESTAMPTZ NOT NULL,
        elapsed_s        DOUBLE NOT NULL,
        distance_m       DOUBLE NOT NULL,
        split_s_per_500m REAL NOT NULL,
        stroke_rate_spm  REAL NOT NULL,
        power_watts      REAL NOT NULL,
        heart_rate_bpm   SMALLINT,
        phase            TEXT NOT NULL,
        PRIMARY KEY (session_id, seq)
    );

    CREATE TABLE stroke (
        session_id       TEXT NOT NULL,
        seq              BIGINT NOT NULL,
        rower_id         TEXT NOT NULL,
        ts               TIMESTAMPTZ NOT NULL,
        stroke_number    INTEGER NOT NULL,
        drive_time_ms    INTEGER NOT NULL,
        recovery_time_ms INTEGER NOT NULL,
        stroke_rate_spm  REAL NOT NULL,
        power_watts      REAL NOT NULL,
        split_s_per_500m REAL NOT NULL,
        distance_m       DOUBLE NOT NULL,
        drive_length_m   REAL,
        PRIMARY KEY (session_id, seq)
    );

    -- Lifecycle events keep their full JSON payload: the tagged enum evolves
    -- (ADR 0005) and raw must stay lossless; typed columns are derived later.
    CREATE TABLE workout_event (
        session_id TEXT NOT NULL,
        seq        BIGINT NOT NULL,
        rower_id   TEXT NOT NULL,
        ts         TIMESTAMPTZ NOT NULL,
        event_type TEXT NOT NULL,
        payload    TEXT NOT NULL,
        PRIMARY KEY (session_id, seq)
    );
    "#,
    ),
    (
        2,
        r#"
    -- Workout plans (ADR 0009). The full WorkoutPlan is stored lossless as
    -- JSON; status tracks the lifecycle
    -- (draft -> recommended -> scheduled -> active -> completed/abandoned).
    CREATE TABLE plan (
        plan_id    TEXT PRIMARY KEY,
        rower_id   TEXT NOT NULL,
        created_at TIMESTAMPTZ NOT NULL,
        status     TEXT NOT NULL,
        body       TEXT NOT NULL
    );
    "#,
    ),
    (
        3,
        r#"
    -- Per-segment plan adherence, computed after a session ends (ADR 0009).
    -- Derived data: always rebuildable from plan + monitor_sample.
    CREATE TABLE plan_compliance (
        plan_id       TEXT NOT NULL,
        session_id    TEXT NOT NULL,
        segment_index INTEGER NOT NULL,
        intent        TEXT NOT NULL,
        sample_count  INTEGER NOT NULL,
        split_in_band REAL NOT NULL,
        spm_in_band   REAL NOT NULL,
        PRIMARY KEY (plan_id, session_id, segment_index)
    );
    "#,
    ),
];
