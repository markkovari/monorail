# ADR 0001: Record Architecture Decisions

## Status

Accepted

## Context

Monorail is a system that captures live rowing telemetry from a Concept2 PM5
monitor attached to a Raspberry Pi Zero 2 W, streams it over NATS JetStream,
aggregates it in DuckDB, and produces predictions (e.g., split/pace forecasts,
fatigue estimates). The system spans embedded constraints (Pi Zero 2 W: 4×
Cortex-A53, 512 MB RAM), a binary wire protocol (CSAFE), a messaging layer, and
an analytics layer. Decisions made early — protocol choice, serialization,
crate boundaries — are expensive to reverse and need recorded rationale.

## Decision

We will record significant architecture decisions as Architecture Decision
Records (ADRs) in `docs/adr/`, numbered sequentially, using the format:
Status, Context, Decision, Consequences.

A decision is "significant" when it affects crate boundaries, wire formats,
external dependencies, deployment targets, or data retention.

Superseded ADRs are not deleted; their Status is changed to
`Superseded by ADR-NNNN`.

## Consequences

- Rationale survives contributor turnover and long gaps between hacking
  sessions (this is a hobby/side project; context evaporates fast).
- Slight overhead per decision, paid back the first time "why DuckDB and not
  SQLite?" gets asked.
