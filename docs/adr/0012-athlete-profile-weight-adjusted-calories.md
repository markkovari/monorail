# ADR 0012: Athlete Profile and Weight-Adjusted Calories

## Status

Accepted

## Context

The PM5 computes calories for a fixed reference athlete of 175 lb / 79.5 kg:

```
PM kcal/hr        = watts × 4 × 0.8604 + 300
```

(`0.8604` is exact physics — 1 watt-hour = 0.8604 kcal — and `×4` models
~25% human mechanical efficiency; `300` is the reference athlete's baseline
burn.) Concept2's published correction replaces the reference baseline with
one scaled to actual body weight, in pounds:

```
true kcal/hr      = PM kcal/hr − 300 + 1.714 × weight_lb
workout kcal      = true kcal/hr × duration_s / 3600
```

(`1.714 × 175 = 300`, so the formulas agree exactly at the reference
weight.) Correct calories therefore require knowing the athlete's weight —
the first piece of athlete state that is not derivable from telemetry.

## Decision

- **Metrics live in `monorail-core::metrics`** as pure functions with the
  constants named and unit-tested against the reference points
  (175 lb ⇒ adjusted = PM; 2:00/500m ⇒ ~203 W). The existing pace↔power
  conversions move here from `monorail-predict` (re-exported there) so all
  Concept2 relations sit in one module usable by UI, sink, and predictors.
- **Athlete profile** (`AthleteProfile { weight_kg }`, SI units in the
  domain; pounds only inside the formula) is a single-row `athlete` table
  (migration 0004), exposed as `GET/PUT /api/v1/athlete`.
- Session summaries gain `duration_s`, `avg_power_watts`, `kcal_pm` (what a
  PM5 would show) and `kcal_adjusted` (`null` until a weight is set) —
  computed at query time from raw samples, never stored (derived data rule,
  ADR 0006).

## Consequences

- Calories become correct for the actual athlete, and the PM-reference
  number stays visible for comparison with the monitor/ErgData.
- Single-athlete assumption is now in the schema (one-row table); a
  multi-athlete future moves weight onto a keyed athlete table and joins via
  `rower_id` — contained change.
- One more thing to set up once (`PUT /api/v1/athlete`); until then adjusted
  calories are honestly `null` rather than silently wrong.
