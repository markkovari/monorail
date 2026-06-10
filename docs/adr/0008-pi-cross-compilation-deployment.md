# ADR 0008: Cross-Compilation and Deployment to the Pi Zero 2 W

## Status

Accepted

## Context

The Pi Zero 2 W (Cortex-A53, 512 MB RAM) cannot realistically compile Rust
workspaces on-device — native builds would take hours and exhaust memory.
Builds must happen on the development machine (Apple Silicon macOS) and ship
as binaries.

Target choice: with a 64-bit Raspberry Pi OS Lite image, the target is
`aarch64-unknown-linux-gnu`. (The Zero 2 W's A53 cores are 64-bit; running
32-bit OS would force the `armv7`/`arm-unknown-linux-gnueabihf` targets for
no benefit.)

Complications:

- `monorail-pm5` links `hidapi` (C); cross-builds need a working C
  cross-toolchain and libudev headers for the target.
- The DuckDB crate must never enter the Pi build graph (ADR 0002/0006) —
  cross-compiling DuckDB's C++ would be painful and pointless.

Toolchain options from macOS: `cross` (Docker-based, prebuilt target images
with C toolchains included) vs `cargo-zigbuild` (zig as cross-linker, no
Docker) vs a hand-rolled GCC sysroot. `cross` is the lowest-friction option
that reliably handles C dependencies like hidapi.

## Decision

- Pi runs **64-bit Raspberry Pi OS Lite**; build target
  **`aarch64-unknown-linux-gnu`**.
- Cross-compile with **`cross`**:
  `cross build --release -p monorail-rower --target aarch64-unknown-linux-gnu`.
  Only the publisher binary is ever built for the Pi target; CI enforces
  that `monorail-rower`'s dependency tree stays DuckDB-free
  (`cargo tree -p monorail-rower -i duckdb` must fail).
- Deployment: `scp`/`rsync` the binary + a `systemd` unit
  (`Restart=always`, runs as a non-root user with udev-granted hidraw
  access per ADR 0003). Config via environment file
  (`/etc/monorail/rower.env`: NATS URL, rower id, poll rates).
- The sink binary (`monorail-sink`) builds natively for the host that runs
  it; no cross-compilation involved.
- A `justfile` (or `make`) target wraps build + deploy so "ship to Pi" is one
  command.

## Consequences

- Sub-minute deploy loop instead of on-device compilation misery.
- Docker becomes a build-time dependency on the dev machine (for `cross`);
  acceptable. If it grates, `cargo-zigbuild` is a drop-in retry — the hidapi
  link step is the only thing to re-verify.
- 64-bit-only decision documented; anyone flashing a 32-bit image will get a
  binary that won't run — README must state the required OS image.
- systemd + `Restart=always` + the reconnect loops in `monorail-pm5`/
  publisher (ADRs 0003/0004) make the Pi appliance-like: plug in, row.
