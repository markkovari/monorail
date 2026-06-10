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
