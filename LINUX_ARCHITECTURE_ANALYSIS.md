# Linux TUI Architecture Analysis

## Executive Summary

TUI+ is a cross-platform terminal-based system monitoring application built in Rust. Originally Windows-centric (PowerShell-driven), it has evolved into a dual-platform system with native Linux support via `/proc`, `/sys`, and standard CLI tools. The architecture follows a clean **Monitor → Data → UI** pipeline with async Tokio tasks, `parking_lot` shared state, and `ratatui` rendering.

---

## 1. High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        src/bin/TUI.rs                           │
│                     (Entry point, event loop)                   │
└─────────────────────────┬───────────────────────────────────────┘
                          │
┌─────────────────────────▼───────────────────────────────────────┐
│                     src/app/mod.rs                               │
│  App { state: AppState, config_manager: Option<ConfigManager> } │
│  - Config loading (tui-config.toml, hot-reload via notify)      │
│  - Event dispatch (crossterm → state mutations)                 │
└──┬──────────────────┬───────────────────┬───────────────────────┘
   │                  │                   │
   ▼                  ▼                   ▼
┌────────┐    ┌──────────────┐    ┌──────────────┐
│ Monitors│   │  AppState    │    │    UI Layer   │
│ (async) │──▶│ (shared data)│──▶ │  (ratatui)   │
└────────┘    └──────────────┘    └──────────────┘
```

### Data Flow

1. **Monitor tasks** (Tokio spawned) poll system data at configurable intervals
2. Data is written to `Arc<RwLock<Option<T>>>` fields in `AppState`
3. **UI render** reads the shared state each frame and draws widgets
4. **Input handler** mutates UI state (focus, scroll, sort) without touching monitor data

---

## 2. Module Map

### Core Modules (`src/`)

| Module | Purpose |
|--------|---------|
| `lib.rs` | Crate root, re-exports public API |
| `app/` | Application lifecycle, state, config, tabs |
| `monitors/` | Platform-agnostic monitor trait + type definitions |
| `platform/` | Platform-specific implementations (Linux, Windows) |
| `integrations/` | External tool wrappers (Ollama, re-exports) |
| `ui/` | Ratatui rendering (tabs, widgets, theme) |
| `utils/` | Formatting, ANSI processing, JSON helpers |
| `events/` | Crossterm event processing |
| `input/` | Keyboard/mouse input handling |
| `config/` | Config file types |
| `network/` | Network server for Cardputer remote |
| `crypto/` | Encryption for remote sessions |
| `protocol/` | Wire protocol for remote desktop |
| `capture/` | Screen capture for remote desktop |

### Platform Abstraction Pattern

```
src/monitors/network.rs          # Conditional re-export
  └─ #[cfg(linux)]  → monitors/linux/network.rs   (LinuxNetworkMonitor)
  └─ #[cfg(windows)] → monitors/windows/network.rs (WindowsNetworkMonitor)

src/monitors/linux/network.rs    # Uses platform::linux::network
  └─ platform/linux/network.rs   # Raw /proc parsing, ip commands
  └─ platform/linux/network_diagnostics.rs  # Advanced diagnostic tools
