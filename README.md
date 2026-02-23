# 🖥️ Cardputer Remote & TUI Dashboard

![Rust](https://img.shields.io/badge/Rust-1.75+-orange?style=flat-square&logo=rust)
![Platform](https://img.shields.io/badge/Platform-Windows%20%7C%20Linux-blue?style=flat-square)
![License](https://img.shields.io/badge/License-MIT-green?style=flat-square)

A highly secure, cross-platform remote desktop and system monitoring solution, originally built for the M5Stack Cardputer, featuring a powerful terminal user interface (TUI).

## ✨ Features

- **🔐 Enterprise-Grade Security** 
  - ECDSA Mutual Authentication & ECDH Forward Secrecy.
  - AES-128-GCM Authenticated Encryption with Monotonic Nonce Replay Protection.
- **📊 Advanced TUI Dashboard** (Windows & Linux)
  - Real-time system monitoring (CPU, RAM, Disk, Network, GPU, Processes, Services).
  - Modular cross-platform architecture abstracting OS-level metrics.
- **💻 Integrated Interactive Console**
  - Fully asynchronous terminal emulator inside the TUI.
  - Support for streaming execution and interactive mode logic.
- **🤖 Ollama Integration** 
  - Built-in chat, model management, and offline prompt engineering interface.

## 🚀 Quick Start

### 1. Generate Security Keys
Generates the required PC/Cardputer ECDSA keypairs and a discovery cookie:
```bash
cargo run --bin keygen
```

### 2. Configure PC Server
Generate or modify your `config.toml` to secure the connection and configure system monitors.

### 3. Launch Applications

**To run the TUI Dashboard:**
```bash
cargo run --release --bin TUI
```
*(Note: Omit `--release` for debugging/development)*

**To run the Remote Desktop Server:**
```bash
cargo run --release --bin cardputer-remote
```

## 🛠️ Cardputer (ESP32) Configuration

1. Create a `/sd/rd_keys/` folder on your MicroSD card.
2. Place the generated binaries:
   - `client.key` (ECDSA Private - 32 bytes)
   - `client.pub` (ECDSA Public - 33 bytes)
   - `server.pub` (PC's Public - 33 bytes)
3. Flash the firmware, connect to WiFi, and select "Remote Desktop".

### 🕹️ Cardputer Controls

| Key Bind | Action | | Key Bind | Action |
|----------|--------|-|----------|--------|
| `FN + ;` | Mouse Up | | `FN + ENTER` | Left Click |
| `FN + .` | Mouse Down | | `FN + BACKSPACE` | Disconnect |
| `FN + ,` | Mouse Left | | `A-Z, 0-9` | Type Data |
| `FN + /` | Mouse Right | | | |

## 🏗️ Technical Implementation details

The TUI utilizes a highly asynchronous, cross-platform architecture built on Tokio:
- **Windows Console:** Leverages native `tokio::process::Command` and fully asynchronous I/O streams for non-blocking execution of PowerShell.
- **Linux Console:** Utilizes `portable-pty` bridged into Tokio via `task::spawn_blocking` and `mpsc` channels to ensure the main async event loop is never blocked by synchronous TTY interactions.

---

**Security Notice:** Private keys are stored unencrypted on the SD card. Physical security of the Cardputer is essential. The initial TCP connection is unencrypted until the handshake completes; use on trusted networks.
