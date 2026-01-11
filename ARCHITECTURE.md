# TUI+ Architecture Plan

## Project Overview

**TUI+** - система мониторинга и управления для Windows на базе Rust с интеграцией PowerShell и Ollama.

### Core Principles
1. **PowerShell Integration** - Проброс команд в PowerShell, парсинг и красивое отображение результатов
2. **Modularity** - Каждый монитор (CPU, GPU, RAM, etc.) - отдельный модуль
3. **Hot Reload** - Горячая перезагрузка конфигурации без перезапуска
4. **Performance** - Минимальная задержка при отрисовке, асинхронный сбор данных

---

## Technology Stack

### Core
- **Language**: Rust (Edition 2021)
- **TUI Framework**: `ratatui` (formerly tui-rs)
- **Event Handling**: `crossterm` (кроссплатформенный терминал)
- **Async Runtime**: `tokio` (для асинхронных задач)

### System Monitoring
- **CPU**: PowerShell `Get-Counter` + WMI
- **GPU**: NVML (NVIDIA Management Library) через `nvml-wrapper` crate
- **RAM**: PowerShell `Get-Counter` + WMI
- **Disk**: PowerShell `Get-PhysicalDisk`, `Get-Disk`, WMI
- **Network**: PowerShell `Get-NetAdapterStatistics`, `Get-NetTCPConnection`
- **Processes**: PowerShell `Get-Process` + WMI

### PowerShell Integration
- **Execution**: `std::process::Command` для запуска PowerShell
- **Parsing**: Custom parsers для каждого типа вывода
- **Caching**: In-memory cache для уменьшения частоты вызовов PS

### Ollama Integration
- **Commands**: `ollama list`, `run`, `rm`, `pull`, `ps`, `show`
- **Execution**: Проброс через PowerShell или прямой вызов binary
- **Status Monitoring**: Парсинг `ollama ps` для отслеживания запущенных моделей

### Disk Analysis (Everything integration)
- **Search Engine**: Everything SDK или command-line `es.exe`
- **Indexing**: Использование индекса Everything для быстрого поиска
- **Visualization**: Tree view с сортировкой по размеру

### Configuration
- **Format**: TOML
- **Location**: `./config.toml` (корень проекта)
- **Hot Reload**: `notify` crate для отслеживания изменений файла

---

## Project Structure

