//! Concept2 PM5 communication (ADR 0003, write side ADR 0010).
//!
//! Split in two layers:
//! - [`csafe`]: pure protocol — frame building/parsing, byte stuffing,
//!   checksums. No I/O; fully unit-testable with golden byte vectors.
//! - [`transport`]: USB HID device discovery and report exchange via
//!   `hidapi`, with timeouts and reconnect-on-unplug.

pub mod csafe;
pub mod transport;

/// Concept2 USB vendor ID.
pub const CONCEPT2_VID: u16 = 0x17A4;
