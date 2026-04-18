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

The CLI (`main.rs`) is organized into `cloud` and `local` subcommand groups. All modes support `--mqtt` flags that enable publishing to Home Assistant via MQTT auto-discovery.

### Operating modes

1. **Cloud monitor** (`cloud monitor`) — polls vendor cloud REST API, zero-config with autodiscovery
2. **Local monitor** (`local monitor`) — receives UDP packets directly from device (requires device reconfiguration)
3. **Proxy** (`local proxy`) — transparent UDP proxy between device and cloud, taps data for MQTT

All three modes share the same MQTT publishing pattern: HA auto-discovery, alarm subscription, reconnect with backoff, and offline detection.

### Module responsibilities

- `protocol.rs` — All protocol constants, packet types, BLE framing, checksums, device ID derivation, RFC 3339 parsing, staleness detection. The `udp` submodule contains `TempPacket`, `build_alarm_packet()`, `parse_alarm_packet()`.
- `cloud.rs` — Cloud REST API client (login, devices, temperature, alarm). Uses derived device ID (not WiFi MAC) for API calls.
- `udp.rs` — Bidirectional UDP proxy with packet parsing and forwarding.
- `mqtt.rs` — Hand-rolled MQTT v3.1.1 client with HA auto-discovery (9 entities). Supports subscribe + publish for bidirectional alarm control. Cloud bridge runs here; local/proxy bridge runs in `main.rs`.
- `lan.rs` — LAN discovery via UDP broadcast and AT command interface. Supports infinite retry for unmonitored services.
- `ble.rs` — BLE provisioning sequence (data structures only, no runtime BLE).
- `main.rs` — CLI definition (clap derive), command handlers, `mqtt_proxy_publisher()` for local/proxy MQTT bridge.

### Key protocol details

- **Device ID** is NOT the WiFi MAC. It's derived: `"02" + wifi_mac[4:]`. See `wifi_mac_to_device_id()`.
- **Temperature packets** (18 bytes) use **little-endian** u16 ÷ 10 for temps at bytes 12-13 (CH1) and 14-15 (CH2).
- **Alarm packets** (16 bytes) also use **little-endian** u16 ÷ 10 at the same offsets.
- **Keepalive packets** (14 bytes) share the same framing but carry no temperature data. The device only starts sending temperature packets after receiving an echo of its keepalive.
- **Checksum**: `(sum(bytes[1..N]) + 0x3C) & 0xFF` where N=16 for temp, N=12 for keepalive, N=14 for alarm.
- **Cloud staleness**: the `time` field in cloud responses freezes when the device stops sending. After 60s we mark offline in MQTT; after exactly 10 minutes the cloud returns error 101.
- Full protocol documentation is in `PROTOCOL.md`.

## Conventions

- All personal data (MAC addresses, IPs) in code and git history is anonymized. Cloud server addresses (`smartserver.emaxtime.cn`, `47.52.241.127`) are NOT anonymized.
- The MQTT implementation is hand-rolled (no external MQTT crate) to keep dependencies minimal. Uses a separate 30s keepalive timer decoupled from the poll interval.
- Modules that contain functions not yet called from `main.rs` use `#![allow(dead_code)]` at the module level.
- Local timestamps use `libc::localtime_r` (not UTC).
- Licensed under MIT OR Apache-2.0.

## Release Process

1. Bump version in `Cargo.toml`
2. Run `cargo update` to regenerate `Cargo.lock`
3. **Run `cargo fmt`, `cargo clippy`, and `cargo test`** — CI checks these on push
4. Commit and push to master
5. Create a GitHub release (triggers the deploy pipeline):
   ```
   gh release create v0.1.10 --title "v0.1.10" --notes "..."
   ```

The deploy workflow handles everything else automatically:
- Builds multi-arch Docker images (amd64 + aarch64)
- Pushes to `ghcr.io/bjornwein/grillsense/`
- Notifies ha-apps repo via repository dispatch, which auto-updates `grillsense/config.yaml` and `CHANGELOG.md`

### Manual HA App Update
1. Go to Settings → Add-ons
2. Click the "Check for updates" arrow in top right corner
3. "GrillSense Thermometer" should now be marked as "Update available"
4. Click it and then "Update" to install the latest version
