# TUI+ System Monitor

Advanced cross-platform system monitoring TUI (Text User Interface) application built with Rust, featuring PowerShell integration for Windows and Ollama LLM management.

## Features

### ✅ Implemented (v1.0)

- **Core Architecture**
  - Rust-based TUI using `ratatui` framework
  - Asynchronous data collection with `tokio`
  - Hot-reloadable TOML configuration
  - Modular monitor system

- **UI/UX**
  - Tab-based navigation with custom highlighting (Variant B)
    - Normal: `[CPU]` white
    - Selected: `(CPU)` yellow
  - Compact and full view modes (toggle with F2)
  - Command history system with circular menu (Ctrl+F)
  - Arrow key navigation (Up/Down) for command history

- **Monitoring Tabs**
  - CPU Monitor (Full implementation with mock data)
  - GPU Monitor (Framework ready)
  - RAM Monitor (Framework ready)
  - Disk Monitor (Framework ready)
  - Network Monitor (Framework ready)
  - Ollama Manager (Framework ready)

- **Integrations**
  - PowerShell command execution
  - Ollama LLM management (framework)

## Requirements

- Rust 1.70+ (Edition 2021)
- Windows (primary target)
- PowerShell 5.1+ or PowerShell Core 7+
- Optional: NVIDIA GPU for GPU monitoring

## Installation

\`\`\`bash
cargo build --release
./target/release/tui-plus
\`\`\`

## Usage

**Keyboard Shortcuts:**
- `Ctrl+C` - Exit
- `Tab/Shift+Tab` - Navigate tabs
- `F2` - Toggle compact mode
- `Ctrl+F` - Command history menu
- `↑/↓` - Navigate history
- `1-9,0` - Jump to tab

## Configuration

Edit `config.toml` to customize settings.

See [DESIGN_CONCEPTS.md](DESIGN_CONCEPTS.md) and [ARCHITECTURE.md](ARCHITECTURE.md) for more details.
