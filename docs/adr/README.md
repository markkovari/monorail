# Architecture Decision Records

Index of ADRs for **monorail** — PM5 rowing telemetry: Pi Zero 2 W → NATS
JetStream → DuckDB → predictions → coached workout programs.

| # | Title | Status |
|---|-------|--------|
| [0001](0001-record-architecture-decisions.md) | Record architecture decisions | Accepted |
| [0002](0002-cargo-workspace-monorepo.md) | Single Cargo workspace monorepo | Accepted |
| [0003](0003-pm5-via-usb-hid-csafe.md) | PM5 connection via USB HID + CSAFE | Accepted |
| [0004](0004-nats-jetstream-transport.md) | NATS JetStream as telemetry transport | Accepted |
| [0005](0005-serialization-format.md) | Wire serialization: JSON with schema discipline | Accepted |
| [0006](0006-duckdb-aggregation-store.md) | DuckDB as aggregation store / system of record | Accepted |
| [0007](0007-prediction-approach.md) | Prediction: classical models over DuckDB features | Accepted |
| [0008](0008-pi-cross-compilation-deployment.md) | Cross-compilation and deployment to Pi Zero 2 W | Accepted |
| [0009](0009-workout-programs-and-coaching.md) | Workout programs: goal-driven plan generation | Accepted |
| [0010](0010-command-plane-pm5-programming.md) | Command plane: pushing plans to Pi, programming PM5 | Accepted |
| [0011](0011-web-frontend-leptos.md) | Web frontend: Leptos SPA + HTTP/SSE API on sink | Accepted |
| [0012](0012-athlete-profile-weight-adjusted-calories.md) | Athlete profile and weight-adjusted calories | Accepted |
| [0013](0013-concept2-logbook-import.md) | Concept2 Logbook (ErgData) import | Accepted |

## System sketch

```
┌──────────┐  USB HID/CSAFE   ┌─────────────────┐  telemetry (JetStream)  ┌──────────────────────┐
│   PM5    │◄────────────────►│  Pi Zero 2 W    │────────────────────────►│  Aggregation host    │
│ (rowerg) │  poll + program  │  monorail-rower │◄────────────────────────│  monorail-sink       │
└──────────┘                  └─────────────────┘  commands (req/reply)   │  ├─ DuckDB (store)   │
                                                                          │  ├─ predictions      │
     athlete sees programmed intervals + target pace on PM5               │  ├─ coach (plans)    │
                                                                          │  └─ API + UI bundle  │
                                                                          └──────────┬───────────┘
                                                              REST + SSE             │
                                                   ┌─────────────────────────────────┘
                                                   ▼
                                          browser (Leptos WASM SPA)
```

Crates: `monorail-core` (domain/wire types) ← `monorail-pm5`,
`monorail-stream`, `monorail-store` ← `monorail-predict` ← `monorail-coach`
← `monorail-api`; `web/monorail-ui` (Leptos, depends on core only).
Bins: `monorail-rower` (Pi), `monorail-sink` (host: consumer + DB + API + UI).

## Adding an ADR

Copy the Status/Context/Decision/Consequences shape, next sequential number,
add a row here. Supersede, don't delete.
