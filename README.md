# Cardputer Remote & TUI Dashboard

Rust-based terminal dashboard plus a secure remote desktop server for M5Stack
Cardputer clients.

## Features

- Remote desktop server with ECDSA mutual authentication, ECDH session keys,
  AES-128-GCM encryption, replay protection, and mDNS discovery.
- Cross-platform TUI dashboard for CPU, GPU, RAM, disk, network, processes,
  services, Ollama, and settings.
- Windows remote desktop capture/input through native desktop backends.
- Linux remote desktop capture/input through the same Rust API:
  - screen capture: `scrap` first, with `grim`/`maim` fallback on Linux;
  - input injection: `xdotool` runtime backend on Linux.

## Security Setup

The repository `config.toml` is a template and intentionally does not contain
valid secrets. The server refuses all-zero keys and invalid P-256 keys.

Generate real keys and a discovery cookie:

```bash
cargo run --bin keygen
```

Create a local config that is not committed:

```bash
cp config.toml config.local.toml
```

Then copy the generated `[security]` values into `config.local.toml`.

For LAN access, set:

```toml
[network]
bind_address = "0.0.0.0"
```

Use `127.0.0.1` only for local testing.

Run the server with the local config:

```bash
cargo run --release --bin cardputer-remote -- --config config.local.toml
```

## TUI Dashboard

```bash
cargo run --release --bin TUI
```

## Linux Runtime Notes

For Linux remote desktop:

- Build dependencies on Debian/Ubuntu:
  `sudo apt install libxcb1-dev libxcb-shm0-dev libxcb-randr0-dev`
- X11/XWayland capture uses `scrap` first.
- wlroots Wayland: install `grim` for screenshot fallback.
- X11/XWayland input uses `xdotool` at runtime; no `libxdo-dev` link-time
  dependency is required for the default Linux build.

Pure Wayland compositors may require explicit desktop permissions or portal
configuration before capture/input is allowed.

## Cardputer SD Card

Create `/sd/rd_keys/` and copy:

- `client.key`
- `client.pub`
- `server.pub`
- `cookie`

The key generator can write these files directly:

```bash
cargo run --bin keygen -- --output /path/to/sdcard
```

## Verification

```bash
cargo test
cargo check --all-targets
cross check --target x86_64-unknown-linux-gnu --all-targets
```

`Cross.toml` installs the Linux X11 development libraries required by the
remote desktop capture/input backends in the cross container.
