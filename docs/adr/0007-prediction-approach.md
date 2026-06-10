# ADR 0007: Prediction — Classical Models over DuckDB Features

## Status

Accepted

## Context

The point of aggregating is prediction: e.g., projected 2k/5k times, pace
fade within a piece, expected splits given stroke rate and recent training
load, plateau/fatigue trends. Data volume is small (one athlete; thousands
of strokes per session, hundreds of sessions per year). Deep-learning
toolchains (tch/burn + training infra) are wildly out of proportion; the
leverage is in feature engineering over the stroke history, which DuckDB
already does well (ADR 0006).

Domain priors also exist: rowing physiology has established models (power ∝
pace⁻³ relationship of the Concept2 flywheel, critical-power / W′ models,
Riegel-style endurance scaling) that need only a handful of fitted
parameters.

## Decision

- Predictions live in `monorail-predict`, consuming **typed feature frames
  from `monorail-store`** — never raw SQL, never raw NATS messages.
- Start with classical, inspectable models implemented directly in Rust
  (with `linfa`/`ndarray` where regression utilities help):
  1. physics/physiology-grounded parametric fits (critical power from recent
     maximal efforts; Riegel exponent fitted to the athlete's own history);
  2. simple regression on engineered features (rolling training load,
     stroke-rate/pace curves) for split forecasting;
  3. residual monitoring so model error is visible per session.
- Model parameters are stored back into DuckDB (`model_fit` table) with
  fit timestamp and training-data watermark, so predictions are reproducible
  and refits are incremental.
- Trait `Predictor` abstracts model families; the prediction *interface*
  (inputs: feature frame; outputs: typed prediction + confidence) is the
  stable contract, the model behind it is replaceable.

## Consequences

- Models are explainable and debuggable with SQL alone; no GPU, no Python
  sidecar, runs inside the sink binary.
- Ceiling on model sophistication is accepted: if fancier ML is ever
  warranted, the `Predictor` trait and DuckDB feature pipeline remain, and a
  Python/ONNX path becomes a superseding ADR rather than a rewrite.
- Honest accuracy tracking (residuals table) from day one prevents
  prediction theater.
