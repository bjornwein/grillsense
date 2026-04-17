# Copilot Instructions for GrillSense

## Build, Test, Lint

```sh
cargo build                          # build
cargo test                           # run all tests
cargo test test_alarm_packet         # run a single test by name
cargo clippy                         # lint (must produce zero warnings)
cargo fmt                            # format (must be clean before commit)
```

**Zero warnings policy**: `cargo clippy` and `cargo fmt -- --check` must both pass clean. Fix all warnings before committing.

## Architecture

This is a reverse-engineered Rust CLI for the GrillSense/Dangrill/Ezon WiFi BBQ thermometer. It communicates with the device over three channels:

- **Cloud REST API** (`cloud.rs`) — HTTPS polling against `smartserver.emaxtime.cn`
- **UDP binary protocol** (`protocol.rs`, `udp.rs`) — 18-byte temp packets and 16-byte alarm packets sent by the device every ~1s
- **LAN AT commands** (`lan.rs`) — UDP port 48899 for device discovery and reconfiguration

The CLI (`main.rs`) is organized into `cloud` and `local` subcommand groups. Both support `--mqtt` flags that enable publishing to Home Assistant via MQTT auto-discovery.

### Module responsibilities

- `protocol.rs` — All protocol constants, packet types, BLE framing, checksums, device ID derivation. The `udp` submodule contains `TempPacket`, `build_alarm_packet()`, `parse_alarm_packet()`.
- `cloud.rs` — Cloud REST API client (login, devices, temperature, alarm).
- `udp.rs` — Bidirectional UDP proxy with packet parsing and forwarding.
- `mqtt.rs` — Hand-rolled MQTT v3.1.1 client with HA auto-discovery. Supports subscribe + publish for bidirectional alarm control.
- `lan.rs` — LAN discovery via UDP broadcast and AT command interface.
- `ble.rs` — BLE provisioning sequence (data structures only, no runtime BLE).
- `main.rs` — CLI definition (clap derive) and command handlers.

### Key protocol details

- **Device ID** is NOT the WiFi MAC. It's derived: `"02" + wifi_mac[4:]`. See `wifi_mac_to_device_id()`.
- **Temperature packets** (18 bytes) use big-endian u16 ÷ 10 for temps.
- **Alarm packets** (16 bytes) use **little-endian** u16 ÷ 10 — note the endianness difference.
- **Checksum**: `(sum(bytes[1..N]) + 0x3C) & 0xFF` where N=16 for temp, N=14 for alarm.
- Full protocol documentation is in `PROTOCOL.md`.

## Conventions

- All personal data (MAC addresses, IPs) in code and git history is anonymized. Cloud server addresses (`smartserver.emaxtime.cn`, `47.52.241.127`) are NOT anonymized.
- The MQTT implementation is hand-rolled (no external MQTT crate) to keep dependencies minimal.
- Modules that contain functions not yet called from `main.rs` use `#![allow(dead_code)]` at the module level.
- The user does NOT have sudo access. Ask before suggesting commands that require elevated privileges.
