# ADR 0005: Wire Serialization — JSON Now, Schema Discipline for Later

## Status

Accepted

## Context

Telemetry messages need a serialization format for NATS payloads. Candidates:

- **JSON** (`serde_json`): human-readable (`nats sub 'monorail.>'` is
  instantly debuggable), DuckDB ingests it natively (`read_json`, JSON
  functions), schema evolution is forgiving. Cost: ~2–4× larger payloads,
  slower encode — irrelevant at a few KB/s.
- **Postcard / CBOR / MessagePack**: compact binary, still serde-driven, but
  opaque on the wire and needing decode shims in every tool that touches the
  stream.
- **Protobuf**: strong schema contracts and cross-language reach, at the cost
  of codegen plumbing — unjustified while every producer and consumer is a
  Rust crate in this workspace sharing `monorail-core` types.

The real risk with JSON is not size but *schema drift*: anonymous payloads
that quietly change shape. That is addressed with discipline rather than a
binary format.

## Decision

- NATS payloads are **JSON**, produced by `serde` from types in
  `monorail-core`.
- Every message envelope carries:
  - `v`: integer schema version per message type, bumped on breaking change;
  - `session_id` (UUID, minted at workout detection) and `seq` (monotonic
    per session) — also used for JetStream dedup (ADR 0004) and the DuckDB
    primary key (ADR 0006);
  - `ts`: capture timestamp (UTC, from the Pi's clock, NTP-synced).
- Wire types live in `monorail-core` in one module, with serde round-trip
  tests and checked-in sample JSON fixtures acting as the de-facto schema
  documentation.
- Consumers must ignore unknown fields (serde default) — additive changes
  are non-breaking and need no version bump.

## Consequences

- Whole pipeline debuggable with `nats` CLI and `jq`; DuckDB can even ingest
  raw payloads directly during development.
- Bandwidth/CPU cost accepted knowingly; at rowing data rates it is noise.
- If a non-Rust or high-rate producer ever appears, revisit with Protobuf —
  the envelope versioning and `monorail-core` centralization make that a
  contained migration, recorded as a superseding ADR.