```
TUI/
├── Cargo.toml
├── Cargo.lock
├── config.toml                 # Конфигурация приложения
├── README.md
├── DESIGN_CONCEPTS.md
├── ARCHITECTURE.md
│
├── src/
│   ├── main.rs                 # Entry point, app initialization
│   │
│   ├── app/
│   │   ├── mod.rs
│   │   ├── state.rs            # Global application state
│   │   ├── config.rs           # Configuration management + hot reload
│   │   └── tabs.rs             # Tab management (switching, state)
│   │
│   ├── ui/
│   │   ├── mod.rs
│   │   ├── renderer.rs         # Main UI renderer
│   │   ├── theme.rs            # Color schemes and themes
│   │   ├── widgets/
│   │   │   ├── mod.rs
│   │   │   ├── gauge.rs        # Progress bars, gauges
│   │   │   ├── graph.rs        # Line/bar graphs for metrics
│   │   │   ├── table.rs        # Enhanced table widget
│   │   │   ├── tree.rs         # Tree view for disk analyzer
│   │   │   └── radial_menu.rs  # Command selection circle
│   │   │
│   │   └── tabs/
│   │       ├── mod.rs
│   │       ├── cpu.rs          # CPU monitor UI
│   │       ├── gpu.rs          # GPU monitor UI
│   │       ├── ram.rs          # RAM monitor UI
│   │       ├── disk.rs         # Disk monitor UI
│   │       ├── network.rs      # Network monitor UI
│   │       ├── ollama.rs       # Ollama management UI
│   │       ├── processes.rs    # Process list UI
│   │       ├── services.rs     # Windows services UI
│   │       ├── disk_analyzer.rs # Disk space analyzer UI
│   │       └── settings.rs     # Settings UI
│   │
│   ├── monitors/
│   │   ├── mod.rs
│   │   ├── cpu.rs              # CPU data collection
│   │   ├── gpu.rs              # GPU data collection (NVML)
│   │   ├── ram.rs              # RAM data collection
│   │   ├── disk.rs             # Disk data collection
│   │   ├── network.rs          # Network data collection
│   │   ├── processes.rs        # Process monitoring
│   │   └── services.rs         # Windows services monitoring
│   │
│   ├── integrations/
│   │   ├── mod.rs
│   │   ├── powershell/
│   │   │   ├── mod.rs
│   │   │   ├── executor.rs     # PowerShell command execution
│   │   │   ├── parser.rs       # Generic PS output parser
│   │   │   └── commands.rs     # Pre-defined PS commands
│   │   │
│   │   ├── ollama/
│   │   │   ├── mod.rs
│   │   │   ├── client.rs       # Ollama command wrapper
│   │   │   ├── parser.rs       # Ollama output parser
│   │   │   └── models.rs       # Model management
│   │   │
│   │   └── everything/
│   │       ├── mod.rs
│   │       ├── search.rs       # Everything search integration
│   │       └── analyzer.rs     # Disk space analysis logic
│   │
│   ├── events/
│   │   ├── mod.rs
│   │   ├── handler.rs          # Event handler (keyboard, mouse)
│   │   └── input.rs            # Input processing
│   │
│   └── utils/
│       ├── mod.rs
│       ├── format.rs           # Data formatting (bytes, percentages, etc.)
│       ├── cache.rs            # In-memory caching for PS results
│       └── logger.rs           # Logging utilities
│
└── tests/
    ├── integration/
    └── unit/
```

---

## Core Components

### 1. Application State (`app/state.rs`)

```rust
pub struct AppState {
    // Tab management
    pub current_tab: TabType,
    pub tabs: Vec<Tab>,
    pub compact_mode: bool,

    // Monitor data
    pub cpu_data: Arc<RwLock<CpuData>>,
    pub gpu_data: Arc<RwLock<GpuData>>,
    pub ram_data: Arc<RwLock<RamData>>,
    pub disk_data: Arc<RwLock<DiskData>>,
    pub network_data: Arc<RwLock<NetworkData>>,
    pub process_data: Arc<RwLock<ProcessData>>,

    // Ollama integration
    pub ollama_data: Arc<RwLock<OllamaData>>,

    // Configuration
    pub config: Arc<RwLock<Config>>,

    // UI state
    pub command_menu_active: bool,
    pub selected_command: Option<CommandType>,
}

pub enum TabType {
    Cpu,
    Gpu,
    Ram,
    Disk,
    Network,
    Ollama,
    Processes,
    Services,
    DiskAnalyzer,
    Settings,
}
```

### 2. Configuration (`app/config.rs`)

```toml
# config.toml

[general]
app_name = "TUI+"
refresh_rate_ms = 1000
compact_mode = false
theme = "dark"

[tabs]
enabled = ["cpu", "gpu", "ram", "disk", "network", "ollama", "processes", "services"]
default = "cpu"

[monitors.cpu]
enabled = true
refresh_interval_ms = 1000
show_per_core = true
show_frequency = true
show_temperature = true
top_processes_count = 5

[monitors.gpu]
enabled = true
refresh_interval_ms = 1000
use_nvml = true
show_processes = true
show_memory = true
top_processes_count = 3

[monitors.ram]
enabled = true
refresh_interval_ms = 1000
show_breakdown = true
show_pagefile = true
top_processes_count = 5

[monitors.disk]
enabled = true
refresh_interval_ms = 2000
show_health = true
show_temperature = true
show_activity = true

[monitors.network]
enabled = true
refresh_interval_ms = 1000
show_graph = true
graph_duration_seconds = 60
show_connections = true
max_connections = 10

[integrations.ollama]
enabled = true
refresh_interval_ms = 5000
command_timeout_seconds = 30
show_vram_usage = true

[integrations.everything]
enabled = true
es_executable = "C:\\Program Files\\Everything\\es.exe"
max_depth = 10

[ui]
mouse_support = true
tab_switch_key = "Tab"
compact_toggle_key = "F2"
command_menu_key = "Space"
quit_key = "Ctrl+C"

[hotkeys]
cpu = "1"
gpu = "2"
ram = "3"
disk = "4"
network = "5"
ollama = "6"
processes = "7"
services = "8"
disk_analyzer = "9"
settings = "0"

[powershell]
executable = "powershell.exe"
timeout_seconds = 10
use_cache = true
cache_ttl_seconds = 2

[theme.dark]
background = "#1e1e2e"
foreground = "#cdd6f4"
cpu_color = "#f38ba8"
gpu_color = "#94e2d5"
ram_color = "#89b4fa"
disk_color = "#a6e3a1"
network_color = "#f9e2af"
warning_color = "#fab387"
error_color = "#f38ba8"
success_color = "#a6e3a1"
```

