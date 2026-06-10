# ADR 0002: Single Cargo Workspace Monorepo

## Status

Accepted

## Context

The system has several concerns that evolve together but deploy separately:

1. **Domain types** — stroke samples, workout summaries, heart-rate data.
   Shared by everything; must stay dependency-light.
2. **PM5 interaction** — USB HID transport + CSAFE protocol framing. Runs on
   the Pi.
3. **Streaming** — publishing telemetry to NATS JetStream. Runs on the Pi.
4. **Sink** — consuming the stream and persisting/aggregating into DuckDB.
   Runs on a workstation/server (DuckDB on a Pi Zero's 512 MB is a poor fit).
5. **Prediction** — models over aggregated data.

Alternatives considered: separate repositories per concern (polyrepo), or a
single crate with feature flags.

- Polyrepo: version-skew pain for the shared domain types; cross-repo changes
  (e.g., adding a field to `StrokeSample`) become multi-PR dances. Overkill
  for a single-developer project.
- Single crate + features: DuckDB, libusb/hidapi, and NATS dependencies would
  tangle; compiling the sink would require cross-compiling DuckDB for the Pi
  target or careful feature hygiene forever.

## Decision

One git repository, one Cargo workspace. Library crates under `crates/`,
deployable binaries under `bins/`:

```
monorail/
├── Cargo.toml              # [workspace], shared deps via workspace.dependencies
├── crates/
│   ├── monorail-core/      # domain types, units, serde models (no I/O deps)
│   ├── monorail-pm5/       # CSAFE protocol + USB HID transport for PM5
│   ├── monorail-stream/    # NATS JetStream publish/consume helpers, subjects
│   ├── monorail-store/     # DuckDB schema, ingestion, aggregation queries
│   └── monorail-predict/   # prediction models over aggregated data
├── bins/
│   ├── monorail-rower/     # Pi binary: PM5 → JetStream publisher
│   └── monorail-sink/      # server binary: JetStream → DuckDB → predictions
└── docs/adr/
```

Rules:

- `monorail-core` has no I/O dependencies (serde + chrono/uuid class only).
  Everything depends on it; it depends on nothing internal.
- Heavy native deps stay quarantined: `duckdb` only in `monorail-store`,
  `hidapi` only in `monorail-pm5`. The Pi binary must never transitively pull
  DuckDB.
- Shared dependency versions are pinned once in `[workspace.dependencies]`.
- Binaries are thin: CLI parsing, config, wiring. Logic lives in libraries so
  it is testable off-hardware.

## Consequences

- Atomic cross-cutting changes; one `cargo test` covers the system.
- One lockfile — consistent dependency resolution for Pi and server builds.
- Cross-compilation must build only the Pi-relevant subset
  (`cargo build -p monorail-rower`); workspace-wide commands on the Pi target
  would drag DuckDB in. See ADR 0008.
- Crate granularity can be split further later (e.g., `monorail-pm5-proto`
  vs `monorail-pm5-usb`) without repo surgery.
