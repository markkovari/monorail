# ADR 0011: Web Frontend — Leptos SPA + HTTP/SSE API on the Sink

## Status

Accepted

## Context

The CLI entry point (ADR 0009) is not enough: planning a workout, browsing
history, watching live telemetry, and reviewing compliance/predictions want a
browser UI usable from a phone or laptop on the LAN.

Framework: Leptos chosen over a JS/TS SPA. Deciding factor is type flow —
every DTO already lives in `monorail-core` (ADR 0005); Leptos compiles those
same serde types to WASM, so the UI deserializes the exact structs the
pipeline produces. A JS frontend would reintroduce the schema-drift problem
ADR 0005 exists to prevent, mitigated only by codegen (ts-rs/typeshare) and a
second toolchain. Accepted cost: thinner component/charting ecosystem; chart
needs (pace/split curves, force curves, load trends) are canvas-drawable, and
a thin `wasm-bindgen` interop to a JS chart library remains an escape hatch
inside one component, not an architecture change.

Rendering mode: Leptos offers CSR (static WASM bundle) or SSR with server
functions (`leptos_axum`). SSR's benefits (SEO, first-paint, isomorphic RPC)
barely apply to a single-user LAN dashboard, and server functions would fuse
UI builds into the sink's build graph (which carries DuckDB's C++ — painful
compile coupling, ADR 0006). CSR keeps the UI a static artifact behind a
plain API boundary.

Where the API lives: DuckDB is single-writer and the sink process owns the
write connection (ADR 0006); the sink also already holds the NATS connection
for the command plane (ADR 0010). Any API server placed elsewhere would have
to proxy through the sink anyway.

## Decision

- **Leptos, client-side rendered.** New workspace member `web/monorail-ui`,
  built with `trunk` to a static WASM bundle. Excluded from
  `workspace.default-members` so plain `cargo build` never targets
  `wasm32-unknown-unknown`; CI builds it explicitly.
- **API embedded in the sink** as a new library crate `monorail-api` (axum
  router), mounted by `monorail-sink`, which also serves the built UI bundle
  as static files. One process, one port, one systemd unit on the
  aggregation host.
- Surface:

  ```
  REST  /api/v1/plans            POST (goal → generated plan), GET list
        /api/v1/plans/{id}       GET; POST /push   (→ command plane, ADR 0010)
        /api/v1/sessions         GET list; GET /{id} detail + compliance
        /api/v1/predictions      GET current model outputs
        /api/v1/rowers/{id}/status   GET (Pi/PM5 liveness from status subject)
  SSE   /api/v1/live/{rower_id}  monitor samples + stroke events, fanned out
                                 from the sink's JetStream consumer
  ```

  SSE over WebSocket: traffic is strictly server→client at ~10 Hz, SSE is
  plain HTTP (no upgrade dance), auto-reconnects in-browser, and the UI
  sends its commands as ordinary REST calls.
- **DTO discipline**: request/response bodies are `monorail-core` types (or
  thin wrappers in a `monorail-core::api` module), used verbatim on both
  sides. The envelope rules of ADR 0005 (versioning, unknown-field
  tolerance) apply to the API surface too.
- Dependency rule kept clean: `monorail-ui` depends only on `monorail-core`
  (+ Leptos); it never links store/predict/coach. `monorail-api` depends on
  store/predict/coach but contains no SQL — it calls their typed functions.
- Auth: none beyond LAN exposure for v1 (single user, matches the trust
  model of ADR 0010). Anything internet-facing is a superseding ADR.

## Consequences

- One language end to end; adding a field to `StrokeSample` propagates to
  the browser by recompiling, with the compiler — not a codegen step —
  catching mismatches.
- UI build needs `trunk` + the `wasm32-unknown-unknown` target; UI compile
  times stay decoupled from the sink's DuckDB-heavy build.
- Live dashboard comes nearly free: the sink already consumes the telemetry
  stream; SSE fan-out is a broadcast channel away.
- Charting is the known weak spot; budgeted escape hatch is per-component JS
  interop, not a framework change.
- Sink binary becomes the single host-side deployable (consumer + DB + API +
  static UI) — simple ops, but a sink restart now also drops live dashboard
  connections; SSE auto-reconnect makes that a blip.