### 3. PowerShell Integration (`integrations/powershell/executor.rs`)

```rust
pub struct PowerShellExecutor {
    executable: String,
    timeout: Duration,
    cache: Arc<RwLock<Cache>>,
}

impl PowerShellExecutor {
    pub async fn execute(&self, command: &str) -> Result<String, Error> {
        // Check cache first
        if let Some(cached) = self.cache.read().await.get(command) {
            return Ok(cached.clone());
        }

        // Execute PowerShell command
        let output = Command::new(&self.executable)
            .args(&["-NoProfile", "-NonInteractive", "-Command", command])
            .output()
            .await?;

        let result = String::from_utf8(output.stdout)?;

        // Cache result
        self.cache.write().await.set(command, result.clone());

        Ok(result)
    }

    pub async fn execute_script(&self, script_path: &Path) -> Result<String, Error> {
        let output = Command::new(&self.executable)
            .args(&["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", script_path.to_str().unwrap()])
            .output()
            .await?;

        Ok(String::from_utf8(output.stdout)?)
    }
}
```

### 4. GPU Monitoring with NVML (`monitors/gpu.rs`)

```rust
use nvml_wrapper::Nvml;
use nvml_wrapper::Device;

pub struct GpuMonitor {
    nvml: Nvml,
    device: Device<'static>,
}

impl GpuMonitor {
    pub fn new() -> Result<Self, Error> {
        let nvml = Nvml::init()?;
        let device = nvml.device_by_index(0)?;

        Ok(Self { nvml, device })
    }

    pub async fn collect_data(&self) -> Result<GpuData, Error> {
        let utilization = self.device.utilization_rates()?;
        let memory = self.device.memory_info()?;
        let temperature = self.device.temperature(TemperatureSensor::Gpu)?;
        let power = self.device.power_usage()?;
        let fan_speed = self.device.fan_speed(0)?;
        let clock_info = self.device.clock_info(Clock::Graphics)?;

        // Get running processes
        let processes = self.device.running_graphics_processes()?;

        Ok(GpuData {
            name: self.device.name()?,
            utilization: utilization.gpu,
            memory_used: memory.used,
            memory_total: memory.total,
            temperature,
            power_usage: power / 1000, // mW to W
            fan_speed,
            clock_speed: clock_info,
            processes,
        })
    }
}
```

### 5. Hot Reload Configuration (`app/config.rs`)

```rust
use notify::{Watcher, RecursiveMode, watcher};
use std::sync::mpsc::channel;

pub struct ConfigManager {
    config: Arc<RwLock<Config>>,
    config_path: PathBuf,
}

impl ConfigManager {
    pub fn watch(&self) -> Result<(), Error> {
        let (tx, rx) = channel();
        let mut watcher = watcher(tx, Duration::from_secs(1))?;

        watcher.watch(&self.config_path, RecursiveMode::NonRecursive)?;

        // Spawn watcher thread
        let config = self.config.clone();
        let config_path = self.config_path.clone();

        tokio::spawn(async move {
            loop {
                match rx.recv() {
                    Ok(event) => {
                        if let Ok(new_config) = Config::load(&config_path) {
                            *config.write().await = new_config;
                            log::info!("Configuration reloaded");
                        }
                    }
                    Err(e) => {
                        log::error!("Watch error: {:?}", e);
                        break;
                    }
                }
            }
        });

        Ok(())
    }
}
```

