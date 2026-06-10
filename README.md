# monorail

Concept2 PM5 rowing telemetry pipeline: a Raspberry Pi Zero 2 W polls the
monitor over USB HID (CSAFE), streams telemetry over NATS JetStream, a sink
aggregates it in DuckDB, fits prediction models, and generates coached
workout programs it can push back onto the PM5. Leptos web UI on top.

Architecture is documented as ADRs — start at [docs/adr/](docs/adr/README.md).

## Layout

| Path | What |
|------|------|
| `crates/monorail-core` | Domain/wire types; no I/O deps; wasm-safe |
| `crates/monorail-pm5` | CSAFE protocol (pure) + USB HID transport |
| `crates/monorail-stream` | NATS subjects + JetStream helpers |
| `crates/monorail-store` | DuckDB schema, ingest, aggregation queries |
| `crates/monorail-predict` | Prediction models over feature frames |
| `crates/monorail-coach` | Plan generation, templates, compliance |
| `crates/monorail-api` | Axum HTTP/SSE API surface |
| `bins/monorail-rower` | Pi binary: PM5 → JetStream + command handler |
| `bins/monorail-sink` | Host binary: consumer + DuckDB + API + UI |
| `web/monorail-ui` | Leptos CSR dashboard (trunk, wasm32) |

## Build

```sh
cargo build            # everything except the UI (default-members)
cargo test

# UI (requires: rustup target add wasm32-unknown-unknown; cargo install trunk)
cd web/monorail-ui && trunk serve

# Pi binary (requires cross; 64-bit Raspberry Pi OS on the Pi — ADR 0008)
cross build --release -p monorail-rower --target aarch64-unknown-linux-gnu
```

## Testing

- Unit tests live inline in `#[cfg(test)] mod tests` next to the code.
- Fixture/cross-module tests live in each crate's `tests/` directory.
- Pure transforms (CSAFE framing, plan generation) also get `proptest`
  property tests (`crates/monorail-pm5/tests/`, `crates/monorail-coach/tests/`).
- `crates/monorail-core/tests/fixtures/*.json` are the wire-schema record
  (ADR 0005): if `wire_fixtures` tests fail, the schema changed — fix the
  regression or consciously bump `WIRE_VERSION` and update the fixture.

CI (`.github/workflows/ci.yml`) gates on fmt, clippy `-D warnings`, tests,
a wasm32 check of the UI, and a guard that keeps DuckDB out of the Pi
binary's dependency tree; pushes to `main` also cross-compile the Pi binary.