```

This two-tier design separates **monitor logic** (history tracking, speed calculation) from **platform data collection** (reading /proc files).

---

## 3. Linux-Specific Data Sources

### Network Monitoring (`platform/linux/network.rs`)

| Data Source | What It Provides |
|-------------|-----------------|
| `/proc/net/dev` | Per-interface RX/TX byte counters |
| `/proc/net/tcp`, `/proc/net/tcp6` | TCP connection state (inode → PID mapping) |
| `/proc/net/udp`, `/proc/net/udp6` | UDP socket state |
| `/proc/net/route` | IPv4 default gateway |
| `/proc/net/if_inet6` | IPv6 addresses |
| `/sys/class/net/<iface>/` | Link speed, MTU, duplex, MAC, operstate |
| `/sys/class/net/<iface>/device/` | PCI vendor/device, driver, bus info |
| `/proc/<pid>/fd/` + `readlink` | Socket inode → PID ownership mapping |
| `/proc/<pid>/comm` | Process name for socket owner |
| `/etc/resolv.conf` | DNS nameservers |
| `ip -o -4 addr show` | IPv4 addresses per interface |
| `ip -o -6 addr show` | IPv6 addresses per interface |
| `ip -6 route show default` | IPv6 default gateways |
| `resolvectl dns` | systemd-resolved DNS servers |
| `ss -tinpH` | Per-socket bytes_sent/bytes_received |
| `ping` | Latency, packet loss, MTU probing (DF bit) |

### CPU Monitoring (`platform/linux/cpu.rs`)

| Data Source | What It Provides |
|-------------|-----------------|
| `/proc/stat` | Per-core CPU times (user, system, idle, etc.) |
| `/proc/cpuinfo` | CPU model name, core count |
| `/sys/devices/system/cpu/cpu*/cpufreq/` | Per-core frequency (current, min, max) |
| `/sys/class/thermal/thermal_zone*/temp` | CPU temperature |
| `/sys/class/powercap/intel-rapl:*/` | RAPL energy counters (power draw) |

### Memory Monitoring (`platform/linux/memory.rs`)

| Data Source | What It Provides |
|-------------|-----------------|
| `/proc/meminfo` | Total, free, available, cached, buffers, swap |
| `/sys/block/zram*/` | Zram device stats (compression ratio, usage) |
| `dmidecode` | Physical DIMM info (type, speed, slots) |

### Disk Monitoring (`platform/linux/disk.rs`)

| Data Source | What It Provides |
|-------------|-----------------|
| `/proc/diskstats` | Read/write ops, sectors, IO time per block device |
| `/sys/block/*/` | Device model, rotational flag, queue depth |
| `/proc/mounts` | Filesystem mount points |
| `statvfs` | Filesystem capacity and usage |
| `smartctl` | SMART health, temperature, wear level |

### Process Monitoring (`platform/linux/process.rs`)

| Data Source | What It Provides |
|-------------|-----------------|
| `/proc/<pid>/stat` | CPU time, threads, state |
| `/proc/<pid>/status` | Memory (VmRSS), UID |
| `/proc/<pid>/cmdline` | Full command line |
| `/proc/<pid>/io` | IO read/write bytes |

### Service Monitoring (`platform/linux/services.rs`)

| Data Source | What It Provides |
|-------------|-----------------|
| `systemctl list-units` | Service name, status, description |
| `systemctl start/stop/restart` | Service control |
| `systemctl enable/disable` | Startup type changes |

---

## 4. Monitor Architecture

### Trait System (`monitors/traits.rs`)

Each monitor implements an async trait:

```rust
pub trait NetworkMonitorTrait: Send + Sync {
    async fn collect_data(&self) -> Result<NetworkData>;
}
```

All seven monitor traits follow this pattern:
- `CpuMonitorTrait` → `CpuData`
- `GpuMonitorTrait` → `GpuData`
- `RamMonitorTrait` → `RamData`
- `DiskMonitorTrait` → `DiskData`
- `NetworkMonitorTrait` → `NetworkData`
- `ProcessMonitorTrait` → `ProcessData`
- `ServiceMonitorTrait` → `ServiceData`

### Monitor Task Spawning (`app/monitors_task.rs`)

`spawn_monitor_tasks()` creates one Tokio task per monitor:

```
┌──────────┐  loop {                    ┌──────────────────┐
│ CPU Task │───read config──────────────▶│ Arc<RwLock<Cfg>> │
│          │   create/update monitor    └──────────────────┘
│          │───collect_data()──────────▶ Arc<RwLock<Option<CpuData>>>
│          │   sleep(refresh_interval)
│          │  }
└──────────┘
```

Each task:
1. Reads config each iteration (hot-reload awareness)
2. Recreates the monitor if settings changed (PowerShell executor params)
3. Calls `collect_data()` and stores result in the shared `Arc<RwLock<>>`
4. Stores errors in a parallel `Arc<RwLock<Option<String>>>` error slot
5. Sleeps for the configured refresh interval

### Linux Network Monitor Deep Dive (`monitors/linux/network.rs`)

The `LinuxNetworkMonitor` struct maintains:

```rust
pub struct LinuxNetworkMonitor {
    linux_sys: LinuxSysMonitor,               // Platform data collector
    traffic_history: Mutex<VecDeque<TrafficSample>>,  // 60-sample ring buffer
    per_iface_history: Mutex<HashMap<String, VecDeque<TrafficSample>>>,
    last_network_stats: Mutex<Option<(Instant, HashMap<String, (u64, u64)>)>>,
    last_process_stats: Mutex<Option<(Instant, HashMap<u32, (u64, u64)>)>>,
    peak_interface_speeds: Mutex<HashMap<String, (f64, f64)>>,
}
```

**Speed calculation**: Delta bytes between polls divided by elapsed time, converted to Mbps.

**Data collection pipeline** per tick:
1. `get_network_interfaces()` → IP/MAC info
2. `get_network_interfaces_stats()` → sysfs details + traffic counters
3. Calculate per-interface download/upload speeds from deltas
4. Merge IP info, compute peak speeds
5. Sort interfaces (active+gateway first)
6. `get_network_connections()` → parse `/proc/net/tcp*`, `/proc/net/udp*`
7. `get_process_bandwidth()` → `ss -tinpH` or fallback to socket queue parsing
8. Update traffic history ring buffers

---

## 5. Network Diagnostics Engine (`platform/linux/network_diagnostics.rs`)

A comprehensive async diagnostics framework supporting 12 operations:

| Operation | Description |
|-----------|-------------|
| `Resolve` | DNS name resolution via `ToSocketAddrs` |
| `DnsExplain` | Full DNS configuration dump |
| `RouteInspect` | Routing table + policy rules |
| `NicDeepInfo` | Deep interface inspection (ethtool, wifi) |
| `ConnectionLab` | Filtered connection analysis with extended metrics |
| `Ping` | Configurable ping with profiles (Quick/Latency/Loss) |
| `Trace` | Traceroute with ICMP/UDP/TCP protocol support |
| `MtuProbe` | Binary search for Path MTU using DF-bit pings |
| `PortScan` | TCP connect scan on specified ports |
| `NatCapabilityCheck` | NAT type detection |
| `MappingTest` | NAT port mapping test (TCP/UDP) |
| `ExportReport` | JSON/Markdown report generation |

Each operation uses a request/result pattern with an event-driven architecture:
- `NetworkDiagnosticsEngine` processes `NetworkDiagnosticsRequest` messages
- Results are streamed back via `NetworkDiagnosticsEvent` through an unbounded channel
- The UI polls events in `AppState::apply_async_updates()`

---

## 6. UI Architecture

### Tab System (`app/tabs.rs`)

11 tabs available: CPU, GPU, RAM, Disk, DiskAnalyzer, Network, Ollama, Processes, Services, Console, Settings.

Tabs are configurable via `tui-config.toml` and can be reordered/disabled at runtime.

### Rendering Pipeline (`ui/`)

```
ui/mod.rs              # Top-level render dispatcher
  └─ ui/tabs/cpu.rs    # render(f, area, app)
  └─ ui/tabs/network.rs
  └─ ui/tabs/...
  └─ ui/theme.rs       # Color scheme from config
  └─ ui/widgets/       # Reusable widgets (graph, radial_menu)
```

Each tab's `render()` function:
1. Reads shared data via `app.state.<monitor>_data.read()`
2. Checks for errors, shows error/loading state
3. Renders using ratatui widgets (Table, Paragraph, Sparkline, Block, etc.)

### Network Tab Layout (Full Mode)

```
┌─────────────────────────────────────────────────────────┐
│ HEADER: iface status │ ↓DL ↑UL │ GW │ Conns │ RX TX   │ ← 3 rows
├────────┬──────────────┬─────────────────────────────────┤
│ TOOLS  │   CENTER     │        RESULTS                  │
│ (22w)  │  Interface/  │  Summary/Details/Raw/Advice/    │ ← Min 16 rows
│ DNS    │  Connections  │  History tabs                   │
│ Routing│  (switchable) │                                 │
│ Traffic│              │                                 │
│ NAT    │              │                                 │
│ Report │              │                                 │
├────────┴──────┬───────┴─────────────────────────────────┤
│  PARAMETERS   │            ACTIVITY                     │ ← 8 rows
│  (38%)        │            (62%)                        │
├───────────────┴─────────────────────────────────────────┤
│ HELP BAR                                                │ ← 1 row
└─────────────────────────────────────────────────────────┘
```

**Focus zones** cycle with Tab/Shift-Tab: Tools → Parameters → Interface → Results → Activity

### Network UI State (`app/state.rs`)

```rust
pub struct NetworkUIState {
    pub focus: NetworkFocusZone,        // Current keyboard focus
    pub result_tab: NetworkResultTab,   // Summary/Details/Raw/Advice/History
    pub center_view: NetworkCenterView, // Interface or Connections
    pub input_mode: bool,               // Text input active
    pub target_input: String,           // Current input value
    pub selected_tool: NetworkDiagnosticTool,
    pub running_job: Option<u64>,       // Active diagnostic job ID
    pub event_log: VecDeque<String>,    // Diagnostic event stream
    pub detail_lines: Vec<String>,      // Parsed result details
    pub raw_stdout/stderr: Vec<String>, // Raw command output
    pub advice_lines: Vec<String>,      // Generated advice
    pub result_history: VecDeque<NetworkDiagHistoryEntry>,
    pub traffic_marker: Option<TrafficMarker>, // RX/TX delta tracking
    // ... scroll offsets, filter state
}
```

---

## 7. State Management

### Shared State Pattern

All monitor data uses the same pattern:
```rust
pub struct AppState {
    pub cpu_data: Arc<RwLock<Option<CpuData>>>,
    pub cpu_error: Arc<RwLock<Option<String>>>,
    // ... repeat for each monitor
}
```

- **Writers**: Monitor tasks (one per resource type)
- **Readers**: UI render functions (main thread)
- **Synchronization**: `parking_lot::RwLock` (non-poisoning, faster than std)

### Async Update Channel

For non-monitor async results (console output, Ollama chat, diagnostics):
```rust
async_tx: UnboundedSender<AsyncUpdate>,
async_rx: UnboundedReceiver<AsyncUpdate>,
```

The main event loop calls `apply_async_updates()` each tick to drain the channel.

---

## 8. Configuration System

### Hot Reload

```
tui-config.toml  ──(notify crate watches)──▶  ConfigManager
                                                    │
                                    increments version counter
                                                    │
                             AppState::apply_config_updates()
                                    reads on each tick
```

Config changes take effect without restart: tab list, refresh intervals, compact mode, console settings.

### Monitor Config Example

```toml
[monitors.network]
enabled = true
refresh_interval_ms = 2000
show_graph = true
graph_duration_seconds = 60
show_connections = true
max_connections = 100
```

---

## 9. Key Design Patterns

### 1. Conditional Compilation
```rust
#[cfg(target_os = "linux")]
pub use crate::monitors::linux::network::LinuxNetworkMonitor as NetworkMonitor;
```
The rest of the codebase uses `NetworkMonitor` without knowing the platform.

### 2. Integration Re-exports
`integrations/mod.rs` re-exports `LinuxSysMonitor` from `platform::linux`, maintaining backward compatibility as code was reorganized.

### 3. Graceful Fallbacks
Network data collection has multiple fallback paths:
- `ss -tinpH` → fallback to `/proc/net/tcp` queue parsing
- `resolvectl` → fallback to `/etc/resolv.conf`
- IPv6 from `ip` command → fallback to `/proc/net/if_inet6`

### 4. Sorted Deterministic Output
Interfaces are scored and sorted (active+gateway first), connections are ranked by state (ESTABLISHED first), consumers sorted by speed.

### 5. Ring Buffer History
Traffic samples use `VecDeque` with capacity 60, popping front when full — natural sparkline/graph data source.

---

## 10. Connection Table Parsing (`/proc/net/tcp`)

Linux `/proc/net/tcp` format:
```
  sl  local_address rem_address   st tx_queue rx_queue ... inode
   0: 0100007F:0035 00000000:0000 0A 00000000:00000000 ... 12345
```

The parser:
1. Splits whitespace fields
2. Parses hex IP (little-endian for IPv4) and port
3. Maps TCP state codes (01=ESTABLISHED, 0A=LISTEN, etc.)
4. Matches inode to PID via `/proc/<pid>/fd/` symlink scanning
5. Truncates to 512 connections, sorted by state rank

---

## 11. Strengths

- **Zero external dependencies for monitoring** — uses /proc and /sys directly
- **Efficient delta-based speed calculation** — no shell-out for throughput
- **Comprehensive diagnostics** — 12 network tools with async execution
- **Clean platform abstraction** — platform code isolated behind traits
- **Hot-reloadable config** — no restart needed for most changes
- **Non-blocking UI** — all monitoring is async, UI never waits for data

## 12. Areas for Improvement

- **`collect_socket_owners()` is expensive** — scans all `/proc/<pid>/fd/` every call; could be cached with TTL
- **No GPU support on Linux** — `GpuMonitor` Linux impl appears to be a stub
- **`PowerShellExecutor` leak into Linux** — monitor constructors still take `PowerShellExecutor` param even on Linux (it's unused)
- **Missing TCP health metrics** — no retransmit, RTT, or congestion window data from `/proc/net/tcp` extended fields or `ss -ti` output
- **Single-threaded socket owner scan** — could be parallelized with rayon for systems with many PIDs
- **No cgroup/container awareness** — process monitoring doesn't account for containerized processes