### 6. Radial Command Menu (`ui/widgets/radial_menu.rs`)

```rust
pub struct RadialMenu {
    commands: Vec<CommandItem>,
    selected_index: usize,
    center: (u16, u16),
    radius: u16,
}

pub struct CommandItem {
    pub name: String,
    pub tab: TabType,
    pub angle: f32,
    pub color: Color,
}

impl RadialMenu {
    pub fn new(commands: Vec<CommandItem>) -> Self {
        // Calculate angles for equal distribution
        let angle_step = 360.0 / commands.len() as f32;
        let commands_with_angles = commands
            .into_iter()
            .enumerate()
            .map(|(i, mut cmd)| {
                cmd.angle = i as f32 * angle_step;
                cmd
            })
            .collect();

        Self {
            commands: commands_with_angles,
            selected_index: 0,
            center: (40, 12),
            radius: 10,
        }
    }

    pub fn handle_mouse(&mut self, x: u16, y: u16) {
        // Calculate angle from center to mouse position
        let dx = x as f32 - self.center.0 as f32;
        let dy = y as f32 - self.center.1 as f32;
        let angle = dy.atan2(dx).to_degrees();

        // Find nearest command
        let normalized_angle = if angle < 0.0 { angle + 360.0 } else { angle };

        for (i, cmd) in self.commands.iter().enumerate() {
            let diff = (normalized_angle - cmd.angle).abs();
            if diff < 180.0 / self.commands.len() as f32 {
                self.selected_index = i;
                break;
            }
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        // Render circle segments
        for (i, cmd) in self.commands.iter().enumerate() {
            let is_selected = i == self.selected_index;
            let color = if is_selected {
                Color::Yellow
            } else {
                cmd.color
            };

            // Draw segment
            self.draw_segment(frame, area, cmd, color, is_selected);
        }

        // Render center info
        self.render_center_info(frame, area);
    }
}
```

### 7. Disk Analyzer with Everything (`integrations/everything/analyzer.rs`)

```rust
pub struct DiskAnalyzer {
    es_path: PathBuf,
}

impl DiskAnalyzer {
    pub async fn scan_directory(&self, path: &Path) -> Result<DirectoryTree, Error> {
        // Use Everything to quickly find all files
        let output = Command::new(&self.es_path)
            .args(&[
                "-path", path.to_str().unwrap(),
                "-size",
                "-export-csv",
            ])
            .output()
            .await?;

        let csv_data = String::from_utf8(output.stdout)?;

        // Parse CSV and build tree
        let mut tree = DirectoryTree::new(path);

        for line in csv_data.lines().skip(1) {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() >= 2 {
                let file_path = PathBuf::from(parts[0]);
                let size: u64 = parts[1].parse().unwrap_or(0);
                tree.insert(file_path, size);
            }
        }

        tree.calculate_sizes();
        Ok(tree)
    }
}

pub struct DirectoryTree {
    root: PathBuf,
    nodes: HashMap<PathBuf, DirectoryNode>,
}

pub struct DirectoryNode {
    path: PathBuf,
    size: u64,
    children: Vec<PathBuf>,
    is_expanded: bool,
}
```

---

## Data Flow Architecture

### Async Data Collection Pipeline

