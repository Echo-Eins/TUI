# TUI+ System Monitor

Advanced cross-platform system monitoring TUI built with Rust. Windows uses
PowerShell integration, and the UI includes an Ollama model manager.

## Features
- Core architecture
  - Rust TUI using ratatui
  - Async data collection with tokio
  - Hot-reloadable TOML configuration
  - Modular monitor system
- UI/UX
  - Tab-based navigation with custom highlighting
  - Compact and full view modes (toggle with F2)
  - Command history radial menu (Ctrl+F)
  - Keyboard navigation with throttled input
- Monitoring tabs
  - CPU: usage, per-core, frequency, power
  - GPU: utilization, VRAM, temperature, processes
  - RAM: totals, speed, usage
  - Disk: multi-drive, I/O stats, partitions
  - Network: interface stats and traffic history
  - Processes: sorting and paging
  - Services: list + details panel with scroll
  - Disk Analyzer: Everything integration for root folder sizes
- Ollama manager
  - Model list + running models
  - Chat mode with pause/resume
  - Pull/delete/run/stop actions
  - VRAM usage summary
  - Recent Activity with log metadata

## Requirements
- Rust 1.70+ (Edition 2021)
- Windows (primary target)
- PowerShell 5.1+ or PowerShell Core 7+
- Optional: NVIDIA GPU for GPU monitoring

## Installation
```bash
cargo build --release
./target/release/tui-plus
```

## Usage
Keyboard shortcuts:
- Ctrl+C: Exit
- Tab/Shift+Tab: Navigate tabs
- F2: Toggle compact mode
- Ctrl+F: Command history menu
- Up/Down: Navigate lists/history
- 1-9,0: Jump to tab

## Logging
Logs are written to logs/tui-plus.log by default. To override the log path:
```bash
TUI_PLUS_LOG=path/to/custom.log cargo run
```

## Configuration
Edit config.toml to customize settings.

See DESIGN_CONCEPTS.md and ARCHITECTURE.md for more details.
