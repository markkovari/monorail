# ADR 0009: Workout Programs — Goal-Driven Plan Generation

## Status

Accepted

## Context

The system so far only records (ADRs 0003–0007). The user also wants
*prescription*: given a training goal — e.g. "UT2, 40 minutes, hold 2:00/500m
at 20 SPM" — the system should generate a segmented workout plan
(e.g. 10 min build @ 2:05/18 → 20 min core @ 2:00/20 → 10 min strong finish
@ 1:58/22), recommend it, and push it to the erg (delivery is ADR 0010).

Observations:

- A plan is a first-class domain object, not a query result. It has identity
  (it gets pushed, executed, and later scored for compliance), versions, and
  a lifecycle: `Draft → Recommended → Scheduled → Active → Completed/Abandoned`.
- Goals decompose into a small typed vocabulary: intensity zone (UT2/UT1/AT/
  TR/AN — standard rowing zones), duration or distance, target split, target
  stroke rate, optionally target HR zone.
- Plan generation should be informed by the athlete's actual capability —
  `monorail-predict` (ADR 0007) already fits critical-power/Riegel models
  from history. A requested 2:00 split that the models say is unsustainable
  for 40 min should yield a warning or an adjusted recommendation, not blind
  acceptance.
- Generation logic is pure planning over features + goal; it needs history
  access (DuckDB) and prediction, but no PM5 or NATS knowledge.
- Where things run: history, predictions, and the user-facing entry point
  live on the aggregation host — generation belongs there, not on the Pi.

## Decision

- New library crate **`monorail-coach`** (extends the ADR 0002 layout):
  goal types, plan generation, template library, compliance scoring.
  Dependency direction: `coach → predict → store → core`. No I/O deps of its
  own.
- **Plan model** in `monorail-core` (it crosses the wire, so it lives with
  the other wire types per ADR 0005):
  - `WorkoutGoal { zone, duration | distance, target_split, target_spm, hr_cap? }`
  - `WorkoutPlan { plan_id, goal, segments: Vec<Segment>, created_from: ModelWatermark }`
  - `Segment { duration | distance, target_split_range, target_spm_range, intent }`
    where `intent` is a display label (`Build`, `Core`, `Push`, `Recover`).
- **Generation = templates × personalization**:
  1. A curated template library keyed by zone/goal shape (steady UT2 with
     build/core/push phases, classic interval patterns, negative splits) —
     templates are data (checked-in RON/JSON), not code, so adding programs
     needs no recompile.
  2. Personalization pass: `monorail-predict` validates feasibility of the
     requested split/duration against fitted athlete models and shifts
     segment targets accordingly (or annotates the plan with a feasibility
     warning when the user insists on the literal goal).
- **Compliance scoring**: after a session, `monorail-coach` joins the plan's
  segments against recorded telemetry (by `plan_id` stamped into the
  session's workout events) and stores per-segment adherence (time in
  split band, time in SPM band) into DuckDB. Scores feed back into both
  future recommendations and prediction features.
- Plans persist in DuckDB (`plan`, `plan_segment`, `plan_compliance`
  tables, owned by `monorail-store` like all schema per ADR 0006).
- User entry point: subcommand on the sink binary for now
  (`monorail-sink plan new --zone ut2 --duration 40m --split 2:00 --spm 20`);
  a web/API surface is a future ADR.

## Consequences

- Prescription and recording share one domain vocabulary; compliance scoring
  closes the loop (plans get better because results are measured).
- Template-as-data keeps the program library editable without releases.
- Feasibility checking prevents the recommender from rubber-stamping
  physiologically silly goals — predictions (ADR 0007) gain a second
  consumer, which pressure-tests their accuracy.
- New crate + three tables of schema; accepted scope growth.
- Athlete-facing live coaching display (cues mid-workout) is *not* covered
  here — execution/delivery is ADR 0010, and any richer UI is a future ADR.