```
┌──────────────────────────────────────────────────────────────┐
│                     Main Event Loop                          │
│                      (tokio runtime)                         │
└────────────┬─────────────────────────────────────────────────┘
             │
             ├──► User Input Events (keyboard, mouse)
             │    │
             │    └──► Event Handler ──► Update UI State
             │
             ├──► Monitor Tasks (async spawned)
             │    │
             │    ├──► CPU Monitor Task (1s interval)
             │    │    │
             │    │    └──► PowerShell Execute ──► Parse ──► Update CPU Data
             │    │
             │    ├──► GPU Monitor Task (1s interval)
             │    │    │
             │    │    └──► NVML Query ──► Update GPU Data
             │    │
             │    ├──► RAM Monitor Task (1s interval)
             │    │    │
             │    │    └──► PowerShell Execute ──► Parse ──► Update RAM Data
             │    │
             │    ├──► Disk Monitor Task (2s interval)
             │    │    │
             │    │    └──► PowerShell Execute ──► Parse ──► Update Disk Data
             │    │
             │    ├──► Network Monitor Task (1s interval)
             │    │    │
             │    │    └──► PowerShell Execute ──► Parse ──► Update Network Data
             │    │
             │    └──► Ollama Monitor Task (5s interval)
             │         │
             │         └──► Ollama PS ──► Parse ──► Update Ollama Data
             │
             ├──► Config Watcher Task
             │    │
             │    └──► File Change Event ──► Reload Config
             │
             └──► UI Render Task (60 FPS target)
                  │
                  └──► Read Latest Data ──► Render Frame
```

### State Synchronization

```rust
// All monitor data is wrapped in Arc<RwLock<T>>
// Allows concurrent reads, exclusive writes

// Monitor task (writer)
async fn cpu_monitor_task(state: Arc<RwLock<CpuData>>, ps: PowerShellExecutor) {
    loop {
        let data = collect_cpu_data(&ps).await;
        *state.write().await = data;
        sleep(Duration::from_secs(1)).await;
    }
}

// UI render (reader)
async fn render_cpu_tab(state: Arc<RwLock<CpuData>>) {
    let data = state.read().await;
    // Render using data...
}
```

---

## Development Phases

### Phase 1: Core Foundation (Weeks 1-2)
1. ✅ Project structure setup
2. ✅ Basic TUI framework with ratatui
3. ✅ Tab system implementation
4. ✅ Configuration system (TOML + hot reload)
5. ✅ PowerShell integration foundation
6. ✅ Basic event handling (keyboard, mouse)

### Phase 2: CPU & GPU Monitors (Weeks 3-4)
1. ✅ CPU monitor - Full version
   - PowerShell integration for CPU metrics
   - Per-core usage display
   - Frequency and temperature monitoring
   - Top processes display
2. ✅ GPU monitor - Full version
   - NVML integration
   - GPU utilization and memory
   - Temperature, power, fan speed
   - GPU processes display
3. ✅ Compact versions for both
4. ✅ UI polish and theme application

### Phase 3: RAM, Disk, Network (Weeks 5-6)
1. ✅ RAM monitor
   - Memory breakdown (used, cached, free, etc.)
   - Committed memory tracking
   - Top memory consumers
   - Pagefile monitoring
2. ✅ Disk monitor
   - Multi-drive support
   - Health monitoring (SMART)
   - Real-time I/O statistics
   - Per-process disk activity
3. ✅ Network monitor
   - Interface statistics
   - Traffic graphs
   - Active connections
   - Top bandwidth consumers

### Phase 4: Ollama Integration (Week 7)
1. ✅ Ollama command wrapper
2. ✅ Model list and management UI
3. ✅ Running model monitoring
4. ✅ VRAM usage tracking
5. ✅ Command input/output display
6. ✅ Quick actions (run, stop, delete, pull)

### Phase 5: Advanced Features (Weeks 8-9)
1. ✅ Disk analyzer with Everything integration
   - Directory tree visualization
   - File type distribution
   - Largest files listing
2. ✅ Radial command menu
   - Mouse and keyboard support
   - Visual feedback
3. ✅ Process and Services tabs
4. ✅ Settings UI

### Phase 6: Polish & Optimization (Week 10)
1. ✅ Performance optimization
2. ✅ Error handling improvements
3. ✅ Logging and debugging
4. ✅ Documentation
5. ✅ Testing (unit + integration)
6. ✅ Release preparation

---

## Performance Considerations

