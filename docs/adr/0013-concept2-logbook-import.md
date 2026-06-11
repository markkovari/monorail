# ADR 0013: Concept2 Logbook (ErgData) Import

## Status

Accepted

## Context

ErgData and the PM5 sync workout results to the Concept2 Logbook cloud
(`log.concept2.com`), which exposes a REST API (Bearer token,
`GET /api/users/me/results`, paginated). Importing those results gives us:

1. **Calorie ground truth**: the calories ErgData recorded, to compare
   against our own weight-adjusted computation (ADR 0012);
2. **History backfill**: workouts rowed before monorail existed, or rowed
   away from the Pi, still enter the training history that prediction models
   fit on (ADR 0007).

Constraints: third-party API with rate limits and its own schema; results
are summaries (no per-stroke data); authentication is a user-supplied OAuth
access token.

## Decision

- New crate **`monorail-logbook`**: a read-only Logbook API client
  (`reqwest` with rustls; the only crate allowed to speak HTTP to Concept2).
  Tolerant deserialization — unknown fields ignored, optional fields
  defaulted — with checked-in fixture JSON as schema documentation, same
  discipline as ADR 0005.
- Imported results land in their own raw table `logbook_result`
  (migration 0005), keyed by the Logbook's own result id, upserted
  idempotently, raw JSON kept lossless. They are **never** merged into
  `monitor_sample`/`stroke` — different fidelity, different provenance;
  queries that want both join explicitly.
- Sync is **manual**: `POST /api/v1/logbook/sync` fetches everything new and
  reports the count. Token comes from `MONORAIL_LOGBOOK_TOKEN`; without it
  the endpoint answers 503. Background polling is a later decision once
  manual sync proves the mapping.
- OAuth refresh flows are out of scope: the user pastes a token. If/when
  tokens expiring becomes annoying, a refresh-token flow is an amendment.

## Consequences

- ErgData-recorded calories become queryable next to our computed ones —
  disagreement is visible instead of latent.
- Prediction models can fit on full history, not just monorail-era sessions.
- One outbound internet dependency appears in the sink; it is isolated to a
  crate, optional at runtime, and absent from the Pi binary entirely.
- Logbook schema drift surfaces as fixture-test failures, not silent
  misparses.
