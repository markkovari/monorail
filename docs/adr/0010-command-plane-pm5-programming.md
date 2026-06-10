# ADR 0010: Command Plane — Pushing Plans to the Pi and Programming the PM5

## Status

Accepted

## Context

ADR 0009 generates plans on the aggregation host; they must reach the erg.
Until now data flowed one way (Pi → server, ADR 0004). Pushing workouts
reverses that: the server needs to deliver a `WorkoutPlan` to the Pi, and the
Pi needs to program the PM5.

What the PM5 can actually be told (CSAFE, over the existing USB HID link of
ADR 0003): Concept2's PM-proprietary CSAFE commands support configuring
programmed workouts — fixed time/distance with split length, and
fixed/variable interval workouts, including target pace per interval. A
segmented plan (10 min @ 2:05 → 20 min @ 2:00 → 10 min @ 1:58) maps onto a
**variable-interval workout** with per-interval durations and target paces.
Limits to respect:

- Stroke-rate targets are *not* enforceable on the PM5; it displays what it
  displays. SPM adherence is ours to measure (compliance scoring, ADR 0009)
  and to surface in any future coaching UI.
- Workout configuration must happen while the PM5 is idle (not mid-piece);
  the PM5 state machine (`CSAFE_GETSTATUS`) gates when programming is legal.
- PM5 interval count and duration granularity are bounded; the plan-to-PM5
  mapping must validate fit and report when a plan can't be represented
  exactly (fallback: program total duration + splits, keep segment targets
  advisory).

Delivery options server → Pi: separate HTTP server on the Pi (new listener,
new auth surface, breaks when Pi is NAT'd), or reuse NATS — the Pi already
holds an outbound NATS connection. NATS gives request/reply with acks for
free and works regardless of how the Pi is networked.

## Decision

- **Reuse NATS as the command plane.** New subjects, defined in
  `monorail-stream` alongside the telemetry subjects (ADR 0004):

  ```
  monorail.command.<rower_id>.plan       # server → Pi: deliver WorkoutPlan
  monorail.command.<rower_id>.control    # server → Pi: cancel/clear
  monorail.status.<rower_id>             # Pi → server: ack/nack, PM5 state, applied plan_id
  ```

  Commands use **core NATS request/reply** (not JetStream): a command is
  only meaningful against the *current* PM5 state — replaying a stale
  "program workout" command hours later is wrong, so durability is
  explicitly not wanted. The sink retries with backoff while the plan is in
  `Scheduled` state; the Pi nacks with a reason (`Pm5Busy`, `Pm5Offline`,
  `PlanDoesNotFit`) when it can't comply. Telemetry keeps its durable
  JetStream path unchanged.
- **Pi-side execution** (`monorail-rower` grows a command handler):
  1. receive plan → validate against PM5 representational limits;
  2. wait for/verify idle PM5 state;
  3. translate segments → CSAFE variable-interval programming sequence
     (new write-side module in `monorail-pm5`, same pure-protocol/transport
     split and golden-vector tests as ADR 0003);
  4. verify by reading the configuration back, then ack with `plan_id`;
  5. stamp `plan_id` into all subsequent workout events for that session so
     telemetry joins back to the plan (compliance scoring, ADR 0009).
- **Plan-to-PM5 mapping is total**: every plan degrades gracefully. Exact
  variable-interval representation when it fits; otherwise single
  time/distance workout with split length, segments advisory-only — the nack
  /ack tells the server which fidelity was achieved
  (`Programmed::Exact | Approximate { reason }`).
- Trust model: single-user LAN deployment; NATS auth (user/password or NKey)
  scoped so the Pi's credentials can only publish/subscribe its own
  `<rower_id>` subjects. No further hardening until exposure changes.

## Consequences

- Bidirectional system with one broker; no new listener or auth surface on
  the Pi.
- Plans physically land on the monitor: athlete sees programmed intervals
  and target pace on the PM5 itself — no extra display hardware needed for
  v1.
- Deliberate non-durability of commands is a sharp edge worth the
  correctness: a Pi offline at push time means the plan stays `Scheduled`
  and is re-pushed when status traffic shows the Pi back, rather than a
  stale command firing from a queue.
- CSAFE write path roughly doubles `monorail-pm5` protocol surface
  (state machine handling, configuration commands, read-back verification);
  golden-vector testing discipline from ADR 0003 carries over.
- SPM targets remain advisory — honest about PM5 limits; live SPM coaching
  needs a future UI ADR.