### 1. Caching Strategy
- PowerShell calls are expensive (100-500ms each)
- Cache results with TTL (configurable, default 2s)
- Invalidate cache on user-triggered refresh

### 2. Async Execution
- All monitor tasks run concurrently
- Non-blocking UI updates
- Use tokio channels for task communication

### 3. Rendering Optimization
- Target 60 FPS (16ms frame time)
- Only redraw changed components
- Use double buffering (built into crossterm)

### 4. Memory Management
- Limit historical data (e.g., network graphs: 60s max)
- Circular buffers for time-series data
- Lazy loading for disk analyzer

---

## Error Handling

### Levels
1. **Critical**: App cannot continue (exit gracefully)
   - Configuration file missing/corrupt
   - Terminal initialization failure

2. **Warning**: Feature degraded but app continues
   - PowerShell timeout
   - NVML unavailable (fallback to PS for GPU)
   - Everything not installed (manual scan fallback)

3. **Info**: Non-critical issues
   - Cache miss
   - Single metric collection failure

### Strategy
```rust
// Use Result<T, E> extensively
// Display errors in UI, don't crash
match cpu_monitor.collect_data().await {
    Ok(data) => update_state(data),
    Err(e) => {
        log::warn!("CPU data collection failed: {}", e);
        display_error_in_ui("CPU monitor temporarily unavailable");
    }
}
```

---

## Testing Strategy

### Unit Tests
- Each monitor module
- PowerShell parser
- Configuration loader
- Data formatters

### Integration Tests
- End-to-end tab switching
- Configuration hot reload
- PowerShell execution pipeline
- Ollama integration

### Manual Testing
- UI/UX flow
- Performance under load
- Long-running stability (24h+ test)

---

## Dependencies (Cargo.toml)

```toml
[package]
name = "tui-plus"
version = "1.0.0"
edition = "2021"

[dependencies]
# TUI
ratatui = "0.26"
crossterm = "0.27"

# Async runtime
tokio = { version = "1", features = ["full"] }

# Configuration
serde = { version = "1", features = ["derive"] }
toml = "0.8"
notify = "6"

# GPU monitoring
nvml-wrapper = "0.9"

# Utilities
anyhow = "1"
thiserror = "1"
log = "0.4"
env_logger = "0.11"
chrono = "0.4"

# Data structures
parking_lot = "0.12"  # Faster RwLock

[dev-dependencies]
criterion = "0.5"  # Benchmarking
mockall = "0.12"   # Mocking for tests

[profile.release]
opt-level = 3
lto = true
codegen-units = 1
```

---

## Future Enhancements (Post v1.0)

1. **Multi-GPU support**
2. **Custom themes editor**
3. **Export metrics to CSV/JSON**
4. **Remote monitoring (TCP server)**
5. **Alerts and notifications**
6. **Plugin system for custom monitors**
7. **Linux/macOS support**
8. **Web dashboard companion**

---

## Build & Run Instructions

### Prerequisites
```powershell
# Install Rust
winget install Rustlang.Rustup

# Install NVIDIA drivers (for NVML)
# Install Everything (for disk analyzer)
winget install voidtools.Everything
```

### Build
```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release
```

### Run
```bash
# Development
cargo run

# Release
./target/release/tui-plus.exe
```

### Configuration
1. Copy `config.toml.example` to `config.toml`
2. Edit `config.toml` to your preferences
3. Run TUI+ - config will hot reload on save

---

## Contribution Guidelines

### Code Style
- Follow Rust conventions (rustfmt)
- Document public APIs with `///`
- Use descriptive variable names
- Prefer explicit error handling over unwrap/expect

### Commit Messages
```
<type>(<scope>): <subject>

<body>

Examples:
feat(cpu): add per-core frequency monitoring
fix(ollama): parse model names with spaces correctly
docs(readme): add installation instructions
```

### PR Process
1. Fork repo
2. Create feature branch
3. Implement + test
4. Submit PR with description
5. Code review
6. Merge

---

## License

MIT License (or your preference)

---

**End of Architecture Document**
