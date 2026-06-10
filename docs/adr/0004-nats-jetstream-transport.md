# ADR 0004: NATS JetStream as Telemetry Transport

## Status

Accepted

## Context

Telemetry must travel from the Pi (publisher) to the aggregation host
(consumer). Requirements:

- **Durability**: the sink may be down mid-workout; data must not be lost.
- **Replay**: re-ingesting history into a rebuilt DuckDB file must be
  possible.
- **Lightweight publisher**: the Pi Zero 2 W has 512 MB RAM and modest CPU;
  the client must be cheap. (The NATS *server* runs on the aggregation host
  or another always-on box, not on the Pi.)
- **Low volume**: ~10 Hz × small payloads ≈ a few KB/s. Almost anything
  handles the throughput; the deciding factors are durability semantics and
  operational weight.

Alternatives:

- **MQTT (Mosquitto)**: classic IoT choice, but QoS/retained messages are not
  a replayable log; rebuilding the database from history would need a
  separate archival path.
- **Kafka/Redpanda**: proper log, far too heavy operationally for one rower.
- **Direct HTTP/gRPC to the sink**: couples Pi uptime to sink uptime; buffer-
  and-retry logic would end up reinventing a durable queue badly.

NATS JetStream gives an at-least-once durable stream with consumer-managed
acks, a single small static binary for the server, and a pure-Rust async
client (`async-nats`).

## Decision

- Transport is **NATS JetStream** via the `async-nats` crate.
- Subject hierarchy (defined as constants/builders in `monorail-stream`):

  ```
  monorail.telemetry.<rower_id>.stroke     # per-stroke samples
  monorail.telemetry.<rower_id>.monitor    # ~10 Hz monitor snapshots
  monorail.workout.<rower_id>.event        # start/end/interval boundaries, summaries
  ```

- One stream `MONORAIL` capturing `monorail.>`, file storage, `limits`
  retention with generous age/size caps — DuckDB is the system of record
  (ADR 0006); JetStream is the durable buffer and replay source.
- Publisher behavior on the Pi:
  - Publish with JetStream acks; on NATS outage, buffer to a bounded
    in-memory queue and (if the outage persists) spill to a small on-disk
    ring; drain on reconnect. Workout boundaries flush eagerly.
  - Messages carry a monotonic per-session sequence number plus
    `Nats-Msg-Id` (`<session_id>-<seq>`) for JetStream's deduplication
    window, making redelivery after retries idempotent.
- Consumer: durable pull consumer per sink, explicit acks after the DuckDB
  write commits → at-least-once into the database, deduplicated there by
  `(session_id, seq)` primary key.

## Consequences

- Sink restarts and Pi network blips lose nothing; full-history replay is a
  matter of creating a new consumer at `DeliverPolicy::All`.
- One extra service (nats-server) to run — single binary, trivial systemd
  unit, acceptable.
- At-least-once delivery pushes idempotency into the schema (ADR 0006), which
  we wanted anyway for replay.
- Subject scheme includes `rower_id` from day one: a second erg or a
  friend's Pi joins without renaming streams.
