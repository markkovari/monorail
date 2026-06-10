# ADR 0003: PM5 Connection via USB HID + CSAFE

## Status

Accepted

## Context

The Concept2 PM5 exposes rowing telemetry two ways:

1. **USB** — the PM5 enumerates as a USB HID device (Concept2 vendor ID
   `0x17A4`). Communication uses the CSAFE protocol (frame-based
   request/response) carried in HID reports, including Concept2's proprietary
   CSAFE extension commands (`CSAFE_SETUSERCFG1_CMD` wrapper for PM-specific
   commands) that expose per-stroke data, force curves, and workout state.
2. **Bluetooth Low Energy** — Concept2's published BLE services
   (base UUID `CE06xxxx-43E5-11E4-916C-0800200C9A66`) push rowing status,
   stroke data, and additional metrics as notifications without polling.

Constraints and observations:

- The Pi Zero 2 W sits next to the erg; a cable is acceptable and removes
  BLE pairing/reconnect flakiness and radio contention (the Pi shares one
  antenna between WiFi and BLE — and WiFi is needed for NATS).
- USB is request/response (polling), BLE is push. Polling at 10–25 Hz is
  ample: stroke rate tops out around 40 spm and the PM5 updates internally at
  fixed intervals anyway.
- USB also powers nothing — the PM5 runs on its own batteries/flywheel
  generator; the Pi needs its own supply either way.
- The user explicitly plans a wired (USB-C on the Pi side) setup.

Crate options for HID on Linux/ARM: `hidapi` (C library bindings, mature) vs
`rusb` (libusb, would require re-implementing HID report handling) vs raw
`/dev/hidraw` reads. `hidapi` with the `linux-static-hidraw` feature avoids
detaching kernel drivers and is the path used by most CSAFE implementations
(ErgArcade/PyRow lineage).

## Decision

- Connect to the PM5 over **USB HID** using the **CSAFE protocol**, polling.
- Use the `hidapi` crate (hidraw backend) for transport.
- Implement CSAFE in `monorail-pm5` as two layers:
  - **Protocol layer** (pure, no I/O): frame building/parsing — start/stop
    flags, byte stuffing, checksum, command/response enums, proprietary PM
    command wrapping. Fully unit-testable with golden byte vectors.
  - **Transport layer**: HID device discovery (VID `0x17A4`), report
    read/write with timeouts, reconnect-on-unplug loop.
- Poll cadence: fast loop (~10 Hz) for pace/power/stroke-state, slow loop
  (~1 Hz) for workout totals. Emit a sample only when the PM5's
  elapsed-time/stroke counters advance, so idle periods produce no traffic.
- A udev rule grants the Pi service user access to the hidraw node
  (no root).

BLE support is explicitly out of scope for now but the protocol/transport
split leaves room for a `monorail-pm5-ble` transport later; the protocol
layer differs (BLE uses its own characteristic payloads, not CSAFE), so only
the domain-type mapping would be shared.

## Consequences

- Deterministic wired link; no pairing state machine, no WiFi/BLE antenna
  contention on the Pi Zero 2 W.
- We own a CSAFE implementation: byte stuffing, checksums, PM-proprietary
  commands. Mitigated by the pure protocol layer + golden-vector tests, and
  by abundant reference implementations (Py3Row, ErgArcade docs, Concept2
  CSAFE spec).
- Polling means we choose the sampling resolution; per-stroke force curves
  require explicit retrieval rather than arriving as notifications.
- Tethered: the Pi must live within cable reach of the monitor (it does).
