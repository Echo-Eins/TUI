use crate::platform::executor::CommandExecutor;
use anyhow::Result;
use chrono::Local;
use crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent,
    MouseEventKind,
};
use crossterm::terminal;
use parking_lot::RwLock;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use super::{monitors_task, Config, ConfigManager, TabManager, TabType};
use crate::integrations::ollama::{OllamaModel, RunningModel};
use crate::integrations::{ChatLogMetadata, OllamaClient, OllamaData, PowerShellExecutor};
use crate::monitors::{
    CpuData, DiskAnalyzerData, DiskData, GpuData, NetworkData, ProcessData, RamData, ServiceData,
};
use crate::platform::executor::StreamMessage;
#[cfg(target_os = "linux")]
use crate::platform::linux::network_diagnostics as linux_netdiag;
use std::fs;

#[derive(Debug)]
enum AsyncUpdate {
    ConsoleStdout { block_id: u64, line: String },
    ConsoleStderr { block_id: u64, line: String },
    ConsoleCompleted { block_id: u64, exit_code: i32 },
    ConsoleInterrupted { block_id: u64 },
    ConsoleFailed { block_id: u64, error: String },
    OllamaCommandCompleted { title: String, lines: Vec<String> },
    OllamaChatCompleted { response: String },
    OllamaChatFailed { error: String },
    ErrorExplanation { block_id: u64, text: String },
    ErrorExplanationFailed { block_id: u64, error: String },
}

fn session_status_label(status: crate::app::extensions::SessionStatus) -> &'static str {
    match status {
        crate::app::extensions::SessionStatus::Running => "running",
        crate::app::extensions::SessionStatus::Paused => "paused",
        crate::app::extensions::SessionStatus::Finished => "finished",
        crate::app::extensions::SessionStatus::Quit => "quit",
    }
}

pub struct AppState {
    pub config: Arc<RwLock<Config>>,
    pub tab_manager: TabManager,
    pub compact_mode: bool,

    // Monitor data
    pub cpu_data: Arc<RwLock<Option<CpuData>>>,
    pub cpu_error: Arc<RwLock<Option<String>>>,
    pub gpu_data: Arc<RwLock<Option<GpuData>>>,
    pub gpu_error: Arc<RwLock<Option<String>>>,
    pub ram_data: Arc<RwLock<Option<RamData>>>,
    pub ram_error: Arc<RwLock<Option<String>>>,
    pub disk_data: Arc<RwLock<Option<DiskData>>>,
    pub disk_error: Arc<RwLock<Option<String>>>,
    pub disk_analyzer_data: Arc<RwLock<Option<DiskAnalyzerData>>>,
    pub disk_analyzer_error: Arc<RwLock<Option<String>>>,
    pub network_data: Arc<RwLock<Option<NetworkData>>>,
    pub network_error: Arc<RwLock<Option<String>>>,
    pub process_data: Arc<RwLock<Option<ProcessData>>>,
    pub process_error: Arc<RwLock<Option<String>>>,
    pub service_data: Arc<RwLock<Option<ServiceData>>>,
    pub service_error: Arc<RwLock<Option<String>>>,

    // Ollama integration
    pub ollama_data: Arc<RwLock<Option<OllamaData>>>,
    pub ollama_error: Arc<RwLock<Option<String>>>,

    // UI state
    pub console_state: crate::app::console_state::ConsoleState,
    pub history: crate::app::history::CommandHistory,
    pub suggestion_engine: crate::app::suggestions::SuggestionEngine,
    console_extensions: crate::app::extensions::ConsoleCommandRouter,
    console_executor: Arc<dyn CommandExecutor>,
    #[allow(dead_code)]
    pub selected_section: Option<String>,
    pub last_nav_input: Option<Instant>,
    pub last_horizontal_nav_input: Option<Instant>,
    pub last_sort_input: Option<Instant>,
    pub last_widget_scroll_input: Option<Instant>,
    pub last_view_toggle_input: Option<Instant>,
    pub last_text_input: Option<Instant>,
    pub terminal_size: (u16, u16),
    last_console_session_tick: Instant,

    // CPU UI state
    pub cpu_state: CpuUIState,

    // GPU UI state
    pub gpu_state: GpuUIState,

    // RAM UI state
    pub ram_state: RamUIState,

    // Network UI state
    pub network_ui_state: NetworkUIState,

    // Processes UI state
    pub processes_state: ProcessesUIState,

    // Services UI state
    pub services_state: ServicesUIState,

    // Ollama UI state
    pub ollama_state: OllamaUIState,

    #[cfg(target_os = "linux")]
    network_diag_engine: Arc<linux_netdiag::NetworkDiagnosticsEngine>,
    #[cfg(target_os = "linux")]
    network_diag_rx: UnboundedReceiver<linux_netdiag::NetworkDiagnosticsEvent>,

    async_tx: UnboundedSender<AsyncUpdate>,
    async_rx: UnboundedReceiver<AsyncUpdate>,
    last_config_version: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CpuProcessSortColumn {
    Pid,
    Name,
    Cpu,
    Memory,
    Threads,
}

pub struct CpuUIState {
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub sort_column: CpuProcessSortColumn,
    pub sort_ascending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ProcessSortColumn {
    Pid,
    Name,
    Cpu,
    Memory,
    Threads,
    User,
}

pub struct ProcessesUIState {
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub sort_column: ProcessSortColumn,
    pub sort_ascending: bool,
    pub filter: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ServiceSortColumn {
    Name,
    DisplayName,
    Status,
    StartType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatusFilter {
    All,
    Running,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GpuProcessSortColumn {
    Pid,
    Name,
    Gpu,
    Memory,
    Type,
}

pub struct GpuUIState {
    pub selected_index: usize,
    pub sort_column: GpuProcessSortColumn,
    pub sort_ascending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RamPanelFocus {
    Breakdown,
    TopProcesses,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RamProcessSortColumn {
    Pid,
    Name,
    WorkingSet,
    PrivateBytes,
}

pub struct RamUIState {
    pub focused_panel: RamPanelFocus,
    pub selected_index: usize,
    pub sort_column: RamProcessSortColumn,
    pub sort_ascending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkDiagnosticTool {
    Resolve,
    DnsExplain,
    RouteInspect,
    NicDeepInfo,
    ConnectionLab,
    Ping,
    Trace,
    MtuProbe,
    PortScan,
    NatCapability,
    NatMappingTest,
    ExportReport,
}

impl NetworkDiagnosticTool {
    pub const ORDERED: [Self; 12] = [
        Self::Resolve,
        Self::DnsExplain,
        Self::RouteInspect,
        Self::NicDeepInfo,
        Self::ConnectionLab,
        Self::Ping,
        Self::Trace,
        Self::MtuProbe,
        Self::PortScan,
        Self::NatCapability,
        Self::NatMappingTest,
        Self::ExportReport,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Resolve => "Resolve",
            Self::DnsExplain => "DNS Explain",
            Self::RouteInspect => "Route Inspect",
            Self::NicDeepInfo => "NIC Deep Info",
            Self::ConnectionLab => "Connection Lab",
            Self::Ping => "Ping",
            Self::Trace => "Trace",
            Self::MtuProbe => "MTU Probe",
            Self::PortScan => "Port Scan",
            Self::NatCapability => "NAT Capability",
            Self::NatMappingTest => "NAT Mapping Test",
            Self::ExportReport => "Export Report",
        }
    }

    fn previous(self) -> Self {
        let idx = Self::ORDERED
            .iter()
            .position(|tool| *tool == self)
            .unwrap_or(0);
        if idx == 0 {
            Self::ORDERED[Self::ORDERED.len() - 1]
        } else {
            Self::ORDERED[idx - 1]
        }
    }

    fn next(self) -> Self {
        let idx = Self::ORDERED
            .iter()
            .position(|tool| *tool == self)
            .unwrap_or(0);
        Self::ORDERED[(idx + 1) % Self::ORDERED.len()]
    }
}

// ---- Network UI: Focus zones, result tabs, center view, traffic marker ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkFocusZone {
    Tools,
    Interface,
    Results,
    Parameters,
    Activity,
}

impl NetworkFocusZone {
    const CYCLE: [Self; 5] = [
        Self::Tools,
        Self::Parameters,
        Self::Interface,
        Self::Results,
        Self::Activity,
    ];

    pub fn next(self) -> Self {
        let idx = Self::CYCLE.iter().position(|z| *z == self).unwrap_or(0);
        Self::CYCLE[(idx + 1) % Self::CYCLE.len()]
    }

    pub fn prev(self) -> Self {
        let idx = Self::CYCLE.iter().position(|z| *z == self).unwrap_or(0);
        if idx == 0 {
            Self::CYCLE[Self::CYCLE.len() - 1]
        } else {
            Self::CYCLE[idx - 1]
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkResultTab {
    Summary,
    Details,
    Raw,
    Advice,
    History,
}

impl NetworkResultTab {
    pub const TABS: [Self; 5] = [
        Self::Summary,
        Self::Details,
        Self::Raw,
        Self::Advice,
        Self::History,
    ];

    pub fn next(self) -> Self {
        let idx = Self::TABS.iter().position(|t| *t == self).unwrap_or(0);
        Self::TABS[(idx + 1) % Self::TABS.len()]
    }

    pub fn prev(self) -> Self {
        let idx = Self::TABS.iter().position(|t| *t == self).unwrap_or(0);
        if idx == 0 {
            Self::TABS[Self::TABS.len() - 1]
        } else {
            Self::TABS[idx - 1]
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Summary => "Summary",
            Self::Details => "Details",
            Self::Raw => "Raw",
            Self::Advice => "Advice",
            Self::History => "History",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkCenterView {
    Interface,
    Connections,
}

/// Marker for RX/TX traffic counter reset
#[derive(Debug, Clone)]
pub struct TrafficMarker {
    pub bytes_received_at_mark: u64,
    pub bytes_sent_at_mark: u64,
}

/// Category grouping for the tool navigator
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCategory {
    Dns,
    Routing,
    Interfaces,
    Traffic,
    Nat,
    Reporting,
}

impl ToolCategory {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dns => "DNS",
            Self::Routing => "Routing",
            Self::Interfaces => "Interfaces",
            Self::Traffic => "Traffic",
            Self::Nat => "NAT",
            Self::Reporting => "Reporting",
        }
    }
}

impl NetworkDiagnosticTool {
    pub fn category(self) -> ToolCategory {
        match self {
            Self::Resolve | Self::DnsExplain => ToolCategory::Dns,
            Self::RouteInspect | Self::Trace | Self::MtuProbe => ToolCategory::Routing,
            Self::NicDeepInfo => ToolCategory::Interfaces,
            Self::ConnectionLab | Self::Ping | Self::PortScan => ToolCategory::Traffic,
            Self::NatCapability | Self::NatMappingTest => ToolCategory::Nat,
            Self::ExportReport => ToolCategory::Reporting,
        }
    }

    /// Parameter presets for quick configuration (label, value)
    pub fn presets(self) -> Vec<(&'static str, &'static str)> {
        match self {
            Self::Resolve => vec![
                ("Google DNS", "8.8.8.8"),
                ("Cloudflare", "1.1.1.1"),
                ("localhost", "localhost"),
            ],
            Self::DnsExplain => vec![],
            Self::RouteInspect => vec![
                ("Default", ""),
                ("Google", "8.8.8.8"),
                ("Cloudflare", "1.1.1.1"),
            ],
            Self::NicDeepInfo => vec![("All", "")],
            Self::ConnectionLab => vec![
                ("TCP Established", "proto=tcp state=estab"),
                ("TCP Listen", "proto=tcp state=listen"),
                ("All UDP", "proto=udp"),
                ("All (limit 500)", "limit=500"),
            ],
            Self::Ping => vec![
                ("Quick x5", "8.8.8.8 profile=quick count=5"),
                ("Latency x20", "8.8.8.8 profile=latency count=20"),
                ("Loss Test", "8.8.8.8 profile=loss count=50"),
                ("Cloudflare", "1.1.1.1 profile=latency count=10"),
            ],
            Self::Trace => vec![
                ("ICMP Default", "8.8.8.8"),
                ("TCP:443", "8.8.8.8 proto=tcp port=443"),
                ("UDP", "8.8.8.8 proto=udp"),
                ("Cloudflare TCP", "1.1.1.1 proto=tcp port=443"),
            ],
            Self::MtuProbe => vec![("Google", "8.8.8.8"), ("Cloudflare", "1.1.1.1")],
            Self::PortScan => vec![
                ("Common Web", "localhost:22,80,443,8080,8443"),
                ("Databases", "localhost:3306,5432,6379,27017"),
                (
                    "Full Scan",
                    "localhost:21,22,25,53,80,110,143,443,993,3306,5432,8080",
                ),
            ],
            Self::NatCapability => vec![],
            Self::NatMappingTest => vec![
                ("TCP 8080", "tcp 8080 8080 120"),
                ("UDP 9000", "udp 9000 9000 120"),
            ],
            Self::ExportReport => vec![],
        }
    }
}

pub struct NetworkUIState {
    // Focus & navigation
    pub focus: NetworkFocusZone,
    pub result_tab: NetworkResultTab,
    pub center_view: NetworkCenterView,

    // Tool selection & input
    pub input_mode: bool,
    pub target_input: String,
    pub selected_tool: NetworkDiagnosticTool,
    pub tools_scroll_offset: usize,

    // Job state
    pub running_job: Option<u64>,
    pub nat_mapping_confirm_until: Option<Instant>,
    pub last_job: Option<u64>,
    pub last_summary: String,
    pub last_error: Option<String>,

    // Output
    pub event_log: VecDeque<String>,
    pub detail_lines: Vec<String>,
    pub detail_scroll: usize,
    pub raw_stdout: Vec<String>,
    pub raw_stderr: Vec<String>,
    pub advice_lines: Vec<String>,
    pub result_history: VecDeque<NetworkDiagHistoryEntry>,

    // Connections scroll
    pub connections_scroll: usize,
    pub bandwidth_scroll: usize,

    // Activity scroll
    pub activity_scroll: usize,

    // Traffic marker for RX/TX toggle
    pub traffic_marker: Option<TrafficMarker>,
    pub show_marker_traffic: bool,

    // Interface selector (for multi-interface view)
    pub selected_interface_idx: usize,

    // Result filter
    pub filter_active: bool,
    pub filter_input: String,
}

/// Entry in the diagnostics result history
#[derive(Debug, Clone)]
pub struct NetworkDiagHistoryEntry {
    pub job_id: u64,
    pub tool_label: String,
    pub target: String,
    pub summary: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServicesPanelFocus {
    Table,
    Details,
}

pub struct ServicesUIState {
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub sort_column: ServiceSortColumn,
    pub sort_ascending: bool,
    pub status_filter: ServiceStatusFilter,
    pub focused_panel: ServicesPanelFocus,
    pub details_scroll: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OllamaView {
    Models,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OllamaModelSortColumn {
    Name,
    Params,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OllamaRunningSortColumn {
    Name,
    Params,
    PausedAt,
    MessageCount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OllamaPanelFocus {
    Main,
    Vram,
    Activity,
    Additions,
    Help,
    Input,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OllamaInputMode {
    None,
    Pull,
    Command,
    Chat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OllamaActivityView {
    List,
    Log,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct ChatSession {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub chat_scroll: usize,
    pub prompt_buffer: String,
    pub prompt_scroll: usize,
    pub prompt_height: u16,
    pub paused_at: u64,
    pub paused_at_display: String,
}

pub struct OllamaUIState {
    pub selected_model_index: usize,
    pub selected_running_index: usize,
    pub current_view: OllamaView,
    pub focused_panel: OllamaPanelFocus,
    pub input_mode: OllamaInputMode,
    pub input_buffer: String,
    pub chat_active: bool,
    pub active_chat_model: Option<String>,
    pub chat_messages: Vec<ChatMessage>,
    pub chat_scroll: usize,
    pub activity_view: OllamaActivityView,
    pub activity_selected: usize,
    pub activity_log_scroll: usize,
    pub activity_log_lines: Vec<String>,
    pub activity_log_title: String,
    pub activity_expand_started_at: Option<Instant>,
    pub activity_expand_row: Option<usize>,
    pub activity_expand_suppressed: bool,
    pub activity_additions_open: bool,
    pub activity_additions_selected: usize,
    pub model_sort_column: OllamaModelSortColumn,
    pub model_sort_ascending: bool,
    pub running_sort_column: OllamaRunningSortColumn,
    pub running_sort_ascending: bool,
    pub running_summary_scroll: usize,
    pub chat_prompt_height: u16,
    pub chat_prompt_scroll: usize,
    pub paused_chats: Vec<ChatSession>,
    pub pending_delete: Option<OllamaDeleteTarget>,
    pub show_delete_confirm: bool,
    pub chat_pending: bool,
    pub command_pending: bool,
}

#[derive(Debug, Clone)]
pub enum OllamaDeleteTarget {
    Model(String),
    ChatLog(crate::integrations::ollama::ChatLogEntry),
}

impl AppState {
    fn update_terminal_size(&mut self, cols: u16, rows: u16) {
        self.terminal_size = (cols, rows);
        if self.ollama_state.input_mode == OllamaInputMode::Chat {
            let desired = self.suggested_chat_prompt_height(rows);
            self.ollama_state.chat_prompt_height = desired;
            let max_scroll = self.max_chat_prompt_scroll();
            self.ollama_state.chat_prompt_scroll =
                self.ollama_state.chat_prompt_scroll.min(max_scroll);
        }
    }
    fn allow_nav(&mut self) -> bool {
        Self::allow_with_throttle(&mut self.last_nav_input, Duration::from_millis(120))
    }

    pub fn apply_config_updates(&mut self, manager: Option<&ConfigManager>) {
        let Some(manager) = manager else {
            return;
        };

        let version = manager.version();
        if version == self.last_config_version {
            return;
        }

        self.last_config_version = version;
        let config_snapshot = self.config.read().clone();
        let current_tab = self.tab_manager.current();
        let mut new_tabs = TabManager::new(
            config_snapshot.tabs.enabled.clone(),
            &config_snapshot.tabs.default,
        );
        new_tabs.select(current_tab);
        self.tab_manager = new_tabs;
        self.compact_mode = config_snapshot.general.compact_mode;

        // Console config hot-reload
        self.console_state.max_blocks = config_snapshot.console.history_limit;
        self.console_state.status_threshold_ms = config_snapshot.console.status_threshold_ms;
        self.console_state.status_persist_ms = config_snapshot.console.status_persist_ms;
        self.console_state.enable_ai_explain = config_snapshot.console.enable_ai_explain;
    }

    pub fn apply_async_updates(&mut self) {
        // Read config once per tick for inner loops
        let max_lines = self.config.read().console.max_output_lines;

        while let Ok(update) = self.async_rx.try_recv() {
            match update {
                AsyncUpdate::ConsoleStdout { block_id, line } => {
                    if let Some(block) = self.console_state.get_block_mut(block_id) {
                        block.push_line(
                            crate::app::console_state::OutputLine::stdout(line),
                            max_lines,
                        );
                    }
                }
                AsyncUpdate::ConsoleStderr { block_id, line } => {
                    if let Some(block) = self.console_state.get_block_mut(block_id) {
                        block.push_line(
                            crate::app::console_state::OutputLine::stderr(line),
                            max_lines,
                        );
                    }
                }
                AsyncUpdate::ConsoleCompleted {
                    block_id,
                    exit_code,
                } => {
                    // Record to history before completing
                    if let Some(block) = self.console_state.get_block(block_id) {
                        let cmd = block.input.clone();
                        let cwd = block.cwd.clone();
                        let elapsed = block.elapsed_ms() as i64;
                        let hostname = self.console_state.hostname.clone();
                        let _ = self.history.record(
                            &cmd,
                            &cwd,
                            Some(exit_code),
                            Some(elapsed),
                            &hostname,
                        );
                    }
                    self.console_state.complete_block(block_id, exit_code);

                    // Detect permission failure and set sudo hint
                    if exit_code != 0 {
                        if let Some(block) = self.console_state.get_block(block_id) {
                            let stderr =
                                crate::app::sudo::collect_stderr_tail(&block.output_lines, 10);
                            let cmd = block.input.clone();
                            if crate::app::sudo::detect_permission_failure(exit_code, &stderr)
                                && !crate::app::sudo::is_blacklisted(&cmd)
                                && !cmd.starts_with("sudo ")
                            {
                                if let Some(block) = self.console_state.get_block_mut(block_id) {
                                    block.sudo_hint = true;
                                }
                            }

                            if !stderr.trim().is_empty() {
                                if let Some(block) = self.console_state.get_block_mut(block_id) {
                                    block.explain_hint = true;
                                }
                            }
                        }
                    }
                }
                AsyncUpdate::ConsoleInterrupted { block_id } => {
                    if let Some(block) = self.console_state.get_block(block_id) {
                        let cmd = block.input.clone();
                        let cwd = block.cwd.clone();
                        let elapsed = block.elapsed_ms() as i64;
                        let hostname = self.console_state.hostname.clone();
                        let _ =
                            self.history
                                .record(&cmd, &cwd, Some(130), Some(elapsed), &hostname);
                    }
                    self.console_state.interrupt_block(block_id);
                }
                AsyncUpdate::ConsoleFailed { block_id, error } => {
                    // Record failed command to history
                    if let Some(block) = self.console_state.get_block(block_id) {
                        let cmd = block.input.clone();
                        let cwd = block.cwd.clone();
                        let elapsed = block.elapsed_ms() as i64;
                        let hostname = self.console_state.hostname.clone();
                        let _ = self
                            .history
                            .record(&cmd, &cwd, None, Some(elapsed), &hostname);
                    }

                    if let Some(block) = self.console_state.get_block_mut(block_id) {
                        block.fail(error);
                        // Direct execution failures are explainable by AI as well.
                        block.explain_hint = true;
                    }
                    if self.console_state.active_block_id == Some(block_id) {
                        self.console_state.active_block_id = None;
                    }
                }
                AsyncUpdate::OllamaCommandCompleted { title, lines } => {
                    self.ollama_state.activity_view = OllamaActivityView::Log;
                    self.ollama_state.activity_log_lines = lines;
                    self.ollama_state.activity_log_title = title;
                    self.ollama_state.activity_log_scroll = 0;
                    self.ollama_state.focused_panel = OllamaPanelFocus::Activity;
                    self.ollama_state.command_pending = false;
                    self.close_activity_additions();
                }
                AsyncUpdate::OllamaChatCompleted { response } => {
                    self.ollama_state.chat_pending = false;
                    if self.ollama_state.chat_active && !response.is_empty() {
                        self.ollama_state.chat_messages.push(ChatMessage {
                            role: ChatRole::Assistant,
                            text: response,
                        });
                        self.ollama_state.chat_scroll = usize::MAX;
                    }
                }
                AsyncUpdate::OllamaChatFailed { error } => {
                    self.ollama_state.chat_pending = false;
                    if self.ollama_state.chat_active {
                        self.ollama_state.chat_messages.push(ChatMessage {
                            role: ChatRole::Assistant,
                            text: format!("Error: {error}"),
                        });
                        self.ollama_state.chat_scroll = usize::MAX;
                    }
                }
                AsyncUpdate::ErrorExplanation { block_id, text } => {
                    if let Some(block) = self.console_state.get_block_mut(block_id) {
                        block.explanation = Some(text);
                        block.is_explaining = false;
                    }
                }
                AsyncUpdate::ErrorExplanationFailed { block_id, error } => {
                    if let Some(block) = self.console_state.get_block_mut(block_id) {
                        block.explanation = Some(format!("Error explaining: {}", error));
                        block.is_explaining = false;
                    }
                }
            }
        }

        #[cfg(target_os = "linux")]
        while let Ok(event) = self.network_diag_rx.try_recv() {
            self.apply_network_diag_event(event);
        }
    }
    fn allow_horizontal_nav(&mut self) -> bool {
        Self::allow_with_throttle(
            &mut self.last_horizontal_nav_input,
            Duration::from_millis(180),
        )
    }

    fn allow_sort_toggle(&mut self) -> bool {
        Self::allow_with_throttle(&mut self.last_sort_input, Duration::from_millis(200))
    }

    fn allow_view_toggle(&mut self) -> bool {
        Self::allow_with_throttle(&mut self.last_view_toggle_input, Duration::from_millis(200))
    }

    #[cfg(target_os = "linux")]
    fn push_network_log(&mut self, line: String) {
        const MAX_LOG_LINES: usize = 80;
        if self.network_ui_state.event_log.len() >= MAX_LOG_LINES {
            self.network_ui_state.event_log.pop_front();
        }
        let ts = Local::now().format("%H:%M:%S").to_string();
        self.network_ui_state
            .event_log
            .push_back(format!("[{ts}] {line}"));
    }

    #[cfg(target_os = "linux")]
    fn apply_network_diag_event(&mut self, event: linux_netdiag::NetworkDiagnosticsEvent) {
        match event {
            linux_netdiag::NetworkDiagnosticsEvent::Started { job } => {
                self.network_ui_state.running_job = Some(job.id);
                self.network_ui_state.last_error = None;
                self.push_network_log(format!(
                    "Job #{} started: {}",
                    job.id,
                    network_operation_label(job.operation)
                ));
            }
            linux_netdiag::NetworkDiagnosticsEvent::Progress { job_id, message } => {
                self.push_network_log(format!("Job #{}: {}", job_id, message));
            }
            linux_netdiag::NetworkDiagnosticsEvent::Completed { job_id, result } => {
                if self.network_ui_state.running_job == Some(job_id) {
                    self.network_ui_state.running_job = None;
                }
                self.network_ui_state.nat_mapping_confirm_until = None;
                self.network_ui_state.last_job = Some(job_id);
                self.network_ui_state.last_summary = result.summary();
                self.network_ui_state.last_error = None;
                self.network_ui_state.detail_lines = network_result_detail_lines(&result);
                self.network_ui_state.detail_scroll = 0;

                // Populate raw and advice tabs
                let (raw_out, raw_err) = network_result_raw_lines(&result);
                self.network_ui_state.raw_stdout = raw_out;
                self.network_ui_state.raw_stderr = raw_err;
                self.network_ui_state.advice_lines = network_result_advice_lines(&result);

                // Push to result history
                let timestamp = Local::now().format("%H:%M:%S").to_string();
                let entry = NetworkDiagHistoryEntry {
                    job_id,
                    tool_label: self.network_ui_state.selected_tool.label().to_string(),
                    target: self.network_ui_state.target_input.clone(),
                    summary: self.network_ui_state.last_summary.clone(),
                    timestamp,
                };
                if self.network_ui_state.result_history.len() >= 64 {
                    self.network_ui_state.result_history.pop_front();
                }
                self.network_ui_state.result_history.push_back(entry);

                self.push_network_log(format!(
                    "Job #{} completed: {}",
                    job_id, self.network_ui_state.last_summary
                ));
                for detail in network_result_log_lines(&result) {
                    self.push_network_log(format!("  {detail}"));
                }
            }
            linux_netdiag::NetworkDiagnosticsEvent::Failed { job_id, error } => {
                if self.network_ui_state.running_job == Some(job_id) {
                    self.network_ui_state.running_job = None;
                }
                self.network_ui_state.nat_mapping_confirm_until = None;
                self.network_ui_state.last_job = Some(job_id);
                let hint_text = error
                    .hint
                    .clone()
                    .unwrap_or_else(|| "No additional hint".to_string());
                self.network_ui_state.last_error = Some(match error.hint.clone() {
                    Some(hint) => format!("{} ({hint})", error.message),
                    None => error.message.clone(),
                });
                self.network_ui_state.detail_lines = vec![
                    format!("Job #{job_id} failed"),
                    format!("Code: {:?}", error.code),
                    format!("Message: {}", error.message),
                    format!("Hint: {hint_text}"),
                ];
                self.network_ui_state.detail_scroll = 0;
                if let Some(err) = &self.network_ui_state.last_error {
                    self.push_network_log(format!("Job #{} failed: {}", job_id, err));
                }
            }
            linux_netdiag::NetworkDiagnosticsEvent::Cancelled { job_id } => {
                if self.network_ui_state.running_job == Some(job_id) {
                    self.network_ui_state.running_job = None;
                }
                self.network_ui_state.nat_mapping_confirm_until = None;
                self.network_ui_state.detail_lines = vec![format!("Job #{job_id} cancelled")];
                self.network_ui_state.detail_scroll = 0;
                self.push_network_log(format!("Job #{} cancelled", job_id));
            }
        }
    }

    fn start_selected_network_diagnostic(&mut self) {
        #[cfg(target_os = "linux")]
        {
            if self.network_ui_state.running_job.is_some() {
                self.network_ui_state.last_error =
                    Some("Another diagnostics job is already running".to_string());
                return;
            }

            let tool = self.network_ui_state.selected_tool;
            let input = self.network_ui_state.target_input.trim().to_string();

            if !matches!(tool, NetworkDiagnosticTool::NatMappingTest) {
                self.network_ui_state.nat_mapping_confirm_until = None;
            }

            let request_bundle = match tool {
                NetworkDiagnosticTool::Resolve => {
                    if input.is_empty() {
                        Err("Resolve target is empty".to_string())
                    } else {
                        Ok((
                            linux_netdiag::NetworkDiagnosticsRequest::Resolve(
                                linux_netdiag::ResolveRequest { query: input },
                            ),
                            Duration::from_secs(10),
                        ))
                    }
                }
                NetworkDiagnosticTool::DnsExplain => Ok((
                    linux_netdiag::NetworkDiagnosticsRequest::DnsExplain(
                        linux_netdiag::DnsExplainRequest {
                            include_gateways: true,
                        },
                    ),
                    Duration::from_secs(12),
                )),
                NetworkDiagnosticTool::RouteInspect => {
                    let target = if input.is_empty() { None } else { Some(input) };
                    Ok((
                        linux_netdiag::NetworkDiagnosticsRequest::RouteInspect(
                            linux_netdiag::RouteInspectRequest {
                                target,
                                include_policy_rules: true,
                            },
                        ),
                        Duration::from_secs(15),
                    ))
                }
                NetworkDiagnosticTool::NicDeepInfo => {
                    let interface = if input.is_empty() { None } else { Some(input) };
                    Ok((
                        linux_netdiag::NetworkDiagnosticsRequest::NicDeepInfo(
                            linux_netdiag::NicDeepInfoRequest {
                                interface,
                                include_stats: true,
                                include_wifi: true,
                            },
                        ),
                        Duration::from_secs(25),
                    ))
                }
                NetworkDiagnosticTool::ConnectionLab => {
                    let (protocol_filter, state_filter, limit) = parse_connection_lab_input(&input);
                    Ok((
                        linux_netdiag::NetworkDiagnosticsRequest::ConnectionLab(
                            linux_netdiag::ConnectionLabRequest {
                                protocol_filter,
                                state_filter,
                                limit,
                                include_extended_metrics: true,
                            },
                        ),
                        Duration::from_secs(18),
                    ))
                }
                NetworkDiagnosticTool::Ping => {
                    if input.is_empty() {
                        Err("Ping target is empty".to_string())
                    } else {
                        match parse_ping_diag_input(&input) {
                            Ok(request) => {
                                let timeout = if request.continuous {
                                    Duration::from_secs(request.deadline_secs.max(3) as u64 + 6)
                                } else {
                                    Duration::from_secs(
                                        request
                                            .count
                                            .max(1)
                                            .saturating_mul(request.timeout_secs.max(1))
                                            .saturating_add(6)
                                            as u64,
                                    )
                                };
                                Ok((
                                    linux_netdiag::NetworkDiagnosticsRequest::Ping(request),
                                    timeout,
                                ))
                            }
                            Err(error) => Err(error),
                        }
                    }
                }
                NetworkDiagnosticTool::Trace => {
                    if input.is_empty() {
                        Err("Trace target is empty".to_string())
                    } else {
                        match parse_trace_diag_input(&input) {
                            Ok(request) => {
                                let attempts = if request.enable_fallback { 3u64 } else { 1u64 };
                                let timeout = Duration::from_secs(
                                    (request.max_hops as u64)
                                        .saturating_mul(request.timeout_secs as u64)
                                        .saturating_mul(attempts)
                                        .saturating_add(10)
                                        .clamp(20, 180),
                                );
                                Ok((
                                    linux_netdiag::NetworkDiagnosticsRequest::Trace(request),
                                    timeout,
                                ))
                            }
                            Err(error) => Err(error),
                        }
                    }
                }
                NetworkDiagnosticTool::MtuProbe => {
                    if input.is_empty() {
                        Err("MTU target is empty".to_string())
                    } else {
                        Ok((
                            linux_netdiag::NetworkDiagnosticsRequest::MtuProbe(
                                linux_netdiag::MtuProbeRequest { target: input },
                            ),
                            Duration::from_secs(35),
                        ))
                    }
                }
                NetworkDiagnosticTool::PortScan => {
                    let (target, ports) = parse_network_scan_input(&input);
                    if target.is_empty() {
                        Err("Port scan target is empty".to_string())
                    } else {
                        let timeout = Duration::from_millis((ports.len() as u64 * 500).max(6_000));
                        Ok((
                            linux_netdiag::NetworkDiagnosticsRequest::PortScan(
                                linux_netdiag::PortScanRequest {
                                    target,
                                    ports,
                                    timeout_ms: 450,
                                },
                            ),
                            timeout,
                        ))
                    }
                }
                NetworkDiagnosticTool::NatCapability => Ok((
                    linux_netdiag::NetworkDiagnosticsRequest::NatCapabilityCheck(
                        linux_netdiag::NatCapabilityRequest { timeout_secs: 8 },
                    ),
                    Duration::from_secs(28),
                )),
                NetworkDiagnosticTool::NatMappingTest => {
                    let now = Instant::now();
                    let confirm_active = self
                        .network_ui_state
                        .nat_mapping_confirm_until
                        .is_some_and(|until| until > now);
                    if !confirm_active {
                        self.network_ui_state.nat_mapping_confirm_until =
                            Some(now + Duration::from_secs(10));
                        Err(
                            "NAT mapping test is active and potentially sensitive. Press Enter again within 10s to confirm."
                                .to_string(),
                        )
                    } else {
                        match parse_nat_mapping_input(&input) {
                            Ok((protocol, internal_port, external_port, ttl_seconds)) => {
                                self.network_ui_state.nat_mapping_confirm_until = None;
                                Ok((
                                    linux_netdiag::NetworkDiagnosticsRequest::MappingTest(
                                        linux_netdiag::NatMappingTestRequest {
                                            protocol,
                                            internal_port,
                                            external_port,
                                            ttl_seconds,
                                            require_confirmation: true,
                                        },
                                    ),
                                    Duration::from_secs(35),
                                ))
                            }
                            Err(error) => {
                                self.network_ui_state.nat_mapping_confirm_until = None;
                                Err(error)
                            }
                        }
                    }
                }
                NetworkDiagnosticTool::ExportReport => Ok((
                    linux_netdiag::NetworkDiagnosticsRequest::ExportReport(
                        linux_netdiag::ExportReportRequest {
                            format: linux_netdiag::ReportFormat::Json,
                            max_entries: 32,
                        },
                    ),
                    Duration::from_secs(6),
                )),
            };

            match request_bundle {
                Ok((request, timeout)) => {
                    let job_id = self.network_diag_engine.start(request, timeout);
                    self.network_ui_state.running_job = Some(job_id);
                    self.network_ui_state.last_error = None;
                    self.push_network_log(format!("Queued {} as job #{}", tool.label(), job_id));
                }
                Err(error) => {
                    self.network_ui_state.last_error = Some(error.clone());
                    self.push_network_log(format!("Cannot run {}: {}", tool.label(), error));
                }
            }
            return;
        }

        #[cfg(not(target_os = "linux"))]
        {
            self.network_ui_state.last_error =
                Some("Interactive diagnostics is available only on Linux".to_string());
        }
    }

    fn cancel_network_diagnostic(&mut self) {
        #[cfg(target_os = "linux")]
        {
            if let Some(job_id) = self.network_ui_state.running_job {
                if self.network_diag_engine.cancel(job_id) {
                    self.push_network_log(format!("Cancel requested for job #{}", job_id));
                } else {
                    self.push_network_log(format!("Job #{} was not active", job_id));
                }
                self.network_ui_state.running_job = None;
            } else {
                self.push_network_log("No active diagnostics job to cancel".to_string());
            }
            return;
        }

        #[cfg(not(target_os = "linux"))]
        {
            self.network_ui_state.last_error =
                Some("Interactive diagnostics is available only on Linux".to_string());
        }
    }

    fn network_set_traffic_marker(&mut self) {
        let network_data = self.network_data.read();
        if let Some(data) = network_data.as_ref() {
            if let Some(iface) = data.interfaces.first() {
                self.network_ui_state.traffic_marker = Some(TrafficMarker {
                    bytes_received_at_mark: iface.bytes_received,
                    bytes_sent_at_mark: iface.bytes_sent,
                });
            }
        }
    }

    fn reset_activity_expand_state(&mut self) {
        self.ollama_state.activity_expand_started_at = Some(Instant::now());
        self.ollama_state.activity_expand_row = Some(self.ollama_state.activity_selected);
        self.ollama_state.activity_expand_suppressed = false;
    }

    fn close_activity_additions(&mut self) {
        self.ollama_state.activity_additions_open = false;
        self.ollama_state.activity_additions_selected = 0;
        if self.ollama_state.focused_panel == OllamaPanelFocus::Additions {
            self.ollama_state.focused_panel = OllamaPanelFocus::Activity;
        }
    }

    fn maybe_start_activity_expand_timer(&mut self) {
        if self.ollama_state.activity_expand_suppressed {
            return;
        }
        if self.ollama_state.activity_view != OllamaActivityView::List {
            return;
        }
        if self.ollama_state.focused_panel != OllamaPanelFocus::Activity {
            return;
        }
        self.ollama_state.activity_expand_started_at = Some(Instant::now());
        self.ollama_state.activity_expand_row = Some(self.ollama_state.activity_selected);
    }

    fn activity_expand_ready(&self) -> bool {
        if self.ollama_state.activity_expand_suppressed {
            return false;
        }
        if self.ollama_state.activity_view != OllamaActivityView::List {
            return false;
        }
        if self.ollama_state.focused_panel != OllamaPanelFocus::Activity {
            return false;
        }
        if self.ollama_state.activity_expand_row != Some(self.ollama_state.activity_selected) {
            return false;
        }
        let Some(started_at) = self.ollama_state.activity_expand_started_at else {
            return false;
        };
        started_at.elapsed() >= Duration::from_secs(2)
    }

    fn sorted_ollama_models(&self) -> Vec<OllamaModel> {
        let mut models = self
            .ollama_data
            .read()
            .as_ref()
            .map(|data| data.models.clone())
            .unwrap_or_default();
        sort_ollama_models(
            &mut models,
            self.ollama_state.model_sort_column,
            self.ollama_state.model_sort_ascending,
        );
        models
    }

    pub(crate) fn sorted_ollama_running_models(&self) -> Vec<RunningModel> {
        let mut models = self
            .ollama_data
            .read()
            .as_ref()
            .map(|data| data.running_models.clone())
            .unwrap_or_default();
        let mut known = HashSet::new();
        for model in &models {
            known.insert(model.name.to_ascii_lowercase());
        }
        for session in &self.ollama_state.paused_chats {
            let key = session.model.to_ascii_lowercase();
            if !known.contains(&key) {
                models.push(Self::build_running_placeholder(&session.model, "Paused"));
                known.insert(key);
            }
        }
        if let Some(active) = self.ollama_state.active_chat_model.as_deref() {
            let key = active.to_ascii_lowercase();
            if !known.contains(&key) {
                models.push(Self::build_running_placeholder(active, "Running"));
            }
        }
        sort_ollama_running(
            &mut models,
            self.ollama_state.running_sort_column,
            self.ollama_state.running_sort_ascending,
            &self.ollama_state.paused_chats,
            self.ollama_state.active_chat_model.as_deref(),
            &self.ollama_state.chat_messages,
        );
        models
    }

    fn selected_running_model_name(&self) -> Option<String> {
        let models = self.sorted_ollama_running_models();
        if models.is_empty() {
            return None;
        }
        let idx = self
            .ollama_state
            .selected_running_index
            .min(models.len().saturating_sub(1));
        models.get(idx).map(|model| model.name.clone())
    }

    fn build_running_placeholder(model_name: &str, processor: &str) -> RunningModel {
        let (params_value, params_unit, params_display) = Self::parse_params_from_name(model_name);
        let is_cloud = model_name.to_ascii_lowercase().contains("cloud");
        RunningModel {
            name: model_name.to_string(),
            size_bytes: 0,
            size_display: "-".to_string(),
            gpu_memory_mb: None,
            gpu_memory_display: if is_cloud {
                "cloud".to_string()
            } else {
                "-".to_string()
            },
            params_value,
            params_unit,
            params_display,
            processor: processor.to_string(),
            until: None,
        }
    }

    fn parse_params_from_name(name: &str) -> (Option<f64>, Option<char>, String) {
        let chars: Vec<char> = name.chars().collect();
        for (idx, ch) in chars.iter().enumerate() {
            let unit = ch.to_ascii_uppercase();
            if !matches!(unit, 'M' | 'B' | 'T') {
                continue;
            }
            if idx == 0 {
                continue;
            }
            let mut start = idx;
            while start > 0 {
                let prev = chars[start - 1];
                if prev.is_ascii_digit() || prev == '.' {
                    start -= 1;
                } else {
                    break;
                }
            }
            if start == idx {
                continue;
            }
            let num_str: String = chars[start..idx].iter().collect();
            if let Ok(value) = num_str.parse::<f64>() {
                let display = Self::format_param_display(value, unit);
                return (Some(value), Some(unit), display);
            }
        }
        (None, None, "-".to_string())
    }

    fn format_param_display(value: f64, unit: char) -> String {
        if (value.fract() - 0.0).abs() < f64::EPSILON {
            format!("{:.0}{}", value, unit)
        } else {
            let mut text = format!("{:.2}", value);
            while text.ends_with('0') {
                text.pop();
            }
            if text.ends_with('.') {
                text.pop();
            }
            format!("{text}{unit}")
        }
    }

    fn toggle_model_sort(&mut self, column: OllamaModelSortColumn) {
        if self.ollama_state.model_sort_column == column {
            self.ollama_state.model_sort_ascending = !self.ollama_state.model_sort_ascending;
        } else {
            self.ollama_state.model_sort_column = column;
            self.ollama_state.model_sort_ascending = true;
        }
    }

    fn toggle_running_sort(&mut self, column: OllamaRunningSortColumn) {
        if self.ollama_state.running_sort_column == column {
            self.ollama_state.running_sort_ascending = !self.ollama_state.running_sort_ascending;
        } else {
            self.ollama_state.running_sort_column = column;
            self.ollama_state.running_sort_ascending = true;
        }
    }

    fn toggle_cpu_sort(&mut self, column: CpuProcessSortColumn) {
        if self.cpu_state.sort_column == column {
            self.cpu_state.sort_ascending = !self.cpu_state.sort_ascending;
        } else {
            self.cpu_state.sort_column = column;
            self.cpu_state.sort_ascending = match column {
                CpuProcessSortColumn::Name => true,
                _ => false,
            };
        }
    }

    fn toggle_gpu_sort(&mut self, column: GpuProcessSortColumn) {
        if self.gpu_state.sort_column == column {
            self.gpu_state.sort_ascending = !self.gpu_state.sort_ascending;
        } else {
            self.gpu_state.sort_column = column;
            self.gpu_state.sort_ascending = true;
        }
    }

    fn allow_widget_scroll(&mut self) -> bool {
        Self::allow_with_throttle(
            &mut self.last_widget_scroll_input,
            Duration::from_millis(150),
        )
    }

    fn allow_text_input(&mut self) -> bool {
        Self::allow_with_throttle(&mut self.last_text_input, Duration::from_millis(8))
    }

    fn suggested_chat_prompt_height(&self, rows: u16) -> u16 {
        let fixed = if self.compact_mode { 3 } else { 3 + 8 + 5 };
        let min_main = 10;
        let available = rows.saturating_sub(fixed);
        let half = available / 2;
        let max_prompt = rows.saturating_sub(fixed.saturating_add(min_main)).max(3);
        half.max(3).min(max_prompt)
    }

    fn max_chat_prompt_height(&self) -> u16 {
        let (_, rows) = self.terminal_size;
        let reserved = if self.compact_mode {
            3 + 6
        } else {
            3 + 8 + 5 + 10
        };
        let max_height = rows.saturating_sub(reserved as u16);
        max_height.max(3)
    }

    fn max_chat_prompt_scroll(&self) -> usize {
        let (cols, _) = self.terminal_size;
        let width = cols.saturating_sub(2) as usize;
        let input_text = format!("chat {}_", self.ollama_state.input_buffer);
        let line_count = Self::wrapped_line_count(&input_text, width);
        line_count.saturating_sub(self.ollama_state.chat_prompt_height as usize)
    }

    fn wrapped_line_count(text: &str, width: usize) -> usize {
        if width == 0 {
            return 0;
        }
        if text.is_empty() {
            return 1;
        }
        let mut count = 1usize;
        let mut line_len = 0usize;
        for ch in text.chars() {
            if ch == '\n' {
                count += 1;
                line_len = 0;
                continue;
            }
            line_len += 1;
            if line_len > width {
                count += 1;
                line_len = 1;
            }
        }
        count
    }

    fn allow_with_throttle(last_input: &mut Option<Instant>, min_delay: Duration) -> bool {
        let now = Instant::now();
        if let Some(last) = last_input {
            if now.duration_since(*last) < min_delay {
                return false;
            }
        }
        *last_input = Some(now);
        true
    }

    fn next_ollama_focus(&self, current: OllamaPanelFocus) -> OllamaPanelFocus {
        let allow_input = self.ollama_state.input_mode != OllamaInputMode::None;
        if self.compact_mode {
            let next = match current {
                OllamaPanelFocus::Main => OllamaPanelFocus::Help,
                OllamaPanelFocus::Help => OllamaPanelFocus::Input,
                OllamaPanelFocus::Input => OllamaPanelFocus::Main,
                OllamaPanelFocus::Additions => OllamaPanelFocus::Help,
                _ => OllamaPanelFocus::Main,
            };
            if !allow_input && next == OllamaPanelFocus::Input {
                OllamaPanelFocus::Main
            } else {
                next
            }
        } else {
            let next = match current {
                OllamaPanelFocus::Main => OllamaPanelFocus::Vram,
                OllamaPanelFocus::Vram => OllamaPanelFocus::Activity,
                OllamaPanelFocus::Activity => {
                    if self.ollama_state.activity_additions_open {
                        OllamaPanelFocus::Additions
                    } else {
                        OllamaPanelFocus::Help
                    }
                }
                OllamaPanelFocus::Additions => OllamaPanelFocus::Help,
                OllamaPanelFocus::Help => OllamaPanelFocus::Input,
                OllamaPanelFocus::Input => OllamaPanelFocus::Main,
            };
            if !allow_input && next == OllamaPanelFocus::Input {
                OllamaPanelFocus::Main
            } else {
                next
            }
        }
    }

    fn prev_ollama_focus(&self, current: OllamaPanelFocus) -> OllamaPanelFocus {
        let allow_input = self.ollama_state.input_mode != OllamaInputMode::None;
        if self.compact_mode {
            let prev = match current {
                OllamaPanelFocus::Main => OllamaPanelFocus::Input,
                OllamaPanelFocus::Input => OllamaPanelFocus::Help,
                OllamaPanelFocus::Help => OllamaPanelFocus::Main,
                OllamaPanelFocus::Additions => OllamaPanelFocus::Help,
                _ => OllamaPanelFocus::Help,
            };
            if !allow_input && prev == OllamaPanelFocus::Input {
                OllamaPanelFocus::Help
            } else {
                prev
            }
        } else {
            let prev = match current {
                OllamaPanelFocus::Main => OllamaPanelFocus::Input,
                OllamaPanelFocus::Input => OllamaPanelFocus::Help,
                OllamaPanelFocus::Help => {
                    if self.ollama_state.activity_additions_open {
                        OllamaPanelFocus::Additions
                    } else {
                        OllamaPanelFocus::Activity
                    }
                }
                OllamaPanelFocus::Additions => OllamaPanelFocus::Activity,
                OllamaPanelFocus::Activity => OllamaPanelFocus::Vram,
                OllamaPanelFocus::Vram => OllamaPanelFocus::Main,
            };
            if !allow_input && prev == OllamaPanelFocus::Input {
                OllamaPanelFocus::Help
            } else {
                prev
            }
        }
    }

    fn start_ollama_chat(&mut self, model_name: String) {
        if self.ollama_state.chat_active && !self.ollama_state.chat_messages.is_empty() {
            self.finish_ollama_chat();
        } else {
            self.ollama_state.chat_messages.clear();
        }

        self.ollama_state.chat_active = true;
        self.ollama_state.active_chat_model = Some(model_name);
        self.ollama_state.chat_messages.clear();
        self.ollama_state.chat_scroll = 0;
        self.ollama_state.chat_prompt_scroll = 0;
        self.ollama_state.chat_prompt_height =
            self.suggested_chat_prompt_height(self.terminal_size.1);
        self.ollama_state.chat_pending = false;
        self.ollama_state.input_mode = OllamaInputMode::Chat;
        self.ollama_state.input_buffer.clear();
        self.ollama_state.focused_panel = OllamaPanelFocus::Input;
        self.ollama_state.activity_view = OllamaActivityView::List;
        self.ollama_state.activity_log_lines.clear();
        self.ollama_state.activity_log_title.clear();
        self.ollama_state.activity_log_scroll = 0;
        self.close_activity_additions();
    }

    fn pause_ollama_chat(&mut self) {
        if !self.ollama_state.chat_active {
            return;
        }

        let model_name = match self.ollama_state.active_chat_model.clone() {
            Some(name) => name,
            None => return,
        };

        let now = Local::now();
        let paused_at_display = now.format("%Y-%m-%d %H:%M").to_string();

        if !self.ollama_state.chat_messages.is_empty() {
            let log = self.build_chat_log();
            let (last_prompt, message_count, total_turns) = self.chat_message_stats();
            if let Ok(client) = OllamaClient::new(None) {
                if let Ok(entry) = client.save_chat_log_prefixed("p", &model_name, &log) {
                    let metadata = ChatLogMetadata {
                        model: model_name.clone(),
                        ended_at: entry.ended_at,
                        ended_at_display: entry.ended_at_display.clone(),
                        paused_at: Some(now.timestamp() as u64),
                        paused_at_display: Some(paused_at_display.clone()),
                        last_user_prompt: last_prompt,
                        message_count,
                        total_turns,
                    };
                    let _ = client.write_chat_metadata(&entry.path, &metadata);
                }
            }
        }

        let session = ChatSession {
            model: model_name.clone(),
            messages: self.ollama_state.chat_messages.clone(),
            chat_scroll: self.ollama_state.chat_scroll,
            prompt_buffer: self.ollama_state.input_buffer.clone(),
            prompt_scroll: self.ollama_state.chat_prompt_scroll,
            prompt_height: self.ollama_state.chat_prompt_height,
            paused_at: now.timestamp() as u64,
            paused_at_display,
        };

        if let Some(existing) = self
            .ollama_state
            .paused_chats
            .iter_mut()
            .find(|entry| entry.model == model_name)
        {
            *existing = session;
        } else {
            self.ollama_state.paused_chats.push(session);
        }

        self.ollama_state.chat_active = false;
        self.ollama_state.active_chat_model = None;
        self.ollama_state.chat_messages.clear();
        self.ollama_state.chat_scroll = 0;
        self.ollama_state.chat_pending = false;
        self.ollama_state.input_mode = OllamaInputMode::None;
        self.ollama_state.input_buffer.clear();
        self.ollama_state.chat_prompt_scroll = 0;
        self.ollama_state.chat_prompt_height = 3;
        self.ollama_state.focused_panel = OllamaPanelFocus::Main;
        self.ollama_state.activity_view = OllamaActivityView::List;
        self.ollama_state.activity_log_lines.clear();
        self.ollama_state.activity_log_title.clear();
        self.ollama_state.activity_log_scroll = 0;
        self.close_activity_additions();
    }

    fn resume_ollama_chat(&mut self, model_name: &str) -> bool {
        let idx = match self
            .ollama_state
            .paused_chats
            .iter()
            .position(|entry| entry.model == model_name)
        {
            Some(index) => index,
            None => return false,
        };
        let session = self.ollama_state.paused_chats.remove(idx);

        self.ollama_state.chat_active = true;
        self.ollama_state.active_chat_model = Some(session.model);
        self.ollama_state.chat_messages = session.messages;
        self.ollama_state.chat_scroll = session.chat_scroll;
        self.ollama_state.chat_pending = false;
        self.ollama_state.input_mode = OllamaInputMode::Chat;
        self.ollama_state.input_buffer = session.prompt_buffer;
        self.ollama_state.chat_prompt_scroll = session.prompt_scroll;
        self.ollama_state.chat_prompt_height = session.prompt_height.max(3);
        self.ollama_state.focused_panel = OllamaPanelFocus::Input;
        self.ollama_state.activity_view = OllamaActivityView::List;
        self.ollama_state.activity_log_lines.clear();
        self.ollama_state.activity_log_title.clear();
        self.ollama_state.activity_log_scroll = 0;
        self.close_activity_additions();
        true
    }

    fn build_chat_prompt(&self, new_prompt: &str) -> String {
        let mut prompt = String::new();
        for message in &self.ollama_state.chat_messages {
            match message.role {
                ChatRole::User => {
                    Self::append_chat_lines(&mut prompt, "Ð—Ð°Ð¿Ñ€Ð¾Ñ: ", &message.text)
                }
                ChatRole::Assistant => {
                    Self::append_chat_lines(&mut prompt, "ÐžÑ‚Ð²ÐµÑ‚: ", &message.text)
                }
            }
        }
        Self::append_chat_lines(&mut prompt, "Ð—Ð°Ð¿Ñ€Ð¾Ñ: ", new_prompt);
        prompt.push_str("ÐžÑ‚Ð²ÐµÑ‚: ");
        prompt
    }

    fn build_chat_log(&self) -> String {
        let mut log = String::new();
        for message in &self.ollama_state.chat_messages {
            match message.role {
                ChatRole::User => {
                    Self::append_chat_lines(&mut log, "Ð—Ð°Ð¿Ñ€Ð¾Ñ: ", &message.text)
                }
                ChatRole::Assistant => {
                    Self::append_chat_lines(&mut log, "ÐžÑ‚Ð²ÐµÑ‚: ", &message.text)
                }
            }
        }
        log
    }

    fn chat_message_stats(&self) -> (String, usize, usize) {
        let last_prompt = self
            .ollama_state
            .chat_messages
            .iter()
            .rev()
            .find(|message| message.role == ChatRole::User)
            .map(|message| message.text.clone())
            .unwrap_or_default();
        let message_count = self
            .ollama_state
            .chat_messages
            .iter()
            .filter(|message| message.role == ChatRole::Assistant)
            .count();
        let total_turns = self.ollama_state.chat_messages.len();
        (last_prompt, message_count, total_turns)
    }

    fn append_chat_lines(output: &mut String, prefix: &str, text: &str) {
        let mut lines = text.lines();
        if let Some(first) = lines.next() {
            output.push_str(prefix);
            output.push_str(first);
            output.push('\n');
        } else {
            output.push_str(prefix);
            output.push('\n');
        }
        for line in lines {
            output.push_str("  ");
            output.push_str(line);
            output.push('\n');
        }
    }

    fn match_prefix<'a>(line: &str, prefixes: &'a [&str]) -> Option<&'a str> {
        for prefix in prefixes {
            if line.starts_with(prefix) {
                return Some(*prefix);
            }
        }
        None
    }

    fn parse_chat_log_messages(&self, path: &str) -> Vec<ChatMessage> {
        let content = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(_) => return Vec::new(),
        };
        const USER_PREFIXES: [&str; 3] =
            ["Ð—Ð°Ð¿Ñ€Ð¾Ñ:", "Ð â€”Ð Â°Ð Ñ—Ð¡Ð‚Ð Ñ•Ð¡Ðƒ:", "Request:"];
        const ASSIST_PREFIXES: [&str; 3] = ["ÐžÑ‚Ð²ÐµÑ‚:", "Ð Ñ›Ð¡â€šÐ Ð†Ð ÂµÐ¡â€š:", "Response:"];

        let mut messages = Vec::new();
        let mut current_role: Option<ChatRole> = None;
        let mut current_text = String::new();

        for raw_line in content.lines() {
            let line = raw_line.trim_end().trim_start_matches('\u{feff}');
            if let Some(prefix) = Self::match_prefix(line, &USER_PREFIXES) {
                if let Some(role) = current_role.take() {
                    let text = current_text.trim_end().to_string();
                    if !text.is_empty() {
                        messages.push(ChatMessage { role, text });
                    }
                }
                current_text = line[prefix.len()..].trim_start().to_string();
                current_role = Some(ChatRole::User);
                continue;
            }
            if let Some(prefix) = Self::match_prefix(line, &ASSIST_PREFIXES) {
                if let Some(role) = current_role.take() {
                    let text = current_text.trim_end().to_string();
                    if !text.is_empty() {
                        messages.push(ChatMessage { role, text });
                    }
                }
                current_text = line[prefix.len()..].trim_start().to_string();
                current_role = Some(ChatRole::Assistant);
                continue;
            }
            if current_role.is_some() {
                let continuation = line.strip_prefix("  ").unwrap_or(line);
                if !current_text.is_empty() {
                    current_text.push('\n');
                }
                current_text.push_str(continuation);
            }
        }

        if let Some(role) = current_role {
            let text = current_text.trim_end().to_string();
            if !text.is_empty() {
                messages.push(ChatMessage { role, text });
            }
        }

        messages
    }

    fn restart_chat_from_log(&mut self, model_name: String, path: String) {
        let messages = self.parse_chat_log_messages(&path);
        self.start_ollama_chat(model_name);
        self.ollama_state.chat_messages = messages;
        self.ollama_state.chat_scroll = usize::MAX;
    }

    fn spawn_ollama_chat_prompt(&mut self, prompt: String) {
        if self.ollama_state.chat_pending {
            return;
        }

        let model_name = match self.ollama_state.active_chat_model.clone() {
            Some(name) => name,
            None => return,
        };

        let full_prompt = self.build_chat_prompt(&prompt);
        self.ollama_state.chat_messages.push(ChatMessage {
            role: ChatRole::User,
            text: prompt,
        });
        self.ollama_state.chat_scroll = usize::MAX;
        self.ollama_state.chat_pending = true;

        let tx = self.async_tx.clone();
        tokio::spawn(async move {
            let response = match OllamaClient::new(None) {
                Ok(client) => match client.run_model(&model_name, &full_prompt).await {
                    Ok(text) => Ok(text),
                    Err(error) => Err(error.to_string()),
                },
                Err(error) => Err(error.to_string()),
            };

            match response {
                Ok(text) => {
                    let normalized = AppState::normalize_model_response(text.trim());
                    let _ = tx.send(AsyncUpdate::OllamaChatCompleted {
                        response: normalized,
                    });
                }
                Err(error) => {
                    let _ = tx.send(AsyncUpdate::OllamaChatFailed { error });
                }
            }
        });
    }

    fn normalize_model_response(text: &str) -> String {
        let mut normalized = text.replace("\\r\\n", "\n");
        normalized = normalized.replace("\\n", "\n");
        normalized = normalized.replace("\\t", "\t");
        normalized
    }

    fn finish_ollama_chat(&mut self) {
        if let Some(model_name) = self.ollama_state.active_chat_model.clone() {
            if !self.ollama_state.chat_messages.is_empty() {
                let log = self.build_chat_log();
                let (last_prompt, message_count, total_turns) = self.chat_message_stats();
                if let Ok(client) = OllamaClient::new(None) {
                    if let Ok(entry) = client.save_chat_log(&model_name, &log) {
                        let metadata = ChatLogMetadata {
                            model: model_name.clone(),
                            ended_at: entry.ended_at,
                            ended_at_display: entry.ended_at_display.clone(),
                            paused_at: None,
                            paused_at_display: None,
                            last_user_prompt: last_prompt,
                            message_count,
                            total_turns,
                        };
                        let _ = client.write_chat_metadata(&entry.path, &metadata);
                    }
                }
            }
        }

        self.ollama_state.chat_active = false;
        self.ollama_state.active_chat_model = None;
        self.ollama_state.chat_messages.clear();
        self.ollama_state.chat_scroll = 0;
        self.ollama_state.chat_prompt_scroll = 0;
        self.ollama_state.chat_prompt_height = 3;
        self.ollama_state.chat_pending = false;
        self.ollama_state.input_mode = OllamaInputMode::None;
        self.ollama_state.input_buffer.clear();
        self.ollama_state.focused_panel = OllamaPanelFocus::Main;
        self.ollama_state.activity_view = OllamaActivityView::List;
        self.ollama_state.activity_log_lines.clear();
        self.ollama_state.activity_log_title.clear();
        self.ollama_state.activity_log_scroll = 0;
        self.close_activity_additions();
    }

    fn spawn_ollama_explanation(&mut self, block_id: u64) {
        let stderr = if let Some(block) = self.console_state.get_block(block_id) {
            crate::app::sudo::collect_stderr_tail(&block.output_lines, 15)
        } else {
            return;
        };

        if stderr.is_empty() {
            return;
        }

        let cmd = if let Some(block) = self.console_state.get_block(block_id) {
            block.input.clone()
        } else {
            String::new()
        };

        let prompt = format!(
            "Explain this error concisely. Suggest a fix.\n\nCommand: {}\n\nStderr:\n{}",
            cmd, stderr
        );

        let tx = self.async_tx.clone();
        let active_model = self.ollama_state.active_chat_model.clone();

        tokio::spawn(async move {
            match OllamaClient::new(None) {
                Ok(client) => {
                    let model_name = match active_model {
                        Some(name) => name,
                        None => {
                            // Fallback to first available model
                            if let Ok(models) = client.list_models().await {
                                let models: Vec<OllamaModel> = models;
                                if let Some(first) = models.first() {
                                    first.name.clone()
                                } else {
                                    let _ = tx.send(AsyncUpdate::ErrorExplanationFailed {
                                        block_id,
                                        error: "No Ollama models found. Please pull a model first."
                                            .to_string(),
                                    });
                                    return;
                                }
                            } else {
                                let _ = tx.send(AsyncUpdate::ErrorExplanationFailed {
                                    block_id,
                                    error: "Failed to list Ollama models.".to_string(),
                                });
                                return;
                            }
                        }
                    };

                    match client.run_model(&model_name, &prompt).await {
                        Ok(text) => {
                            let _ = tx.send(AsyncUpdate::ErrorExplanation {
                                block_id,
                                text: AppState::normalize_model_response(text.trim()),
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(AsyncUpdate::ErrorExplanationFailed {
                                block_id,
                                error: e.to_string(),
                            });
                        }
                    }
                }
                Err(_) => {
                    let _ = tx.send(AsyncUpdate::ErrorExplanationFailed {
                        block_id,
                        error: "Ollama is not running. Start with: ollama serve".to_string(),
                    });
                }
            }
        });
    }

    fn spawn_ollama_command(&mut self, command: String) {
        if self.ollama_state.command_pending {
            return;
        }

        self.ollama_state.command_pending = true;
        let tx = self.async_tx.clone();
        tokio::spawn(async move {
            let title = format!("Command: {}", command);
            let output = match OllamaClient::new(None) {
                Ok(client) => match client.execute_command(&command).await {
                    Ok(output) => output,
                    Err(error) => format!("Command failed: {error}"),
                },
                Err(error) => format!("Command failed: {error}"),
            };

            let mut lines: Vec<String> = output.lines().map(|line| line.to_string()).collect();
            if lines.is_empty() {
                lines.push("No output".to_string());
            }

            let _ = tx.send(AsyncUpdate::OllamaCommandCompleted { title, lines });
        });
    }

    // ── Shell builtin interception ──────────────────────────────────────

    fn finish_console_block(
        &mut self,
        input: String,
        lines: Vec<crate::app::console_state::OutputLine>,
        exit_code: i32,
    ) -> u64 {
        let outputs = lines
            .into_iter()
            .map(crate::app::console_state::CommandOutput::Line)
            .collect();
        self.finish_console_block_outputs(input, outputs, exit_code)
    }

    fn finish_console_block_outputs(
        &mut self,
        input: String,
        outputs: Vec<crate::app::console_state::CommandOutput>,
        exit_code: i32,
    ) -> u64 {
        let max_lines = self.config.read().console.max_output_lines;
        let block_id = self.console_state.start_command(input);
        if let Some(block) = self.console_state.get_block_mut(block_id) {
            for output in outputs {
                block.push_output(output, max_lines);
            }
            block.complete(exit_code);
            if exit_code != 0
                && block
                    .output_lines
                    .iter()
                    .any(|line| line.stream == crate::app::console_state::OutputStream::Stderr)
            {
                block.explain_hint = true;
            }
        }
        self.console_state.active_block_id = None;

        if let Some(block) = self.console_state.get_block(block_id) {
            let _ = self.history.record(
                &block.input,
                &block.cwd,
                Some(exit_code),
                Some(block.elapsed_ms() as i64),
                &self.console_state.hostname,
            );
        }

        block_id
    }

    fn start_console_session_block(
        &mut self,
        input: String,
        session: Box<dyn crate::app::extensions::ConsoleSession>,
    ) -> u64 {
        self.last_console_session_tick = Instant::now();
        self.console_state.start_session(input, session)
    }

    fn handle_console_session_key(&mut self, key: KeyEvent) {
        let Some(block_id) = self.console_state.active_session_block_id() else {
            return;
        };

        let status =
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                crate::app::extensions::SessionStatus::Quit
            } else {
                let Some(block) = self.console_state.get_block_mut(block_id) else {
                    return;
                };
                let Some(session) = block.session.as_mut() else {
                    return;
                };
                session.handle_key(key)
            };

        self.apply_console_session_status(block_id, status);
    }

    pub fn tick_console_sessions(&mut self) {
        let Some(block_id) = self.console_state.active_session_block_id() else {
            self.last_console_session_tick = Instant::now();
            return;
        };

        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_console_session_tick);
        if dt < Duration::from_millis(16) {
            return;
        }
        self.last_console_session_tick = now;

        let status = {
            let Some(block) = self.console_state.get_block_mut(block_id) else {
                return;
            };
            let Some(session) = block.session.as_mut() else {
                return;
            };
            session.tick(dt)
        };

        self.apply_console_session_status(block_id, status);
    }

    fn apply_console_session_status(
        &mut self,
        block_id: u64,
        status: crate::app::extensions::SessionStatus,
    ) {
        match status {
            crate::app::extensions::SessionStatus::Running
            | crate::app::extensions::SessionStatus::Paused => return,
            crate::app::extensions::SessionStatus::Finished
            | crate::app::extensions::SessionStatus::Quit => {}
        }

        let max_lines = self.config.read().console.max_output_lines;
        let exit_code = if status == crate::app::extensions::SessionStatus::Quit {
            130
        } else {
            0
        };

        if let Some(block) = self.console_state.get_block_mut(block_id) {
            let summary = block.session.as_ref().map(|session| session.summary());
            if let Some(summary) = summary {
                block.push_line(
                    crate::app::console_state::OutputLine::system(format!(
                        "{} [{}]",
                        summary.title,
                        session_status_label(status)
                    )),
                    max_lines,
                );
                for line in summary.lines {
                    block.push_line(
                        crate::app::console_state::OutputLine::stdout(line),
                        max_lines,
                    );
                }
            }
            block.session = None;
            if status == crate::app::extensions::SessionStatus::Quit {
                block.interrupt();
            } else {
                block.complete(0);
            }
        }

        if self.console_state.active_block_id == Some(block_id) {
            self.console_state.active_block_id = None;
        }

        if let Some(block) = self.console_state.get_block(block_id) {
            let _ = self.history.record(
                &block.input,
                &block.cwd,
                Some(exit_code),
                Some(block.elapsed_ms() as i64),
                &self.console_state.hostname,
            );
        }
    }

    /// Try to intercept a shell builtin command. Returns `Some(true)` if handled,
    /// `Some(false)` should never happen (reserved), `None` if not a builtin.
    fn try_intercept_builtin(&mut self, cmd: &str) -> Option<bool> {
        let trimmed = cmd.trim();
        let words = match crate::app::console_state::split_shell_words(trimmed) {
            Ok(words) => words,
            Err(error) => {
                self.finish_console_block(
                    trimmed.to_string(),
                    vec![crate::app::console_state::OutputLine::stderr(format!(
                        "parse error: {}",
                        error
                    ))],
                    2,
                );
                return Some(true);
            }
        };
        let Some(first_word) = words.first().map(|word| word.as_str()) else {
            return None;
        };

        // ── cd ─────────────────────────────────────────────────────────
        if first_word == "cd" {
            if words.len() > 2 {
                self.finish_console_block(
                    trimmed.to_string(),
                    vec![crate::app::console_state::OutputLine::stderr(
                        "cd: too many arguments",
                    )],
                    1,
                );
                return Some(true);
            }

            let arg = words.get(1).map(String::as_str).unwrap_or("");
            let target = if arg.is_empty() {
                // cd with no args → home directory
                dirs::home_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| self.console_state.cwd.clone())
            } else if arg == "-" {
                // cd - → previous directory
                match &self.console_state.prev_cwd {
                    Some(prev) => prev.clone(),
                    None => {
                        self.finish_console_block(
                            trimmed.to_string(),
                            vec![crate::app::console_state::OutputLine::stderr(
                                "cd: OLDPWD not set",
                            )],
                            1,
                        );
                        return Some(true);
                    }
                }
            } else if arg.starts_with('~') {
                // cd ~/path → expand home
                let home = dirs::home_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "~".to_string());
                let rest = arg.strip_prefix('~').unwrap_or("");
                format!("{}{}", home, rest)
            } else if std::path::Path::new(arg).is_absolute() {
                // Absolute path
                arg.to_string()
            } else {
                // Relative path — resolve from current CWD
                let base = std::path::Path::new(&self.console_state.cwd);
                base.join(arg).display().to_string()
            };

            // Try to canonicalize
            let resolved = std::path::Path::new(&target);
            match std::fs::canonicalize(resolved) {
                Ok(canonical) => {
                    if !canonical.is_dir() {
                        self.finish_console_block(
                            trimmed.to_string(),
                            vec![crate::app::console_state::OutputLine::stderr(format!(
                                "cd: {}: not a directory",
                                target
                            ))],
                            1,
                        );
                        return Some(true);
                    }

                    let new_cwd = canonical.display().to_string();
                    let old_cwd = self.console_state.cwd.clone();
                    self.console_state.prev_cwd = Some(old_cwd);
                    self.console_state.cwd = new_cwd.clone();

                    self.finish_console_block(
                        trimmed.to_string(),
                        vec![crate::app::console_state::OutputLine::stdout(new_cwd)],
                        0,
                    );
                }
                Err(e) => {
                    self.finish_console_block(
                        trimmed.to_string(),
                        vec![crate::app::console_state::OutputLine::stderr(format!(
                            "cd: {}: {}",
                            target, e
                        ))],
                        1,
                    );
                }
            }
            return Some(true);
        }

        // ── export ─────────────────────────────────────────────────────
        if first_word == "export" {
            let mut output = Vec::new();
            let mut exit_code = 0;

            if words.len() == 1 {
                let mut env_pairs: Vec<_> = self.console_state.env_vars.iter().collect();
                env_pairs.sort_by(|a, b| a.0.cmp(b.0));
                if env_pairs.is_empty() {
                    output.push(crate::app::console_state::OutputLine::stdout(
                        "No session environment overrides set.",
                    ));
                } else {
                    for (key, value) in env_pairs {
                        output.push(crate::app::console_state::OutputLine::stdout(format!(
                            "{}={}",
                            key, value
                        )));
                    }
                }
            } else {
                for assignment in words.iter().skip(1) {
                    if let Some((key, value)) = assignment.split_once('=') {
                        let key = key.trim();
                        if !is_valid_shell_identifier(key) {
                            output.push(crate::app::console_state::OutputLine::stderr(format!(
                                "export: `{}`: not a valid identifier",
                                key
                            )));
                            exit_code = 1;
                            continue;
                        }
                        self.console_state
                            .env_vars
                            .insert(key.to_string(), value.to_string());
                        output.push(crate::app::console_state::OutputLine::stdout(format!(
                            "{}={}",
                            key, value
                        )));
                    } else if is_valid_shell_identifier(assignment) {
                        let value = self
                            .console_state
                            .env_vars
                            .get(assignment)
                            .cloned()
                            .or_else(|| std::env::var(assignment).ok())
                            .unwrap_or_default();
                        self.console_state
                            .env_vars
                            .insert(assignment.clone(), value.clone());
                        output.push(crate::app::console_state::OutputLine::stdout(format!(
                            "{}={}",
                            assignment, value
                        )));
                    } else {
                        output.push(crate::app::console_state::OutputLine::stderr(format!(
                            "export: `{}`: not a valid identifier",
                            assignment
                        )));
                        exit_code = 1;
                    }
                }
            }

            self.finish_console_block(trimmed.to_string(), output, exit_code);
            return Some(true);
        }

        // ── unset ──────────────────────────────────────────────────────
        if first_word == "unset" {
            let mut output = Vec::new();
            let mut exit_code = 0;
            if words.len() == 1 {
                output.push(crate::app::console_state::OutputLine::stderr(
                    "unset: usage: unset VAR [VAR ...]",
                ));
                exit_code = 1;
            } else {
                for var_name in words.iter().skip(1) {
                    if is_valid_shell_identifier(var_name) {
                        self.console_state.env_vars.remove(var_name);
                    } else {
                        output.push(crate::app::console_state::OutputLine::stderr(format!(
                            "unset: `{}`: not a valid identifier",
                            var_name
                        )));
                        exit_code = 1;
                    }
                }
            }
            self.finish_console_block(trimmed.to_string(), output, exit_code);
            return Some(true);
        }

        // ── clear ──────────────────────────────────────────────────────
        if first_word == "clear" && words.len() == 1 {
            self.console_state.blocks.clear();
            self.console_state.scroll_offset = 0;
            self.console_state.selected_block = None;
            return Some(true);
        }

        // ── exit ───────────────────────────────────────────────────────
        if first_word == "exit" {
            self.finish_console_block(
                trimmed.to_string(),
                vec![crate::app::console_state::OutputLine::stderr(
                    "exit: not supported in embedded console (use Ctrl+Q to quit the application)",
                )],
                1,
            );
            return Some(true);
        }

        // ── source / . / alias ─────────────────────────────────────────
        if first_word == "source" || first_word == "." || first_word == "alias" {
            self.finish_console_block(
                trimmed.to_string(),
                vec![
                    crate::app::console_state::OutputLine::stderr(
                        "Shell builtins like source/alias only persist per command in the embedded console.",
                    ),
                    crate::app::console_state::OutputLine::stderr(
                        "Use 'export VAR=val' to set session environment variables.",
                    ),
                ],
                1,
            );
            return Some(true);
        }

        None // Not a builtin — proceed with normal execution
    }

    // ── Interactive command detection ──────────────────────────────────

    /// Check if a command would start an interactive/TUI program that cannot
    /// work without a proper PTY (pseudo-terminal).
    fn is_interactive_command(cmd: &str) -> bool {
        let trimmed = cmd.trim();
        let words = match crate::app::console_state::split_shell_words(trimmed) {
            Ok(words) => words,
            Err(_) => trimmed
                .split_whitespace()
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>(),
        };
        command_invocation_is_interactive(&words)
    }

    fn spawn_console_command(&mut self, command: String) {
        if self.console_state.is_running() {
            return;
        }

        let block_id = self.console_state.start_command(command.clone());

        let executor = self.console_executor.clone();

        // Capture console state for the spawned task
        let cwd = self.console_state.cwd.clone();
        let env = self.console_state.env_vars.clone();
        let terminal_size = Some(self.terminal_size);

        // Setup channels for streaming output
        let tx = self.async_tx.clone();

        tokio::spawn(async move {
            let env_ref = if env.is_empty() { None } else { Some(&env) };
            match executor
                .execute_stream(&command, Some(&cwd), env_ref, terminal_size)
                .await
            {
                Ok(mut rx) => {
                    while let Some(msg) = rx.recv().await {
                        match msg {
                            StreamMessage::Stdout(line) => {
                                let _ = tx.send(AsyncUpdate::ConsoleStdout { block_id, line });
                            }
                            StreamMessage::Stderr(line) => {
                                let _ = tx.send(AsyncUpdate::ConsoleStderr { block_id, line });
                            }
                            StreamMessage::Interrupted => {
                                let _ = tx.send(AsyncUpdate::ConsoleInterrupted { block_id });
                            }
                            StreamMessage::Exit(code) => {
                                let _ = tx.send(AsyncUpdate::ConsoleCompleted {
                                    block_id,
                                    exit_code: code,
                                });
                            }
                        }
                    }
                }
                Err(error) => {
                    let _ = tx.send(AsyncUpdate::ConsoleFailed {
                        block_id,
                        error: format!("Error spawning command: {}", error),
                    });
                }
            }
        });
    }

    fn console_extension_context(&self) -> crate::app::extensions::ConsoleContext {
        let config = self.config.read();
        let mut env_vars = self
            .console_state
            .env_vars
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Vec<_>>();
        env_vars.sort_by(|a, b| a.0.cmp(&b.0));

        crate::app::extensions::ConsoleContext {
            cwd: self.console_state.cwd.clone(),
            shell_name: self.console_state.shell_name.clone(),
            terminal_size: self.terminal_size,
            env_vars,
            config: crate::app::extensions::ConsoleConfigContext {
                history_limit: config.console.history_limit,
                max_output_lines: config.console.max_output_lines,
            },
            theme: crate::app::extensions::ConsoleThemeContext {
                name: config.general.theme.clone(),
                compact_mode: config.general.compact_mode,
            },
            permissions: crate::app::extensions::PermissionPolicy::default_deny(),
        }
    }

    fn try_intercept_console_extension(&mut self, cmd: &str) -> bool {
        let context = self.console_extension_context();
        match self.console_extensions.route(cmd, &context) {
            crate::app::extensions::ConsoleRoute::Shell => false,
            crate::app::extensions::ConsoleRoute::Handled(response) => {
                let crate::app::extensions::ConsoleCommandResponse { result, exit_code } = response;
                match result {
                    crate::app::extensions::ConsoleResult::StartSession(session) => {
                        self.start_console_session_block(cmd.trim().to_string(), session);
                    }
                    result => {
                        let response =
                            crate::app::extensions::ConsoleCommandResponse { result, exit_code };
                        let (outputs, exit_code) = response.into_outputs();
                        self.finish_console_block_outputs(
                            cmd.trim().to_string(),
                            outputs,
                            exit_code,
                        );
                    }
                }
                true
            }
        }
    }

    /// Refresh the ghost text based on current input buffer.
    /// Checks suggestion engine first (for command-name completions),
    /// then falls back to history prefix search.
    fn refresh_ghost_text(&mut self) {
        self.refresh_syntax_highlight();
        let input = &self.console_state.input_buffer;
        if input.is_empty() {
            self.console_state.clear_ghost_text();
            return;
        }

        let cwd = self.console_state.cwd.clone();

        if let Some(suggestion) = self.console_extensions.suggest_prefixed_command(input) {
            self.console_state.update_ghost_text(Some(suggestion));
            return;
        }

        // 1. Try suggestion engine (builtins, PATH, aliases, Portage) for command/package completion
        let suggestions = self.suggestion_engine.suggest(input, &cwd);
        if let Some(best) = suggestions.first() {
            if best.text.len() > input.len() && best.text.starts_with(input) {
                self.console_state
                    .update_ghost_text(Some(best.text.clone()));
                return;
            }
        }

        // 2. Fall back to history prefix search (for full command completions)
        match self.history.search_prefix(input, Some(&cwd), 1) {
            Ok(results) => {
                if let Some(entry) = results.first() {
                    if entry.command.len() > input.len() && entry.command.starts_with(input) {
                        self.console_state
                            .update_ghost_text(Some(entry.command.clone()));
                    } else {
                        self.console_state.clear_ghost_text();
                    }
                } else {
                    self.console_state.clear_ghost_text();
                }
            }
            Err(_) => {
                self.console_state.clear_ghost_text();
            }
        }

        self.refresh_syntax_highlight();
    }

    /// Refresh syntax highlighting for the current input buffer.
    fn refresh_syntax_highlight(&mut self) {
        let input = self.console_state.input_buffer.clone();
        if input.is_empty() {
            self.console_state.highlighted_input.clear();
            return;
        }

        let engine = &self.suggestion_engine;
        let extensions = &self.console_extensions;
        self.console_state.highlighted_input = crate::app::syntax::highlight(&input, |cmd| {
            engine.is_known_command(cmd) || extensions.is_prefixed_command(cmd)
        });
    }

    /// Expand command macros (!! / !$ / sudo !!) and return the expanded command.
    fn expand_macros(&self, cmd: &str) -> std::result::Result<String, String> {
        let mut expanded = cmd.to_string();

        // sudo !! → sudo <last_command>
        if expanded.trim() == "sudo !!" {
            let last = self
                .console_state
                .last_command
                .as_deref()
                .ok_or_else(|| "!!: event not found".to_string())?;
            return Ok(crate::app::sudo::sudo_command(last));
        }

        // !! → <last_command>
        if expanded.contains("!!") {
            let last = self
                .console_state
                .last_command
                .as_deref()
                .ok_or_else(|| "!!: event not found".to_string())?;
            expanded = expanded.replace("!!", last);
        }

        // !$ → <last_args>
        if expanded.contains("!$") {
            let last_args = self
                .console_state
                .last_args
                .as_deref()
                .ok_or_else(|| "!$: event not found".to_string())?;
            expanded = expanded.replace("!$", last_args);
        }

        Ok(expanded)
    }

    /// Update history search results based on the current query.
    fn update_history_search_results(&mut self) {
        let query = self.console_state.history_search_query.clone();
        let cwd = self.console_state.cwd.clone();

        match self.history.search_fuzzy(&query, Some(&cwd), 50) {
            Ok(results) => {
                self.console_state.history_search_results =
                    results.into_iter().map(|e| e.command).collect();
                // Reset index if out of bounds
                if self.console_state.history_search_index
                    >= self.console_state.history_search_results.len()
                {
                    self.console_state.history_search_index = 0;
                }
            }
            Err(_) => {
                self.console_state.history_search_results.clear();
                self.console_state.history_search_index = 0;
            }
        }
    }

    pub async fn new(config: Arc<RwLock<Config>>) -> Result<Self> {
        let config_snapshot = config.read().clone();
        let tab_manager = TabManager::new(
            config_snapshot.tabs.enabled.clone(),
            &config_snapshot.tabs.default,
        );

        let compact_mode = config_snapshot.general.compact_mode;

        let (async_tx, async_rx) = unbounded_channel();
        let console_executor = crate::platform::get_executor();

        let cpu_data = Arc::new(RwLock::new(None));
        let cpu_error = Arc::new(RwLock::new(None));
        let gpu_data = Arc::new(RwLock::new(None));
        let gpu_error = Arc::new(RwLock::new(None));
        let ram_data = Arc::new(RwLock::new(None));
        let ram_error = Arc::new(RwLock::new(None));
        let disk_data = Arc::new(RwLock::new(None));
        let disk_error = Arc::new(RwLock::new(None));
        let disk_analyzer_data = Arc::new(RwLock::new(None));
        let disk_analyzer_error = Arc::new(RwLock::new(None));
        let network_data = Arc::new(RwLock::new(None));
        let network_error = Arc::new(RwLock::new(None));
        let process_data = Arc::new(RwLock::new(None));
        let process_error = Arc::new(RwLock::new(None));
        let service_data = Arc::new(RwLock::new(None));
        let service_error = Arc::new(RwLock::new(None));

        let ollama_data = Arc::new(RwLock::new(None));
        let ollama_error = Arc::new(RwLock::new(None));
        let mut console_state =
            crate::app::console_state::ConsoleState::new(config_snapshot.console.history_limit);
        console_state.shell_name = console_executor.name().to_string();
        console_state.status_threshold_ms = config_snapshot.console.status_threshold_ms;
        console_state.status_persist_ms = config_snapshot.console.status_persist_ms;
        console_state.enable_ai_explain = config_snapshot.console.enable_ai_explain;

        // Start monitor tasks
        monitors_task::spawn_monitor_tasks(
            Arc::clone(&config),
            Arc::clone(&cpu_data),
            Arc::clone(&cpu_error),
            Arc::clone(&gpu_data),
            Arc::clone(&gpu_error),
            Arc::clone(&ram_data),
            Arc::clone(&ram_error),
            Arc::clone(&disk_data),
            Arc::clone(&disk_error),
            Arc::clone(&disk_analyzer_data),
            Arc::clone(&disk_analyzer_error),
            Arc::clone(&network_data),
            Arc::clone(&network_error),
            Arc::clone(&process_data),
            Arc::clone(&process_error),
            Arc::clone(&service_data),
            Arc::clone(&service_error),
            Arc::clone(&ollama_data),
            Arc::clone(&ollama_error),
        );

        #[cfg(target_os = "linux")]
        let network_diag_engine = Arc::new(linux_netdiag::NetworkDiagnosticsEngine::new());
        #[cfg(target_os = "linux")]
        let network_diag_rx = network_diag_engine.subscribe();

        Ok(Self {
            config,
            tab_manager,
            compact_mode,

            cpu_data,
            cpu_error,
            gpu_data,
            gpu_error,
            ram_data,
            ram_error,
            disk_data,
            disk_error,
            disk_analyzer_data,
            disk_analyzer_error,
            network_data,
            network_error,
            process_data,
            process_error,
            service_data,
            service_error,

            ollama_data,
            ollama_error,

            console_state,
            suggestion_engine: crate::app::suggestions::SuggestionEngine::new(),
            console_extensions: crate::app::extensions::ConsoleCommandRouter::builtin(),
            console_executor,
            history: {
                let history_dir = dirs::data_local_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("tui-console");
                let _ = std::fs::create_dir_all(&history_dir);
                crate::app::history::CommandHistory::open(history_dir.join("history.db"))
                    .unwrap_or_else(|_| {
                        crate::app::history::CommandHistory::open_in_memory()
                            .expect("in-memory history")
                    })
            },
            selected_section: None,
            last_nav_input: None,
            last_horizontal_nav_input: None,
            last_sort_input: None,
            last_widget_scroll_input: None,
            last_view_toggle_input: None,
            last_text_input: None,
            terminal_size: terminal::size().unwrap_or((120, 40)),
            last_console_session_tick: Instant::now(),

            cpu_state: CpuUIState {
                selected_index: 0,
                scroll_offset: 0,
                sort_column: CpuProcessSortColumn::Cpu,
                sort_ascending: false,
            },

            gpu_state: GpuUIState {
                selected_index: 0,
                sort_column: GpuProcessSortColumn::Gpu,
                sort_ascending: false,
            },

            ram_state: RamUIState {
                focused_panel: RamPanelFocus::TopProcesses,
                selected_index: 0,
                sort_column: RamProcessSortColumn::WorkingSet,
                sort_ascending: false,
            },

            network_ui_state: NetworkUIState {
                focus: NetworkFocusZone::Tools,
                result_tab: NetworkResultTab::Summary,
                center_view: NetworkCenterView::Interface,
                input_mode: false,
                target_input: "1.1.1.1".to_string(),
                selected_tool: NetworkDiagnosticTool::Ping,
                tools_scroll_offset: 0,
                running_job: None,
                nat_mapping_confirm_until: None,
                last_job: None,
                last_summary: "No diagnostics run yet".to_string(),
                last_error: None,
                event_log: VecDeque::with_capacity(80),
                detail_lines: vec![
                    "Run diagnostics to see detailed output.".to_string(),
                    "Use Tab to navigate panels. Enter to run tools.".to_string(),
                ],
                detail_scroll: 0,
                raw_stdout: Vec::new(),
                raw_stderr: Vec::new(),
                advice_lines: Vec::new(),
                result_history: VecDeque::with_capacity(64),
                connections_scroll: 0,
                bandwidth_scroll: 0,
                activity_scroll: 0,
                traffic_marker: None,
                show_marker_traffic: false,
                selected_interface_idx: 0,
                filter_active: false,
                filter_input: String::new(),
            },

            processes_state: ProcessesUIState {
                selected_index: 0,
                scroll_offset: 0,
                sort_column: ProcessSortColumn::Cpu,
                sort_ascending: false,
                filter: String::new(),
            },

            services_state: ServicesUIState {
                selected_index: 0,
                scroll_offset: 0,
                sort_column: ServiceSortColumn::Name,
                sort_ascending: true,
                status_filter: ServiceStatusFilter::All,
                focused_panel: ServicesPanelFocus::Table,
                details_scroll: 0,
            },

            ollama_state: OllamaUIState {
                selected_model_index: 0,
                selected_running_index: 0,
                current_view: OllamaView::Models,
                focused_panel: OllamaPanelFocus::Main,
                input_mode: OllamaInputMode::None,
                input_buffer: String::new(),
                chat_active: false,
                active_chat_model: None,
                chat_messages: Vec::new(),
                chat_scroll: 0,
                activity_view: OllamaActivityView::List,
                activity_selected: 0,
                activity_log_scroll: 0,
                activity_log_lines: Vec::new(),
                activity_log_title: String::new(),
                activity_expand_started_at: None,
                activity_expand_row: None,
                activity_expand_suppressed: false,
                activity_additions_open: false,
                activity_additions_selected: 0,
                model_sort_column: OllamaModelSortColumn::Name,
                model_sort_ascending: true,
                running_sort_column: OllamaRunningSortColumn::Name,
                running_sort_ascending: true,
                running_summary_scroll: 0,
                chat_prompt_height: 3,
                chat_prompt_scroll: 0,
                paused_chats: Vec::new(),
                pending_delete: None,
                show_delete_confirm: false,
                chat_pending: false,
                command_pending: false,
            },
            #[cfg(target_os = "linux")]
            network_diag_engine,
            #[cfg(target_os = "linux")]
            network_diag_rx,
            async_tx,
            async_rx,
            last_config_version: 0,
        })
    }

    pub async fn handle_event(&mut self, event: CrosstermEvent) -> Result<bool> {
        self.apply_async_updates();
        match event {
            CrosstermEvent::Key(key_event) => self.handle_key_event(key_event).await,
            CrosstermEvent::Mouse(mouse_event) => self.handle_mouse_event(mouse_event).await,
            CrosstermEvent::Resize(cols, rows) => {
                self.update_terminal_size(cols, rows);
                Ok(true)
            }
            _ => Ok(true),
        }
    }

    async fn handle_key_event(&mut self, key: KeyEvent) -> Result<bool> {
        let is_initial_press = matches!(key.kind, KeyEventKind::Press);
        if self.tab_manager.current() == TabType::Console
            && self.console_state.active_session_block_id().is_some()
        {
            if is_initial_press {
                self.handle_console_session_key(key);
            }
            return Ok(true);
        }

        // Handle Ctrl+C
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            if is_initial_press && self.tab_manager.current() == TabType::Console {
                if self.console_state.is_running() {
                    // Interrupt the running command
                    if let Some(block_id) = self.console_state.active_block_id {
                        let msg = match self.console_executor.interrupt().await {
                            Ok(()) => "Interrupt signal sent to active command.".to_string(),
                            Err(e) => format!("Failed to interrupt active command: {}", e),
                        };
                        if let Some(block) = self.console_state.get_block_mut(block_id) {
                            let max_lines = self.config.read().console.max_output_lines;
                            block.push_line(
                                crate::app::console_state::OutputLine::system(msg),
                                max_lines,
                            );
                        }
                    }
                    return Ok(true);
                } else if !self.console_state.input_buffer.is_empty() {
                    // Clear the input buffer when no command is running
                    self.console_state.clear_input();
                    self.console_state.clear_ghost_text();
                    self.console_state.reset_history_nav();
                    self.console_state.highlighted_input.clear();
                    return Ok(true);
                }
            }
            return Ok(false);
        }

        if self.tab_manager.current() == TabType::Console {
            use crate::app::console_state::ConsoleMode;

            match self.console_state.mode {
                ConsoleMode::Normal => {
                    // Ctrl+S: sudo retry
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('s')
                    {
                        if is_initial_press {
                            // Find the latest block with sudo_hint
                            if let Some(block) =
                                self.console_state.blocks.iter().rev().find(|b| b.sudo_hint)
                            {
                                let cmd = crate::app::sudo::sudo_command(&block.input);
                                self.console_state.mode = ConsoleMode::Confirm;
                                self.console_state.confirm_command = Some(cmd);
                                self.console_state.confirm_action =
                                    Some("Re-run with sudo".to_string());
                            }
                        }
                        return Ok(true);
                    }

                    // Ctrl+E: Explain error with AI
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('e')
                    {
                        if is_initial_press {
                            if let Some(block_id) = self
                                .console_state
                                .blocks
                                .iter()
                                .rev()
                                .find(|b| b.explain_hint)
                                .map(|b| b.id)
                            {
                                if let Some(mut_block) = self.console_state.get_block_mut(block_id)
                                {
                                    mut_block.is_explaining = true;
                                }
                                self.spawn_ollama_explanation(block_id);
                            }
                        }
                        return Ok(true);
                    }

                    match key.code {
                        KeyCode::Char('i') => {
                            if is_initial_press {
                                self.console_state.enter_insert_mode();
                                self.refresh_ghost_text();
                            }
                            return Ok(true);
                        }
                        KeyCode::Up => {
                            if is_initial_press && self.allow_nav() {
                                self.console_state.scroll_up(1);
                            }
                            return Ok(true);
                        }
                        KeyCode::Down => {
                            if is_initial_press && self.allow_nav() {
                                self.console_state.scroll_down(1);
                            }
                            return Ok(true);
                        }
                        _ => {}
                    }
                }
                ConsoleMode::Insert => {
                    // Ctrl+R: enter history search
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('r')
                    {
                        if is_initial_press {
                            self.console_state.enter_history_search();
                            // Populate with recent history
                            if let Ok(recent) = self.history.get_recent(50) {
                                self.console_state.history_search_results =
                                    recent.into_iter().map(|e| e.command).collect();
                            }
                        }
                        return Ok(true);
                    }

                    // ── Readline shortcuts ──────────────────────────────
                    if is_initial_press && key.modifiers.contains(KeyModifiers::CONTROL) {
                        match key.code {
                            KeyCode::Char('a') => {
                                self.console_state.move_cursor_home();
                                return Ok(true);
                            }
                            KeyCode::Char('e') => {
                                self.console_state.move_cursor_end();
                                return Ok(true);
                            }
                            KeyCode::Char('k') => {
                                let _ = self.console_state.kill_to_end();
                                self.refresh_ghost_text();
                                return Ok(true);
                            }
                            KeyCode::Char('u') => {
                                let _ = self.console_state.kill_to_start();
                                self.refresh_ghost_text();
                                return Ok(true);
                            }
                            KeyCode::Char('w') => {
                                let _ = self.console_state.kill_word_back();
                                self.refresh_ghost_text();
                                return Ok(true);
                            }
                            KeyCode::Char('l') => {
                                // Ctrl+L: clear screen (like bash)
                                self.console_state.blocks.clear();
                                self.console_state.scroll_offset = 0;
                                self.console_state.selected_block = None;
                                return Ok(true);
                            }
                            _ => {}
                        }
                    }

                    match key.code {
                        KeyCode::Esc => {
                            if is_initial_press {
                                self.console_state.enter_normal_mode();
                                self.console_state.clear_ghost_text();
                                self.console_state.reset_history_nav();
                            }
                            return Ok(true);
                        }
                        KeyCode::Home => {
                            if is_initial_press {
                                self.console_state.move_cursor_home();
                            }
                            return Ok(true);
                        }
                        KeyCode::End => {
                            if is_initial_press {
                                self.console_state.move_cursor_end();
                            }
                            return Ok(true);
                        }
                        KeyCode::Char(c) => {
                            if is_initial_press && self.allow_text_input() {
                                self.console_state.reset_history_nav();
                                self.console_state.insert_char(c);
                                self.refresh_ghost_text();
                            }
                            return Ok(true);
                        }
                        KeyCode::Backspace => {
                            if is_initial_press
                                && self.allow_text_input()
                                && self.console_state.cursor_position > 0
                            {
                                self.console_state.reset_history_nav();
                                self.console_state.backspace();
                                self.refresh_ghost_text();
                            }
                            return Ok(true);
                        }
                        KeyCode::Delete => {
                            if is_initial_press
                                && self.allow_text_input()
                                && self.console_state.cursor_position
                                    < self.console_state.input_char_count()
                            {
                                self.console_state.delete_char();
                                self.refresh_ghost_text();
                            }
                            return Ok(true);
                        }
                        KeyCode::Left => {
                            if is_initial_press && self.console_state.cursor_position > 0 {
                                self.console_state.move_cursor_left();
                            }
                            return Ok(true);
                        }
                        KeyCode::Right => {
                            if is_initial_press {
                                if key.modifiers.contains(KeyModifiers::ALT) {
                                    // Alt+Right: accept one word from ghost text
                                    self.console_state.accept_ghost_word();
                                    self.refresh_ghost_text();
                                } else if self.console_state.cursor_position
                                    >= self.console_state.input_char_count()
                                    && self.console_state.ghost_text.is_some()
                                {
                                    // Right at end of buffer: accept full ghost text
                                    self.console_state.accept_ghost_text();
                                    self.console_state.clear_ghost_text();
                                } else {
                                    self.console_state.move_cursor_right();
                                }
                            }
                            return Ok(true);
                        }
                        KeyCode::Up => {
                            if is_initial_press {
                                // History navigation: Up cycles backward through history
                                if self.console_state.history_nav_cache.is_empty() {
                                    if let Ok(recent) = self.history.get_recent(200) {
                                        let cmds: Vec<String> =
                                            recent.into_iter().map(|e| e.command).collect();
                                        self.console_state.start_history_nav(cmds);
                                    }
                                }
                                self.console_state.history_nav_up();
                                self.console_state.clear_ghost_text();
                                self.refresh_syntax_highlight();
                            }
                            return Ok(true);
                        }
                        KeyCode::Down => {
                            if is_initial_press {
                                self.console_state.history_nav_down();
                                self.console_state.clear_ghost_text();
                                self.refresh_syntax_highlight();
                            }
                            return Ok(true);
                        }
                        KeyCode::Enter => {
                            if is_initial_press {
                                let raw_cmd = self.console_state.input_buffer.clone();
                                self.console_state.clear_input();
                                self.console_state.clear_ghost_text();
                                self.console_state.reset_history_nav();
                                self.console_state.highlighted_input.clear();
                                self.console_state.scroll_offset = 0;

                                if !raw_cmd.trim().is_empty() {
                                    if self.try_intercept_console_extension(&raw_cmd) {
                                        return Ok(true);
                                    }

                                    let cmd =
                                        match self.expand_macros(&raw_cmd) {
                                            Ok(cmd) => cmd,
                                            Err(error) => {
                                                self.finish_console_block(
                                                raw_cmd.trim().to_string(),
                                                vec![crate::app::console_state::OutputLine::stderr(
                                                    error,
                                                )],
                                                1,
                                            );
                                                return Ok(true);
                                            }
                                        };

                                    // Intercept 'explain' meta-command
                                    if cmd.trim() == "explain" {
                                        if let Some(block_id) = self
                                            .console_state
                                            .blocks
                                            .iter()
                                            .rev()
                                            .find(|b| b.explain_hint)
                                            .map(|b| b.id)
                                        {
                                            if let Some(mut_block) =
                                                self.console_state.get_block_mut(block_id)
                                            {
                                                mut_block.is_explaining = true;
                                            }
                                            self.spawn_ollama_explanation(block_id);
                                        } else {
                                            let block_id = self
                                                .console_state
                                                .start_command("explain".to_string());
                                            if let Some(block) =
                                                self.console_state.get_block_mut(block_id)
                                            {
                                                let max_lines =
                                                    self.config.read().console.max_output_lines;
                                                block.push_line(crate::app::console_state::OutputLine::stderr("No failed command with stderr output found to explain."), max_lines);
                                                block.complete(1);
                                            }
                                            self.console_state.active_block_id = None;
                                        }
                                        return Ok(true);
                                    }

                                    // ── Shell builtin interception ─────────────────────
                                    if let Some(handled) = self.try_intercept_builtin(&cmd) {
                                        if handled {
                                            return Ok(true);
                                        }
                                    }

                                    // ── Interactive command blocking ───────────────────
                                    if Self::is_interactive_command(&cmd) {
                                        let block_id =
                                            self.console_state.start_command(cmd.clone());
                                        if let Some(block) =
                                            self.console_state.get_block_mut(block_id)
                                        {
                                            let max_lines =
                                                self.config.read().console.max_output_lines;
                                            block.push_line(
                                                crate::app::console_state::OutputLine::stderr(
                                                    "Interactive commands are not supported in the embedded console."
                                                ),
                                                max_lines,
                                            );
                                            block.push_line(
                                                crate::app::console_state::OutputLine::stderr(
                                                    "  Use a full terminal emulator for interactive programs."
                                                ),
                                                max_lines,
                                            );
                                            block.complete(1);
                                        }
                                        self.console_state.active_block_id = None;
                                        return Ok(true);
                                    }

                                    // Handle pkg meta-commands
                                    let mut final_cmd = cmd.clone();
                                    if final_cmd.trim().starts_with("pkg ") {
                                        let parts: Vec<&str> =
                                            final_cmd.trim().split_whitespace().collect();
                                        if parts.len() >= 3 {
                                            let subcmd = parts[1];
                                            let pkg = parts[2..].join(" ");
                                            final_cmd = match subcmd {
                                                "info" => format!("equery meta {}", pkg),
                                                "find" => format!("emerge -s {}", pkg),
                                                "uses" => format!("equery uses {}", pkg),
                                                "deps" => format!("equery d {}", pkg),
                                                "web" => format!("equery meta -w {}", pkg),
                                                _ => final_cmd,
                                            };
                                        }
                                    }

                                    // Handle auto-detect meta-commands for modern tools
                                    let has_cmd = |name: &str| -> bool {
                                        std::env::var_os("PATH").map_or(false, |paths| {
                                            std::env::split_paths(&paths).any(|dir| {
                                                dir.join(name).is_file()
                                                    || dir.join(format!("{}.exe", name)).is_file()
                                            })
                                        })
                                    };

                                    let trimmed_cmd = final_cmd.trim();
                                    if trimmed_cmd == "tree" || trimmed_cmd.starts_with("tree ") {
                                        let args =
                                            trimmed_cmd.strip_prefix("tree").unwrap_or("").trim();
                                        if has_cmd("eza") {
                                            final_cmd =
                                                format!("eza --tree --color=always {}", args)
                                                    .trim()
                                                    .to_string();
                                        } else if has_cmd("tree") {
                                            final_cmd =
                                                format!("tree -C {}", args).trim().to_string();
                                        }
                                    } else if trimmed_cmd.starts_with("search ") {
                                        let args = trimmed_cmd
                                            .strip_prefix("search ")
                                            .unwrap_or("")
                                            .trim();
                                        if has_cmd("rg") {
                                            // ripgrep
                                            final_cmd =
                                                format!("rg -p {}", args).trim().to_string();
                                        } else {
                                            // fallback grep
                                            final_cmd = format!("grep --color=always -rn {}", args)
                                                .trim()
                                                .to_string();
                                        }
                                    }

                                    // Save for future macro use
                                    let parts =
                                        crate::app::console_state::split_shell_words(&final_cmd)
                                            .unwrap_or_else(|_| {
                                                final_cmd
                                                    .split_whitespace()
                                                    .map(ToOwned::to_owned)
                                                    .collect()
                                            });
                                    self.console_state.last_command = Some(final_cmd.clone());
                                    if parts.len() > 1 {
                                        self.console_state.last_args = parts.last().cloned();
                                    } else {
                                        self.console_state.last_args = None;
                                    }

                                    self.spawn_console_command(final_cmd);
                                }
                            }
                            return Ok(true);
                        }
                        KeyCode::Tab => {
                            // Accept full ghost text on Tab as alternative to Right
                            if is_initial_press && self.console_state.ghost_text.is_some() {
                                self.console_state.accept_ghost_text();
                                self.console_state.clear_ghost_text();
                            }
                            return Ok(true);
                        }
                        _ => {}
                    }
                }
                ConsoleMode::HistorySearch => match key.code {
                    KeyCode::Esc => {
                        if is_initial_press {
                            self.console_state.exit_history_search(false);
                        }
                        return Ok(true);
                    }
                    KeyCode::Enter => {
                        if is_initial_press {
                            self.console_state.exit_history_search(true);
                            self.refresh_ghost_text();
                        }
                        return Ok(true);
                    }
                    KeyCode::Up => {
                        if is_initial_press {
                            self.console_state.history_search_up();
                        }
                        return Ok(true);
                    }
                    KeyCode::Down => {
                        if is_initial_press {
                            self.console_state.history_search_down();
                        }
                        return Ok(true);
                    }
                    KeyCode::Char(c) => {
                        if is_initial_press {
                            self.console_state.history_search_query.push(c);
                            self.update_history_search_results();
                        }
                        return Ok(true);
                    }
                    KeyCode::Backspace => {
                        if is_initial_press {
                            self.console_state.history_search_query.pop();
                            self.update_history_search_results();
                        }
                        return Ok(true);
                    }
                    _ => {}
                },
                ConsoleMode::Confirm => {
                    match key.code {
                        KeyCode::Enter => {
                            if is_initial_press {
                                // Execute the confirmed command
                                if let Some(cmd) = self.console_state.confirm_command.take() {
                                    self.console_state.confirm_action = None;
                                    self.console_state.mode = ConsoleMode::Normal;
                                    self.spawn_console_command(cmd);
                                }
                            }
                            return Ok(true);
                        }
                        KeyCode::Esc => {
                            if is_initial_press {
                                // Cancel — return to Normal mode
                                self.console_state.confirm_command = None;
                                self.console_state.confirm_action = None;
                                self.console_state.mode = ConsoleMode::Normal;
                            }
                            return Ok(true);
                        }
                        _ => {
                            return Ok(true);
                        }
                    }
                }
            }
        }

        // Handle tab-specific hotkeys first
        if self.tab_manager.current() == TabType::Cpu {
            let process_count = self
                .cpu_data
                .read()
                .as_ref()
                .map(|d| d.top_processes.len())
                .unwrap_or(0);
            match key.code {
                KeyCode::Up => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    if self.cpu_state.selected_index > 0 {
                        self.cpu_state.selected_index -= 1;
                        if self.cpu_state.selected_index < self.cpu_state.scroll_offset {
                            self.cpu_state.scroll_offset = self.cpu_state.selected_index;
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Down => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    if self.cpu_state.selected_index + 1 < process_count {
                        self.cpu_state.selected_index += 1;
                    }
                    return Ok(true);
                }
                KeyCode::PageUp => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    self.cpu_state.selected_index =
                        self.cpu_state.selected_index.saturating_sub(10);
                    self.cpu_state.scroll_offset = self.cpu_state.selected_index;
                    return Ok(true);
                }
                KeyCode::PageDown => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    if self.cpu_state.selected_index + 10 < process_count {
                        self.cpu_state.selected_index += 10;
                    } else if process_count > 0 {
                        self.cpu_state.selected_index = process_count - 1;
                    }
                    return Ok(true);
                }
                KeyCode::Char('p') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.toggle_cpu_sort(CpuProcessSortColumn::Pid);
                    return Ok(true);
                }
                KeyCode::Char('n') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.toggle_cpu_sort(CpuProcessSortColumn::Name);
                    return Ok(true);
                }
                KeyCode::Char('c') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.toggle_cpu_sort(CpuProcessSortColumn::Cpu);
                    return Ok(true);
                }
                KeyCode::Char('m') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.toggle_cpu_sort(CpuProcessSortColumn::Memory);
                    return Ok(true);
                }
                KeyCode::Char('t') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.toggle_cpu_sort(CpuProcessSortColumn::Threads);
                    return Ok(true);
                }
                KeyCode::Home => {
                    self.cpu_state.selected_index = 0;
                    self.cpu_state.scroll_offset = 0;
                    return Ok(true);
                }
                KeyCode::End => {
                    if process_count > 0 {
                        self.cpu_state.selected_index = process_count - 1;
                    }
                    return Ok(true);
                }
                _ => {}
            }
        }

        if self.tab_manager.current() == TabType::Processes {
            match key.code {
                KeyCode::Up => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    if self.processes_state.selected_index > 0 {
                        self.processes_state.selected_index -= 1;
                        if self.processes_state.selected_index < self.processes_state.scroll_offset
                        {
                            self.processes_state.scroll_offset =
                                self.processes_state.selected_index;
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Down => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    let process_count = self
                        .process_data
                        .read()
                        .as_ref()
                        .map(|d| d.processes.len())
                        .unwrap_or(0);
                    if self.processes_state.selected_index + 1 < process_count {
                        self.processes_state.selected_index += 1;
                    }
                    return Ok(true);
                }
                KeyCode::PageUp => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    if self.processes_state.selected_index >= 10 {
                        self.processes_state.selected_index -= 10;
                    } else {
                        self.processes_state.selected_index = 0;
                    }
                    self.processes_state.scroll_offset = self.processes_state.selected_index;
                    return Ok(true);
                }
                KeyCode::PageDown => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    let process_count = self
                        .process_data
                        .read()
                        .as_ref()
                        .map(|d| d.processes.len())
                        .unwrap_or(0);
                    if self.processes_state.selected_index + 10 < process_count {
                        self.processes_state.selected_index += 10;
                    } else if process_count > 0 {
                        self.processes_state.selected_index = process_count - 1;
                    }
                    return Ok(true);
                }
                KeyCode::Char('p') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.processes_state.sort_column = ProcessSortColumn::Pid;
                    self.processes_state.sort_ascending = !self.processes_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('n') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.processes_state.sort_column = ProcessSortColumn::Name;
                    self.processes_state.sort_ascending = !self.processes_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('c') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.processes_state.sort_column = ProcessSortColumn::Cpu;
                    self.processes_state.sort_ascending = !self.processes_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('m') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.processes_state.sort_column = ProcessSortColumn::Memory;
                    self.processes_state.sort_ascending = !self.processes_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('t') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.processes_state.sort_column = ProcessSortColumn::Threads;
                    self.processes_state.sort_ascending = !self.processes_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('u') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.processes_state.sort_column = ProcessSortColumn::User;
                    self.processes_state.sort_ascending = !self.processes_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('/') => {
                    // Enter filter mode (will be handled in UI)
                    return Ok(true);
                }
                _ => {}
            }
        }

        if self.tab_manager.current() == TabType::Gpu {
            let process_count = self
                .gpu_data
                .read()
                .as_ref()
                .map(|d| d.processes.len())
                .unwrap_or(0);
            match key.code {
                KeyCode::Up => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    if self.gpu_state.selected_index > 0 {
                        self.gpu_state.selected_index -= 1;
                    }
                    return Ok(true);
                }
                KeyCode::Down => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    if self.gpu_state.selected_index + 1 < process_count {
                        self.gpu_state.selected_index += 1;
                    }
                    return Ok(true);
                }
                KeyCode::PageUp => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    let step = 10usize;
                    self.gpu_state.selected_index =
                        self.gpu_state.selected_index.saturating_sub(step);
                    return Ok(true);
                }
                KeyCode::PageDown => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    let step = 10usize;
                    if process_count > 0 {
                        let next = self.gpu_state.selected_index + step;
                        self.gpu_state.selected_index = next.min(process_count.saturating_sub(1));
                    }
                    return Ok(true);
                }
                KeyCode::Char('p') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.toggle_gpu_sort(GpuProcessSortColumn::Pid);
                    return Ok(true);
                }
                KeyCode::Char('n') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.toggle_gpu_sort(GpuProcessSortColumn::Name);
                    return Ok(true);
                }
                KeyCode::Char('g') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.toggle_gpu_sort(GpuProcessSortColumn::Gpu);
                    return Ok(true);
                }
                KeyCode::Char('m') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.toggle_gpu_sort(GpuProcessSortColumn::Memory);
                    return Ok(true);
                }
                KeyCode::Char('t') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.toggle_gpu_sort(GpuProcessSortColumn::Type);
                    return Ok(true);
                }
                _ => {}
            }
        }

        if self.tab_manager.current() == TabType::Ram {
            let process_count = self
                .ram_data
                .read()
                .as_ref()
                .map(|d| d.top_processes.len())
                .unwrap_or(0);
            match key.code {
                KeyCode::Left | KeyCode::Right => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    self.ram_state.focused_panel = match self.ram_state.focused_panel {
                        RamPanelFocus::Breakdown => RamPanelFocus::TopProcesses,
                        RamPanelFocus::TopProcesses => RamPanelFocus::Breakdown,
                    };
                    return Ok(true);
                }
                KeyCode::Up => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    if self.ram_state.focused_panel == RamPanelFocus::TopProcesses
                        && self.ram_state.selected_index > 0
                    {
                        self.ram_state.selected_index -= 1;
                    }
                    return Ok(true);
                }
                KeyCode::Down => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    if self.ram_state.focused_panel == RamPanelFocus::TopProcesses
                        && self.ram_state.selected_index + 1 < process_count
                    {
                        self.ram_state.selected_index += 1;
                    }
                    return Ok(true);
                }
                KeyCode::PageUp => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    let step = 10usize;
                    if self.ram_state.focused_panel == RamPanelFocus::TopProcesses {
                        self.ram_state.selected_index =
                            self.ram_state.selected_index.saturating_sub(step);
                    }
                    return Ok(true);
                }
                KeyCode::PageDown => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    let step = 10usize;
                    if self.ram_state.focused_panel == RamPanelFocus::TopProcesses
                        && process_count > 0
                    {
                        let next = self.ram_state.selected_index + step;
                        self.ram_state.selected_index = next.min(process_count.saturating_sub(1));
                    }
                    return Ok(true);
                }
                KeyCode::Char('p') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.ram_state.sort_column = RamProcessSortColumn::Pid;
                    self.ram_state.sort_ascending = !self.ram_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('n') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.ram_state.sort_column = RamProcessSortColumn::Name;
                    self.ram_state.sort_ascending = !self.ram_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('w') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.ram_state.sort_column = RamProcessSortColumn::WorkingSet;
                    self.ram_state.sort_ascending = !self.ram_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('b') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    self.ram_state.sort_column = RamProcessSortColumn::PrivateBytes;
                    self.ram_state.sort_ascending = !self.ram_state.sort_ascending;
                    return Ok(true);
                }
                _ => {}
            }
        }

        if self.tab_manager.current() == TabType::Network {
            // --- Input mode: text editing in Parameters zone ---
            if self.network_ui_state.input_mode {
                match key.code {
                    KeyCode::Esc => {
                        if is_initial_press {
                            self.network_ui_state.input_mode = false;
                            self.network_ui_state.focus = NetworkFocusZone::Tools;
                            self.network_ui_state.nat_mapping_confirm_until = None;
                        }
                        return Ok(true);
                    }
                    KeyCode::Enter => {
                        if is_initial_press {
                            self.network_ui_state.input_mode = false;
                            self.network_ui_state.focus = NetworkFocusZone::Tools;
                            self.start_selected_network_diagnostic();
                        }
                        return Ok(true);
                    }
                    KeyCode::Backspace => {
                        if is_initial_press {
                            self.network_ui_state.target_input.pop();
                            self.network_ui_state.nat_mapping_confirm_until = None;
                        }
                        return Ok(true);
                    }
                    KeyCode::Char(c) => {
                        if is_initial_press
                            && !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT)
                        {
                            self.network_ui_state.target_input.push(c);
                            self.network_ui_state.nat_mapping_confirm_until = None;
                        }
                        return Ok(true);
                    }
                    _ => {}
                }
            }

            // --- Filter mode: text editing for result filtering ---
            if self.network_ui_state.filter_active {
                match key.code {
                    KeyCode::Esc => {
                        if is_initial_press {
                            self.network_ui_state.filter_active = false;
                            self.network_ui_state.filter_input.clear();
                        }
                        return Ok(true);
                    }
                    KeyCode::Enter => {
                        if is_initial_press {
                            // Keep filter active but stop editing (apply filter)
                            self.network_ui_state.filter_active = false;
                            // Don't clear — keep filter_input for display
                        }
                        return Ok(true);
                    }
                    KeyCode::Backspace => {
                        if is_initial_press {
                            self.network_ui_state.filter_input.pop();
                            if self.network_ui_state.filter_input.is_empty() {
                                self.network_ui_state.filter_active = false;
                            }
                        }
                        return Ok(true);
                    }
                    KeyCode::Char(c) => {
                        if is_initial_press
                            && !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT)
                        {
                            self.network_ui_state.filter_input.push(c);
                        }
                        return Ok(true);
                    }
                    _ => {}
                }
            }

            // --- Global Network tab keys (work in any focus zone) ---
            match key.code {
                // Backtick (`) / Shift+Backtick: cycle focus zones
                // NOTE: Tab/BackTab are NOT intercepted so they pass through
                // to the global handler which switches between main tabs.
                KeyCode::Char('`') => {
                    if is_initial_press {
                        if key.modifiers.contains(KeyModifiers::SHIFT) {
                            self.network_ui_state.focus = self.network_ui_state.focus.prev();
                        } else {
                            self.network_ui_state.focus = self.network_ui_state.focus.next();
                        }
                    }
                    return Ok(true);
                }

                // Left/Right: switch result sub-tabs when Results focused,
                // exit Results zone at boundaries; cycle focus zones otherwise
                KeyCode::Left => {
                    if is_initial_press {
                        match self.network_ui_state.focus {
                            NetworkFocusZone::Results => {
                                if self.network_ui_state.result_tab == NetworkResultTab::Summary {
                                    // At left boundary — exit Results to prev zone
                                    self.network_ui_state.focus =
                                        self.network_ui_state.focus.prev();
                                } else {
                                    self.network_ui_state.result_tab =
                                        self.network_ui_state.result_tab.prev();
                                }
                            }
                            _ => {
                                self.network_ui_state.focus = self.network_ui_state.focus.prev();
                            }
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Right => {
                    if is_initial_press {
                        match self.network_ui_state.focus {
                            NetworkFocusZone::Results => {
                                if self.network_ui_state.result_tab == NetworkResultTab::History {
                                    // At right boundary — exit Results to next zone
                                    self.network_ui_state.focus =
                                        self.network_ui_state.focus.next();
                                } else {
                                    self.network_ui_state.result_tab =
                                        self.network_ui_state.result_tab.next();
                                }
                            }
                            _ => {
                                self.network_ui_state.focus = self.network_ui_state.focus.next();
                            }
                        }
                    }
                    return Ok(true);
                }

                // Up/Down: context-dependent navigation
                KeyCode::Up => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    match self.network_ui_state.focus {
                        NetworkFocusZone::Tools | NetworkFocusZone::Parameters => {
                            // Both Tools and Parameters zones navigate tools up
                            self.network_ui_state.selected_tool =
                                self.network_ui_state.selected_tool.previous();
                            self.network_ui_state.nat_mapping_confirm_until = None;
                        }
                        NetworkFocusZone::Results => {
                            self.network_ui_state.detail_scroll =
                                self.network_ui_state.detail_scroll.saturating_sub(1);
                        }
                        NetworkFocusZone::Activity => {
                            self.network_ui_state.activity_scroll =
                                self.network_ui_state.activity_scroll.saturating_sub(1);
                        }
                        NetworkFocusZone::Interface => match self.network_ui_state.center_view {
                            NetworkCenterView::Connections => {
                                self.network_ui_state.connections_scroll =
                                    self.network_ui_state.connections_scroll.saturating_sub(1);
                            }
                            NetworkCenterView::Interface => {
                                self.network_ui_state.selected_interface_idx = self
                                    .network_ui_state
                                    .selected_interface_idx
                                    .saturating_sub(1);
                            }
                        },
                    }
                    return Ok(true);
                }
                KeyCode::Down => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    match self.network_ui_state.focus {
                        NetworkFocusZone::Tools | NetworkFocusZone::Parameters => {
                            // Both Tools and Parameters zones navigate tools down
                            self.network_ui_state.selected_tool =
                                self.network_ui_state.selected_tool.next();
                            self.network_ui_state.nat_mapping_confirm_until = None;
                        }
                        NetworkFocusZone::Results => {
                            self.network_ui_state.detail_scroll =
                                self.network_ui_state.detail_scroll.saturating_add(1);
                        }
                        NetworkFocusZone::Activity => {
                            self.network_ui_state.activity_scroll =
                                self.network_ui_state.activity_scroll.saturating_add(1);
                        }
                        NetworkFocusZone::Interface => match self.network_ui_state.center_view {
                            NetworkCenterView::Connections => {
                                self.network_ui_state.connections_scroll =
                                    self.network_ui_state.connections_scroll.saturating_add(1);
                            }
                            NetworkCenterView::Interface => {
                                self.network_ui_state.selected_interface_idx = self
                                    .network_ui_state
                                    .selected_interface_idx
                                    .saturating_add(1);
                            }
                        },
                    }
                    return Ok(true);
                }

                // PgUp/PgDn/Home/End: scroll in Results/Activity/Connections
                KeyCode::PageUp => {
                    if !self.allow_widget_scroll() {
                        return Ok(true);
                    }
                    match self.network_ui_state.focus {
                        NetworkFocusZone::Interface => {
                            self.network_ui_state.connections_scroll =
                                self.network_ui_state.connections_scroll.saturating_sub(8);
                        }
                        NetworkFocusZone::Activity => {
                            self.network_ui_state.activity_scroll =
                                self.network_ui_state.activity_scroll.saturating_sub(8);
                        }
                        _ => {
                            self.network_ui_state.detail_scroll =
                                self.network_ui_state.detail_scroll.saturating_sub(8);
                        }
                    }
                    return Ok(true);
                }
                KeyCode::PageDown => {
                    if !self.allow_widget_scroll() {
                        return Ok(true);
                    }
                    match self.network_ui_state.focus {
                        NetworkFocusZone::Interface => {
                            self.network_ui_state.connections_scroll =
                                self.network_ui_state.connections_scroll.saturating_add(8);
                        }
                        NetworkFocusZone::Activity => {
                            self.network_ui_state.activity_scroll =
                                self.network_ui_state.activity_scroll.saturating_add(8);
                        }
                        _ => {
                            self.network_ui_state.detail_scroll =
                                self.network_ui_state.detail_scroll.saturating_add(8);
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Home => {
                    if !self.allow_widget_scroll() {
                        return Ok(true);
                    }
                    self.network_ui_state.detail_scroll = 0;
                    self.network_ui_state.connections_scroll = 0;
                    self.network_ui_state.activity_scroll = 0;
                    return Ok(true);
                }
                KeyCode::End => {
                    if !self.allow_widget_scroll() {
                        return Ok(true);
                    }
                    self.network_ui_state.detail_scroll = usize::MAX / 2;
                    self.network_ui_state.connections_scroll = usize::MAX / 2;
                    self.network_ui_state.activity_scroll = usize::MAX / 2;
                    return Ok(true);
                }

                // Enter: run tool (from Tools or Parameters zone)
                KeyCode::Enter => {
                    if is_initial_press {
                        self.start_selected_network_diagnostic();
                    }
                    return Ok(true);
                }

                // i: enter input mode (jump to Parameters zone)
                KeyCode::Char('i') => {
                    if is_initial_press {
                        self.network_ui_state.input_mode = true;
                        self.network_ui_state.focus = NetworkFocusZone::Parameters;
                    }
                    return Ok(true);
                }

                // x: cancel running job
                KeyCode::Char('x') => {
                    if is_initial_press {
                        self.cancel_network_diagnostic();
                    }
                    return Ok(true);
                }

                // k: clear activity log
                KeyCode::Char('k') => {
                    if is_initial_press {
                        self.network_ui_state.event_log.clear();
                        self.network_ui_state.activity_scroll = 0;
                    }
                    return Ok(true);
                }

                // v: toggle center view (Interface <-> Connections)
                KeyCode::Char('v') => {
                    if is_initial_press {
                        self.network_ui_state.center_view = match self.network_ui_state.center_view
                        {
                            NetworkCenterView::Interface => NetworkCenterView::Connections,
                            NetworkCenterView::Connections => NetworkCenterView::Interface,
                        };
                    }
                    return Ok(true);
                }

                // 0: toggle traffic marker (RX/TX reset point)
                KeyCode::Char('0') => {
                    if is_initial_press {
                        if self.network_ui_state.show_marker_traffic {
                            // Already showing marker → switch back to global
                            self.network_ui_state.show_marker_traffic = false;
                        } else if self.network_ui_state.traffic_marker.is_some() {
                            // Marker exists but showing global → reset marker to new point
                            self.network_ui_state.traffic_marker = None;
                            // Will be set on next render with current values
                            self.network_set_traffic_marker();
                            self.network_ui_state.show_marker_traffic = true;
                        } else {
                            // No marker yet → create one
                            self.network_set_traffic_marker();
                            self.network_ui_state.show_marker_traffic = true;
                        }
                    }
                    return Ok(true);
                }

                // /: enter filter mode for results
                KeyCode::Char('/') => {
                    if is_initial_press && !self.network_ui_state.input_mode {
                        self.network_ui_state.filter_active = true;
                        self.network_ui_state.filter_input.clear();
                        self.network_ui_state.focus = NetworkFocusZone::Results;
                    }
                    return Ok(true);
                }

                // 1-4: apply parameter preset for current tool
                KeyCode::Char(c @ '1'..='4') => {
                    if is_initial_press {
                        let preset_idx = (c as u8 - b'1') as usize;
                        let tool = self.network_ui_state.selected_tool;
                        let presets = tool.presets();
                        if let Some((_label, value)) = presets.get(preset_idx) {
                            self.network_ui_state.target_input = value.to_string();
                        }
                    }
                    return Ok(true);
                }
                _ => {}
            }
        }

        // Services tab hotkeys
        if self.tab_manager.current() == TabType::Services {
            match key.code {
                KeyCode::Left | KeyCode::Right => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    if self.compact_mode {
                        self.services_state.focused_panel = ServicesPanelFocus::Table;
                    } else {
                        self.services_state.focused_panel = match self.services_state.focused_panel
                        {
                            ServicesPanelFocus::Table => ServicesPanelFocus::Details,
                            ServicesPanelFocus::Details => ServicesPanelFocus::Table,
                        };
                        if self.services_state.focused_panel == ServicesPanelFocus::Table {
                            self.services_state.details_scroll = 0;
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Up => {
                    if self.services_state.focused_panel == ServicesPanelFocus::Details {
                        if !self.allow_widget_scroll() {
                            return Ok(true);
                        }
                        self.services_state.details_scroll =
                            self.services_state.details_scroll.saturating_sub(1);
                        return Ok(true);
                    }
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    if self.services_state.selected_index > 0 {
                        self.services_state.selected_index -= 1;
                        if self.services_state.selected_index < self.services_state.scroll_offset {
                            self.services_state.scroll_offset = self.services_state.selected_index;
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Down => {
                    if self.services_state.focused_panel == ServicesPanelFocus::Details {
                        if !self.allow_widget_scroll() {
                            return Ok(true);
                        }
                        self.services_state.details_scroll += 1;
                        return Ok(true);
                    }
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    let service_count = self
                        .service_data
                        .read()
                        .as_ref()
                        .map(|d| d.services.len())
                        .unwrap_or(0);
                    if self.services_state.selected_index + 1 < service_count {
                        self.services_state.selected_index += 1;
                    }
                    return Ok(true);
                }
                KeyCode::PageUp => {
                    if self.services_state.focused_panel == ServicesPanelFocus::Details {
                        if !self.allow_widget_scroll() {
                            return Ok(true);
                        }
                        self.services_state.details_scroll =
                            self.services_state.details_scroll.saturating_sub(10);
                        return Ok(true);
                    }
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    if self.services_state.selected_index >= 10 {
                        self.services_state.selected_index -= 10;
                    } else {
                        self.services_state.selected_index = 0;
                    }
                    self.services_state.scroll_offset = self.services_state.selected_index;
                    return Ok(true);
                }
                KeyCode::PageDown => {
                    if self.services_state.focused_panel == ServicesPanelFocus::Details {
                        if !self.allow_widget_scroll() {
                            return Ok(true);
                        }
                        self.services_state.details_scroll += 10;
                        return Ok(true);
                    }
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    let service_count = self
                        .service_data
                        .read()
                        .as_ref()
                        .map(|d| d.services.len())
                        .unwrap_or(0);
                    if self.services_state.selected_index + 10 < service_count {
                        self.services_state.selected_index += 10;
                    } else if service_count > 0 {
                        self.services_state.selected_index = service_count - 1;
                    }
                    return Ok(true);
                }
                KeyCode::Char('n') => {
                    if self.services_state.focused_panel != ServicesPanelFocus::Table
                        || !is_initial_press
                        || !self.allow_sort_toggle()
                    {
                        return Ok(true);
                    }
                    self.services_state.sort_column = ServiceSortColumn::Name;
                    self.services_state.sort_ascending = !self.services_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('d') => {
                    if self.services_state.focused_panel != ServicesPanelFocus::Table
                        || !is_initial_press
                        || !self.allow_sort_toggle()
                    {
                        return Ok(true);
                    }
                    self.services_state.sort_column = ServiceSortColumn::DisplayName;
                    self.services_state.sort_ascending = !self.services_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('s') => {
                    if self.services_state.focused_panel != ServicesPanelFocus::Table
                        || !is_initial_press
                        || !self.allow_sort_toggle()
                    {
                        return Ok(true);
                    }
                    self.services_state.sort_column = ServiceSortColumn::Status;
                    self.services_state.sort_ascending = !self.services_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('t') => {
                    if self.services_state.focused_panel != ServicesPanelFocus::Table
                        || !is_initial_press
                        || !self.allow_sort_toggle()
                    {
                        return Ok(true);
                    }
                    self.services_state.sort_column = ServiceSortColumn::StartType;
                    self.services_state.sort_ascending = !self.services_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('f') => {
                    // Cycle through filter options
                    self.services_state.status_filter = match self.services_state.status_filter {
                        ServiceStatusFilter::All => ServiceStatusFilter::Running,
                        ServiceStatusFilter::Running => ServiceStatusFilter::Stopped,
                        ServiceStatusFilter::Stopped => ServiceStatusFilter::All,
                    };
                    return Ok(true);
                }
                _ => {}
            }
        }

        // Ollama tab hotkeys
        if self.tab_manager.current() == TabType::Ollama {
            if self.ollama_state.show_delete_confirm {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Enter => {
                        if let Some(target) = self.ollama_state.pending_delete.clone() {
                            match target {
                                OllamaDeleteTarget::Model(model_name) => {
                                    tokio::spawn(async move {
                                        use crate::integrations::OllamaClient;
                                        if let Ok(client) = OllamaClient::new(None) {
                                            let _ = client.remove_model(&model_name).await;
                                        }
                                    });
                                }
                                OllamaDeleteTarget::ChatLog(entry) => {
                                    let log_path = entry.path.clone();
                                    let meta_path =
                                        std::path::PathBuf::from(&log_path).with_extension("toml");
                                    let _ = fs::remove_file(&log_path);
                                    let _ = fs::remove_file(&meta_path);
                                    if let Some(data) = self.ollama_data.write().as_mut() {
                                        data.chat_logs.retain(|item| item.path != entry.path);
                                    }
                                }
                            }
                        }
                        self.ollama_state.pending_delete = None;
                        self.ollama_state.show_delete_confirm = false;
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.ollama_state.pending_delete = None;
                        self.ollama_state.show_delete_confirm = false;
                    }
                    _ => {}
                }
                return Ok(true);
            }

            if self.ollama_state.focused_panel == OllamaPanelFocus::Input
                || matches!(
                    self.ollama_state.input_mode,
                    OllamaInputMode::Pull | OllamaInputMode::Command
                )
            {
                match key.code {
                    KeyCode::Tab if is_initial_press => {
                        self.ollama_state.focused_panel =
                            self.next_ollama_focus(self.ollama_state.focused_panel);
                        self.maybe_start_activity_expand_timer();
                    }
                    KeyCode::BackTab if is_initial_press => {
                        self.ollama_state.focused_panel =
                            self.prev_ollama_focus(self.ollama_state.focused_panel);
                        self.maybe_start_activity_expand_timer();
                    }
                    KeyCode::Left => {
                        if self.allow_horizontal_nav() {
                            self.ollama_state.focused_panel =
                                self.prev_ollama_focus(self.ollama_state.focused_panel);
                            self.maybe_start_activity_expand_timer();
                        }
                    }
                    KeyCode::Right => {
                        if self.allow_horizontal_nav() {
                            self.ollama_state.focused_panel =
                                self.next_ollama_focus(self.ollama_state.focused_panel);
                            self.maybe_start_activity_expand_timer();
                        }
                    }
                    KeyCode::Enter => match self.ollama_state.input_mode {
                        OllamaInputMode::Pull => {
                            let model_name = self.ollama_state.input_buffer.trim().to_string();
                            if !model_name.is_empty() {
                                tokio::spawn(async move {
                                    use crate::integrations::OllamaClient;
                                    if let Ok(client) = OllamaClient::new(None) {
                                        let _ = client.pull_model(&model_name).await;
                                    }
                                });
                            }
                            self.ollama_state.input_buffer.clear();
                            self.ollama_state.input_mode = OllamaInputMode::None;
                            self.ollama_state.focused_panel = OllamaPanelFocus::Main;
                        }
                        OllamaInputMode::Command => {
                            let command = self.ollama_state.input_buffer.trim().to_string();
                            if !command.is_empty() {
                                self.spawn_ollama_command(command);
                            }
                            self.ollama_state.input_buffer.clear();
                            self.ollama_state.input_mode = OllamaInputMode::None;
                        }
                        OllamaInputMode::Chat => {
                            let prompt = self.ollama_state.input_buffer.trim().to_string();
                            if !prompt.is_empty() {
                                self.spawn_ollama_chat_prompt(prompt);
                            }
                            self.ollama_state.input_buffer.clear();
                            self.ollama_state.chat_prompt_scroll = 0;
                        }
                        OllamaInputMode::None => {}
                    },
                    KeyCode::Esc => {
                        if self.ollama_state.input_mode == OllamaInputMode::Chat
                            && self.ollama_state.chat_active
                        {
                            self.finish_ollama_chat();
                        } else {
                            self.ollama_state.input_buffer.clear();
                            self.ollama_state.input_mode = OllamaInputMode::None;
                            self.ollama_state.focused_panel = OllamaPanelFocus::Main;
                        }
                    }
                    KeyCode::Backspace => {
                        self.ollama_state.input_buffer.pop();
                    }
                    KeyCode::Up | KeyCode::Down
                        if self.ollama_state.input_mode == OllamaInputMode::Chat =>
                    {
                        if !self.allow_widget_scroll() {
                            return Ok(true);
                        }
                        let max_height = self.max_chat_prompt_height();
                        let max_scroll = self.max_chat_prompt_scroll();
                        if key.code == KeyCode::Up {
                            if max_scroll > 0 && self.ollama_state.chat_prompt_scroll > 0 {
                                self.ollama_state.chat_prompt_scroll -= 1;
                            } else if self.ollama_state.chat_prompt_height < max_height {
                                self.ollama_state.chat_prompt_height += 1;
                            }
                        } else if max_scroll > 0
                            && self.ollama_state.chat_prompt_scroll < max_scroll
                        {
                            self.ollama_state.chat_prompt_scroll += 1;
                        } else if self.ollama_state.chat_prompt_height > 3 {
                            self.ollama_state.chat_prompt_height -= 1;
                        }
                    }
                    KeyCode::Char(c) => {
                        if self.ollama_state.input_mode == OllamaInputMode::None {
                            return Ok(true);
                        }
                        let allow_input = if self.ollama_state.input_mode == OllamaInputMode::Chat {
                            matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
                                && self.allow_text_input()
                        } else {
                            self.allow_text_input()
                        };
                        if allow_input {
                            self.ollama_state.input_buffer.push(c);
                        }
                    }
                    _ => {}
                }
                return Ok(true);
            }

            match key.code {
                KeyCode::Char('n') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    if self.ollama_state.focused_panel == OllamaPanelFocus::Main
                        && !self.ollama_state.chat_active
                    {
                        match self.ollama_state.current_view {
                            OllamaView::Models => {
                                self.toggle_model_sort(OllamaModelSortColumn::Name);
                            }
                            OllamaView::Running => {
                                self.toggle_running_sort(OllamaRunningSortColumn::Name);
                            }
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Char('m') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    if self.ollama_state.focused_panel == OllamaPanelFocus::Main
                        && !self.ollama_state.chat_active
                    {
                        match self.ollama_state.current_view {
                            OllamaView::Models => {
                                self.toggle_model_sort(OllamaModelSortColumn::Params);
                            }
                            OllamaView::Running => {
                                self.toggle_running_sort(OllamaRunningSortColumn::Params);
                            }
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Char('t') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    if self.ollama_state.focused_panel == OllamaPanelFocus::Main
                        && !self.ollama_state.chat_active
                    {
                        match self.ollama_state.current_view {
                            OllamaView::Models => {
                                self.toggle_model_sort(OllamaModelSortColumn::Modified);
                            }
                            OllamaView::Running => {
                                self.toggle_running_sort(OllamaRunningSortColumn::PausedAt);
                            }
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Char('g') => {
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    if self.ollama_state.focused_panel == OllamaPanelFocus::Main
                        && !self.ollama_state.chat_active
                        && self.ollama_state.current_view == OllamaView::Running
                    {
                        self.toggle_running_sort(OllamaRunningSortColumn::MessageCount);
                    }
                    return Ok(true);
                }
                KeyCode::Char('a') => {
                    if !is_initial_press {
                        return Ok(true);
                    }
                    if self.ollama_state.focused_panel == OllamaPanelFocus::Activity
                        && self.ollama_state.activity_view == OllamaActivityView::List
                    {
                        self.ollama_state.activity_additions_open = true;
                        self.ollama_state.activity_additions_selected = 0;
                        return Ok(true);
                    }
                    return Ok(true);
                }
                KeyCode::Left => {
                    if !self.allow_horizontal_nav() {
                        return Ok(true);
                    }
                    self.ollama_state.focused_panel =
                        self.prev_ollama_focus(self.ollama_state.focused_panel);
                    self.maybe_start_activity_expand_timer();
                    return Ok(true);
                }
                KeyCode::Right => {
                    if !self.allow_horizontal_nav() {
                        return Ok(true);
                    }
                    self.ollama_state.focused_panel =
                        self.next_ollama_focus(self.ollama_state.focused_panel);
                    self.maybe_start_activity_expand_timer();
                    return Ok(true);
                }
                KeyCode::Enter => {
                    if self.ollama_state.focused_panel == OllamaPanelFocus::Additions
                        && self.ollama_state.activity_additions_open
                        && self.ollama_state.activity_view == OllamaActivityView::List
                    {
                        let entry = self.ollama_data.read().as_ref().and_then(|data| {
                            let idx = self
                                .ollama_state
                                .activity_selected
                                .min(data.chat_logs.len().saturating_sub(1));
                            data.chat_logs.get(idx).cloned()
                        });
                        if let Some(entry) = entry {
                            self.restart_chat_from_log(entry.model, entry.path);
                        }
                        self.close_activity_additions();
                        return Ok(true);
                    }
                    if self.ollama_state.focused_panel == OllamaPanelFocus::Main
                        && self.ollama_state.current_view == OllamaView::Running
                        && !self.ollama_state.chat_active
                    {
                        let model_name = self
                            .sorted_ollama_running_models()
                            .get(self.ollama_state.selected_running_index)
                            .map(|model| model.name.clone());
                        if let Some(model_name) = model_name {
                            if self.resume_ollama_chat(&model_name) {
                                return Ok(true);
                            }
                        }
                    }
                }
                KeyCode::Esc => {
                    if self.ollama_state.focused_panel == OllamaPanelFocus::Additions
                        && self.ollama_state.activity_additions_open
                    {
                        self.close_activity_additions();
                        return Ok(true);
                    }
                    if self.ollama_state.focused_panel == OllamaPanelFocus::Activity
                        && self.ollama_state.activity_view == OllamaActivityView::List
                        && self.activity_expand_ready()
                    {
                        self.ollama_state.activity_expand_suppressed = true;
                        return Ok(true);
                    }
                    if self.ollama_state.focused_panel == OllamaPanelFocus::Activity
                        && self.ollama_state.activity_view == OllamaActivityView::Log
                    {
                        self.ollama_state.activity_view = OllamaActivityView::List;
                        self.ollama_state.activity_log_lines.clear();
                        self.ollama_state.activity_log_title.clear();
                        self.ollama_state.activity_log_scroll = 0;
                        self.maybe_start_activity_expand_timer();
                        return Ok(true);
                    }
                }
                KeyCode::Up => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    match self.ollama_state.focused_panel {
                        OllamaPanelFocus::Main => {
                            if self.ollama_state.chat_active {
                                if !self.allow_widget_scroll() {
                                    return Ok(true);
                                }
                                self.ollama_state.chat_scroll =
                                    self.ollama_state.chat_scroll.saturating_sub(1);
                            } else {
                                match self.ollama_state.current_view {
                                    OllamaView::Models => {
                                        if self.ollama_state.selected_model_index > 0 {
                                            self.ollama_state.selected_model_index -= 1;
                                        }
                                    }
                                    OllamaView::Running => {
                                        if self.ollama_state.selected_running_index > 0 {
                                            self.ollama_state.selected_running_index -= 1;
                                        }
                                    }
                                }
                            }
                        }
                        OllamaPanelFocus::Activity => match self.ollama_state.activity_view {
                            OllamaActivityView::List => {
                                let prev = self.ollama_state.activity_selected;
                                if self.ollama_state.activity_selected > 0 {
                                    self.ollama_state.activity_selected -= 1;
                                }
                                if self.ollama_state.activity_selected != prev {
                                    self.reset_activity_expand_state();
                                }
                            }
                            OllamaActivityView::Log => {
                                if !self.allow_widget_scroll() {
                                    return Ok(true);
                                }
                                self.ollama_state.activity_log_scroll =
                                    self.ollama_state.activity_log_scroll.saturating_sub(1);
                            }
                        },
                        OllamaPanelFocus::Vram => {
                            if !self.allow_widget_scroll() {
                                return Ok(true);
                            }
                            self.ollama_state.running_summary_scroll =
                                self.ollama_state.running_summary_scroll.saturating_sub(1);
                        }
                        OllamaPanelFocus::Additions => {
                            if self.ollama_state.activity_additions_open
                                && self.ollama_state.activity_additions_selected > 0
                            {
                                self.ollama_state.activity_additions_selected -= 1;
                            }
                        }
                        _ => {}
                    }
                    return Ok(true);
                }
                KeyCode::Down => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    match self.ollama_state.focused_panel {
                        OllamaPanelFocus::Main => {
                            if self.ollama_state.chat_active {
                                if !self.allow_widget_scroll() {
                                    return Ok(true);
                                }
                                self.ollama_state.chat_scroll += 1;
                            } else {
                                match self.ollama_state.current_view {
                                    OllamaView::Models => {
                                        let model_count = self
                                            .ollama_data
                                            .read()
                                            .as_ref()
                                            .map(|d| d.models.len())
                                            .unwrap_or(0);
                                        if self.ollama_state.selected_model_index + 1 < model_count
                                        {
                                            self.ollama_state.selected_model_index += 1;
                                        }
                                    }
                                    OllamaView::Running => {
                                        let running_count =
                                            self.sorted_ollama_running_models().len();
                                        if self.ollama_state.selected_running_index + 1
                                            < running_count
                                        {
                                            self.ollama_state.selected_running_index += 1;
                                        }
                                    }
                                }
                            }
                        }
                        OllamaPanelFocus::Activity => match self.ollama_state.activity_view {
                            OllamaActivityView::List => {
                                let log_count = self
                                    .ollama_data
                                    .read()
                                    .as_ref()
                                    .map(|d| d.chat_logs.len())
                                    .unwrap_or(0);
                                let prev = self.ollama_state.activity_selected;
                                if self.ollama_state.activity_selected + 1 < log_count {
                                    self.ollama_state.activity_selected += 1;
                                }
                                if self.ollama_state.activity_selected != prev {
                                    self.reset_activity_expand_state();
                                }
                            }
                            OllamaActivityView::Log => {
                                if !self.allow_widget_scroll() {
                                    return Ok(true);
                                }
                                self.ollama_state.activity_log_scroll += 1;
                            }
                        },
                        OllamaPanelFocus::Vram => {
                            if !self.allow_widget_scroll() {
                                return Ok(true);
                            }
                            self.ollama_state.running_summary_scroll =
                                self.ollama_state.running_summary_scroll.saturating_add(1);
                        }
                        OllamaPanelFocus::Additions => {
                            let additions_len = if self.ollama_state.activity_additions_open {
                                1usize
                            } else {
                                0usize
                            };
                            if additions_len > 0
                                && self.ollama_state.activity_additions_selected + 1 < additions_len
                            {
                                self.ollama_state.activity_additions_selected += 1;
                            }
                        }
                        _ => {}
                    }
                    return Ok(true);
                }
                KeyCode::PageUp => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    let step = 10usize;
                    match self.ollama_state.focused_panel {
                        OllamaPanelFocus::Main => {
                            if self.ollama_state.chat_active {
                                if !self.allow_widget_scroll() {
                                    return Ok(true);
                                }
                                self.ollama_state.chat_scroll =
                                    self.ollama_state.chat_scroll.saturating_sub(5);
                            } else {
                                match self.ollama_state.current_view {
                                    OllamaView::Models => {
                                        self.ollama_state.selected_model_index = self
                                            .ollama_state
                                            .selected_model_index
                                            .saturating_sub(step);
                                    }
                                    OllamaView::Running => {
                                        self.ollama_state.selected_running_index = self
                                            .ollama_state
                                            .selected_running_index
                                            .saturating_sub(step);
                                    }
                                }
                            }
                        }
                        OllamaPanelFocus::Activity => match self.ollama_state.activity_view {
                            OllamaActivityView::List => {
                                let prev = self.ollama_state.activity_selected;
                                self.ollama_state.activity_selected =
                                    self.ollama_state.activity_selected.saturating_sub(step);
                                if self.ollama_state.activity_selected != prev {
                                    self.reset_activity_expand_state();
                                }
                            }
                            OllamaActivityView::Log => {
                                if !self.allow_widget_scroll() {
                                    return Ok(true);
                                }
                                self.ollama_state.activity_log_scroll =
                                    self.ollama_state.activity_log_scroll.saturating_sub(step);
                            }
                        },
                        OllamaPanelFocus::Vram => {
                            if !self.allow_widget_scroll() {
                                return Ok(true);
                            }
                            self.ollama_state.running_summary_scroll = self
                                .ollama_state
                                .running_summary_scroll
                                .saturating_sub(step);
                        }
                        _ => {}
                    }
                    return Ok(true);
                }
                KeyCode::PageDown => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    let step = 10usize;
                    match self.ollama_state.focused_panel {
                        OllamaPanelFocus::Main => {
                            if self.ollama_state.chat_active {
                                if !self.allow_widget_scroll() {
                                    return Ok(true);
                                }
                                self.ollama_state.chat_scroll += 5;
                            } else {
                                match self.ollama_state.current_view {
                                    OllamaView::Models => {
                                        let model_count = self
                                            .ollama_data
                                            .read()
                                            .as_ref()
                                            .map(|d| d.models.len())
                                            .unwrap_or(0);
                                        if model_count > 0 {
                                            let next =
                                                self.ollama_state.selected_model_index + step;
                                            self.ollama_state.selected_model_index =
                                                next.min(model_count.saturating_sub(1));
                                        }
                                    }
                                    OllamaView::Running => {
                                        let running_count =
                                            self.sorted_ollama_running_models().len();
                                        if running_count > 0 {
                                            let next =
                                                self.ollama_state.selected_running_index + step;
                                            self.ollama_state.selected_running_index =
                                                next.min(running_count.saturating_sub(1));
                                        }
                                    }
                                }
                            }
                        }
                        OllamaPanelFocus::Activity => match self.ollama_state.activity_view {
                            OllamaActivityView::List => {
                                let log_count = self
                                    .ollama_data
                                    .read()
                                    .as_ref()
                                    .map(|d| d.chat_logs.len())
                                    .unwrap_or(0);
                                if log_count > 0 {
                                    let prev = self.ollama_state.activity_selected;
                                    let next = self.ollama_state.activity_selected + step;
                                    self.ollama_state.activity_selected =
                                        next.min(log_count.saturating_sub(1));
                                    if self.ollama_state.activity_selected != prev {
                                        self.reset_activity_expand_state();
                                    }
                                }
                            }
                            OllamaActivityView::Log => {
                                if !self.allow_widget_scroll() {
                                    return Ok(true);
                                }
                                self.ollama_state.activity_log_scroll += step;
                            }
                        },
                        OllamaPanelFocus::Vram => {
                            if !self.allow_widget_scroll() {
                                return Ok(true);
                            }
                            self.ollama_state.running_summary_scroll = self
                                .ollama_state
                                .running_summary_scroll
                                .saturating_add(step);
                        }
                        _ => {}
                    }
                    return Ok(true);
                }
                KeyCode::Char('v') => {
                    if !is_initial_press || !self.allow_view_toggle() {
                        return Ok(true);
                    }
                    if self.ollama_state.chat_active {
                        self.pause_ollama_chat();
                        self.ollama_state.current_view = OllamaView::Running;
                        self.ollama_state.focused_panel = OllamaPanelFocus::Main;
                        return Ok(true);
                    }
                    self.ollama_state.current_view = match self.ollama_state.current_view {
                        OllamaView::Models => OllamaView::Running,
                        OllamaView::Running => OllamaView::Models,
                    };
                    self.ollama_state.focused_panel = OllamaPanelFocus::Main;
                    return Ok(true);
                }
                KeyCode::Char('r') => {
                    if !is_initial_press {
                        return Ok(true);
                    }
                    let model_name = match self.ollama_state.current_view {
                        OllamaView::Models => self
                            .sorted_ollama_models()
                            .get(self.ollama_state.selected_model_index)
                            .map(|model| model.name.clone()),
                        OllamaView::Running => self.selected_running_model_name(),
                    };
                    if let Some(model_name) = model_name {
                        if !self.resume_ollama_chat(&model_name) {
                            self.start_ollama_chat(model_name);
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Char('s') | KeyCode::Char('u') => {
                    let model_name = self.selected_running_model_name();
                    if let Some(model_name) = model_name {
                        if self.ollama_state.active_chat_model.as_deref()
                            == Some(model_name.as_str())
                        {
                            self.finish_ollama_chat();
                        }
                        if let Some(pos) = self
                            .ollama_state
                            .paused_chats
                            .iter()
                            .position(|session| session.model == model_name)
                        {
                            self.ollama_state.paused_chats.remove(pos);
                        }
                        tokio::spawn(async move {
                            use crate::integrations::OllamaClient;
                            if let Ok(client) = OllamaClient::new(None) {
                                let _ = client.stop_model(&model_name).await;
                            }
                        });
                    }
                    return Ok(true);
                }
                KeyCode::Char('d') => {
                    if !is_initial_press {
                        return Ok(true);
                    }
                    if self.ollama_state.focused_panel == OllamaPanelFocus::Activity
                        && self.ollama_state.activity_view == OllamaActivityView::List
                    {
                        let entry = self.ollama_data.read().as_ref().and_then(|data| {
                            let idx = self
                                .ollama_state
                                .activity_selected
                                .min(data.chat_logs.len().saturating_sub(1));
                            data.chat_logs.get(idx).cloned()
                        });
                        if let Some(entry) = entry {
                            self.ollama_state.pending_delete =
                                Some(OllamaDeleteTarget::ChatLog(entry));
                            self.ollama_state.show_delete_confirm = true;
                        }
                        return Ok(true);
                    }
                    if self.ollama_state.current_view == OllamaView::Running {
                        return Ok(true);
                    }
                    let target_name = match self.ollama_state.current_view {
                        OllamaView::Models => self
                            .sorted_ollama_models()
                            .get(self.ollama_state.selected_model_index)
                            .map(|model| model.name.clone()),
                        OllamaView::Running => self
                            .sorted_ollama_running_models()
                            .get(self.ollama_state.selected_running_index)
                            .map(|model| model.name.clone()),
                    };
                    if let Some(name) = target_name {
                        self.ollama_state.pending_delete = Some(OllamaDeleteTarget::Model(name));
                        self.ollama_state.show_delete_confirm = true;
                    }
                    return Ok(true);
                }
                KeyCode::Char('p') => {
                    if !is_initial_press {
                        return Ok(true);
                    }
                    self.ollama_state.input_mode = OllamaInputMode::Pull;
                    self.ollama_state.input_buffer.clear();
                    self.ollama_state.focused_panel = OllamaPanelFocus::Input;
                    return Ok(true);
                }
                KeyCode::Char('c') => {
                    if !is_initial_press {
                        return Ok(true);
                    }
                    self.ollama_state.input_mode = OllamaInputMode::Command;
                    self.ollama_state.input_buffer.clear();
                    self.ollama_state.focused_panel = OllamaPanelFocus::Input;
                    return Ok(true);
                }
                KeyCode::Char('l') => {
                    return Ok(true);
                }
                _ => {}
            }
        }

        // Handle global hotkeys
        match key.code {
            KeyCode::F(2) => {
                self.compact_mode = !self.compact_mode;
                if self.compact_mode {
                    self.services_state.focused_panel = ServicesPanelFocus::Table;
                    self.services_state.details_scroll = 0;
                }
            }
            KeyCode::Tab if is_initial_press => {
                self.tab_manager.next();
            }
            KeyCode::BackTab if is_initial_press => {
                self.tab_manager.previous();
            }
            KeyCode::Char('1') => self.tab_manager.select(TabType::Cpu),
            KeyCode::Char('2') => self.tab_manager.select(TabType::Gpu),
            KeyCode::Char('3') => self.tab_manager.select(TabType::Ram),
            KeyCode::Char('4') => self.tab_manager.select(TabType::Disk),
            KeyCode::Char('5') => self.tab_manager.select(TabType::Network),
            KeyCode::Char('6') => self.tab_manager.select(TabType::Ollama),
            KeyCode::Char('7') => self.tab_manager.select(TabType::Processes),
            KeyCode::Char('8') => self.tab_manager.select(TabType::Services),
            KeyCode::Char('9') => self.tab_manager.select(TabType::DiskAnalyzer),
            KeyCode::Char('0') => self.tab_manager.select(TabType::Settings),
            _ => {}
        }

        Ok(true)
    }

    async fn handle_mouse_event(&mut self, mouse: MouseEvent) -> Result<bool> {
        match mouse.kind {
            MouseEventKind::Down(_) => {
                // Handle mouse clicks for radial menu
            }
            _ => {}
        }

        Ok(true)
    }

    #[allow(dead_code)]
    async fn execute_command(&mut self) -> Result<()> {
        if true {
            return Ok(());
        }

        // Add to history
        let command = String::new();
        // self.command_history.add(command.clone());

        // Execute PowerShell command
        let ps = PowerShellExecutor::new(
            self.config.read().powershell.executable.clone(),
            self.config.read().powershell.timeout_seconds,
            self.config.read().powershell.cache_ttl_seconds,
            self.config.read().powershell.use_cache,
        );

        tokio::spawn(async move {
            match ps.execute(&command).await {
                Ok(output) => {
                    log::info!("Command output: {}", output);
                }
                Err(e) => {
                    log::error!("Command failed: {}", e);
                }
            }
        });

        Ok(())
    }
}

fn is_valid_shell_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(ch) if ch == '_' || ch.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn shell_command_basename(command: &str) -> &str {
    command
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(command)
}

fn command_invocation_is_interactive(words: &[String]) -> bool {
    let Some(command) = words.first().map(|word| shell_command_basename(word)) else {
        return false;
    };

    const ALWAYS_INTERACTIVE: &[&str] = &[
        "vim", "nvim", "vi", "nano", "emacs", "micro", "joe", "pico", "less", "more", "most",
        "top", "htop", "btop", "atop", "glances", "nmon", "telnet", "ftp", "sftp", "tmux",
        "screen", "byobu", "mc", "ranger", "nnn", "lf", "vifm",
    ];

    if ALWAYS_INTERACTIVE.contains(&command) {
        return true;
    }

    match command {
        "sudo" => sudo_invocation_is_interactive(words),
        "su" => !words
            .iter()
            .skip(1)
            .any(|arg| arg == "-c" || arg == "--command"),
        "ssh" => ssh_invocation_is_interactive(words),
        "bash" | "zsh" | "fish" | "sh" | "csh" | "tcsh" | "ksh" => {
            shell_invocation_is_interactive(words)
        }
        "python" | "python3" | "ipython" | "node" | "irb" | "ghci" | "lua" => {
            repl_invocation_is_interactive(words)
        }
        "mysql" | "psql" | "sqlite3" | "mongo" | "redis-cli" => {
            database_cli_invocation_is_interactive(words)
        }
        "gdb" | "lldb" => !words.iter().skip(1).any(|arg| {
            matches!(
                arg.as_str(),
                "--batch" | "-batch" | "-ex" | "--eval-command"
            )
        }),
        _ => false,
    }
}

fn sudo_invocation_is_interactive(words: &[String]) -> bool {
    if words.len() == 1 {
        return true;
    }

    let mut command_index = 1;
    while command_index < words.len() {
        let arg = words[command_index].as_str();
        if matches!(arg, "-i" | "-s" | "-") {
            return true;
        }
        if arg == "--" {
            command_index += 1;
            break;
        }
        if matches!(arg, "-u" | "-g" | "-h" | "-p" | "-C" | "-T" | "-t") {
            command_index += 2;
            continue;
        }
        if arg.starts_with('-') || arg.contains('=') {
            command_index += 1;
            continue;
        }
        break;
    }

    if command_index >= words.len() {
        true
    } else {
        command_invocation_is_interactive(&words[command_index..])
    }
}

fn ssh_invocation_is_interactive(words: &[String]) -> bool {
    let mut seen_host = false;
    let mut index = 1;
    while index < words.len() {
        let arg = words[index].as_str();
        if arg == "--" {
            index += 1;
            break;
        }
        if matches!(
            arg,
            "-b" | "-c"
                | "-D"
                | "-E"
                | "-e"
                | "-F"
                | "-I"
                | "-i"
                | "-J"
                | "-L"
                | "-l"
                | "-m"
                | "-O"
                | "-o"
                | "-p"
                | "-Q"
                | "-R"
                | "-S"
                | "-W"
                | "-w"
        ) {
            index += 2;
            continue;
        }
        if arg.starts_with('-') {
            index += 1;
            continue;
        }
        if seen_host {
            return false;
        }
        seen_host = true;
        index += 1;
    }

    if index < words.len() {
        return false;
    }
    seen_host
}

fn shell_invocation_is_interactive(words: &[String]) -> bool {
    if words.len() == 1 {
        return true;
    }
    if has_help_or_version_flag(words) {
        return false;
    }
    if words
        .iter()
        .skip(1)
        .any(|arg| arg.starts_with('-') && arg.contains('i'))
    {
        return true;
    }
    if words
        .iter()
        .skip(1)
        .any(|arg| arg.starts_with('-') && arg.contains('c'))
    {
        return false;
    }
    !words.iter().skip(1).any(|arg| !arg.starts_with('-'))
}

fn repl_invocation_is_interactive(words: &[String]) -> bool {
    if words.len() == 1 {
        return true;
    }
    if has_help_or_version_flag(words) {
        return false;
    }
    if words
        .iter()
        .skip(1)
        .any(|arg| matches!(arg.as_str(), "-i" | "--interactive"))
    {
        return true;
    }
    if words
        .iter()
        .skip(1)
        .any(|arg| matches!(arg.as_str(), "-c" | "-e" | "--eval" | "-m"))
    {
        return false;
    }
    !words.iter().skip(1).any(|arg| !arg.starts_with('-'))
}

fn database_cli_invocation_is_interactive(words: &[String]) -> bool {
    if has_help_or_version_flag(words) {
        return false;
    }
    !words.iter().skip(1).any(|arg| {
        matches!(
            arg.as_str(),
            "-e" | "--execute" | "-c" | "--command" | "-f" | "--file"
        ) || arg.starts_with("-e")
            || arg.starts_with("--execute=")
            || arg.starts_with("--command=")
            || arg.starts_with("--file=")
    })
}

fn has_help_or_version_flag(words: &[String]) -> bool {
    words.iter().skip(1).any(|arg| {
        matches!(
            arg.as_str(),
            "-h" | "--help" | "-V" | "--version" | "-v" | "-?"
        )
    })
}

#[cfg(test)]
mod console_command_tests {
    use super::*;

    fn words(input: &str) -> Vec<String> {
        crate::app::console_state::split_shell_words(input).unwrap()
    }

    #[test]
    fn shell_identifier_validation_matches_posix_names() {
        assert!(is_valid_shell_identifier("_PATH"));
        assert!(is_valid_shell_identifier("PATH_1"));
        assert!(!is_valid_shell_identifier("1PATH"));
        assert!(!is_valid_shell_identifier("BAD-NAME"));
    }

    #[test]
    fn interactive_detection_allows_non_interactive_interpreters() {
        assert!(!command_invocation_is_interactive(&words(
            "python -c 'print(1)'"
        )));
        assert!(!command_invocation_is_interactive(&words(
            "python script.py"
        )));
        assert!(!command_invocation_is_interactive(&words("bash script.sh")));
        assert!(!command_invocation_is_interactive(&words(
            "bash -lc 'echo ok'"
        )));
        assert!(!command_invocation_is_interactive(&words(
            "node -e 'console.log(1)'"
        )));
    }

    #[test]
    fn interactive_detection_blocks_real_pty_workloads() {
        assert!(command_invocation_is_interactive(&words("vim file.txt")));
        assert!(command_invocation_is_interactive(&words("python")));
        assert!(command_invocation_is_interactive(&words("bash -i")));
        assert!(command_invocation_is_interactive(&words("sudo -i")));
        assert!(command_invocation_is_interactive(&words(
            "sudo vim /etc/hosts"
        )));
    }

    #[test]
    fn interactive_detection_allows_sudo_non_interactive_commands() {
        assert!(!command_invocation_is_interactive(&words("sudo ls /root")));
        assert!(!command_invocation_is_interactive(&words(
            "sudo python -c 'print(1)'"
        )));
    }
}

#[cfg(target_os = "linux")]
fn parse_network_scan_input(input: &str) -> (String, Vec<u16>) {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return (String::new(), default_port_profile());
    }

    let mut parts = trimmed.split_whitespace();
    let target = parts.next().unwrap_or_default().to_string();
    let rest = parts.collect::<Vec<_>>().join(",");

    let mut ports = if rest.is_empty() {
        default_port_profile()
    } else {
        rest.split([',', ';', ' '])
            .filter_map(|token| token.trim().parse::<u16>().ok())
            .filter(|port| *port > 0)
            .collect::<Vec<_>>()
    };

    ports.sort_unstable();
    ports.dedup();
    if ports.is_empty() {
        ports = default_port_profile();
    }
    if ports.len() > 64 {
        ports.truncate(64);
    }

    (target, ports)
}

#[cfg(target_os = "linux")]
fn parse_connection_lab_input(input: &str) -> (Option<String>, Option<String>, usize) {
    let mut protocol = None;
    let mut state = None;
    let mut limit = 160usize;

    for token in input.split([',', ';', ' ']).map(str::trim) {
        if token.is_empty() {
            continue;
        }
        let lower = token.to_ascii_lowercase();
        if let Some(value) = lower.strip_prefix("proto=") {
            if !value.is_empty() {
                protocol = Some(value.to_ascii_uppercase());
            }
            continue;
        }
        if let Some(value) = lower.strip_prefix("state=") {
            if !value.is_empty() {
                state = Some(value.to_ascii_uppercase());
            }
            continue;
        }
        if let Some(value) = lower.strip_prefix("limit=") {
            if let Ok(parsed) = value.parse::<usize>() {
                limit = parsed.clamp(10, 512);
            }
            continue;
        }
        if protocol.is_none() && matches!(lower.as_str(), "tcp" | "udp" | "tcp6" | "udp6") {
            protocol = Some(lower.to_ascii_uppercase());
            continue;
        }
        if state.is_none()
            && matches!(
                lower.as_str(),
                "estab"
                    | "established"
                    | "listen"
                    | "close-wait"
                    | "close_wait"
                    | "syn-sent"
                    | "syn_sent"
                    | "syn-recv"
                    | "syn_recv"
                    | "time-wait"
                    | "time_wait"
                    | "unconn"
            )
        {
            let canonical = match lower.as_str() {
                "estab" | "established" => "ESTAB",
                "close_wait" | "close-wait" => "CLOSE-WAIT",
                "syn_sent" | "syn-sent" => "SYN-SENT",
                "syn_recv" | "syn-recv" => "SYN-RECV",
                "time_wait" | "time-wait" => "TIME-WAIT",
                "unconn" => "UNCONN",
                other => other,
            };
            state = Some(canonical.to_ascii_uppercase());
            continue;
        }
        if limit == 160 {
            if let Ok(parsed) = token.parse::<usize>() {
                limit = parsed.clamp(10, 512);
            }
        }
    }

    (protocol, state, limit)
}

#[cfg(target_os = "linux")]
fn parse_ping_diag_input(input: &str) -> std::result::Result<linux_netdiag::PingRequest, String> {
    let mut request = linux_netdiag::PingRequest {
        target: String::new(),
        profile: linux_netdiag::PingProfile::Quick,
        continuous: false,
        count: 4,
        timeout_secs: 2,
        interval_ms: 250,
        deadline_secs: 12,
    };

    for token in input
        .split([' ', ',', ';'])
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        let lower = token.to_ascii_lowercase();
        if request.target.is_empty()
            && !lower.contains('=')
            && !matches!(
                lower.as_str(),
                "quick" | "latency" | "loss" | "continuous" | "cont" | "stream" | "once"
            )
        {
            request.target = token.to_string();
            continue;
        }

        if let Some(value) = lower.strip_prefix("profile=") {
            request.profile = match value {
                "quick" => linux_netdiag::PingProfile::Quick,
                "latency" => linux_netdiag::PingProfile::Latency,
                "loss" => linux_netdiag::PingProfile::Loss,
                _ => return Err(format!("Unknown ping profile `{value}`")),
            };
            continue;
        }
        if matches!(lower.as_str(), "quick" | "latency" | "loss") {
            request.profile = match lower.as_str() {
                "quick" => linux_netdiag::PingProfile::Quick,
                "latency" => linux_netdiag::PingProfile::Latency,
                _ => linux_netdiag::PingProfile::Loss,
            };
            continue;
        }
        if matches!(lower.as_str(), "continuous" | "cont" | "stream") {
            request.continuous = true;
            continue;
        }
        if lower == "once" {
            request.continuous = false;
            continue;
        }

        if let Some(value) = lower.strip_prefix("count=") {
            request.count = value
                .parse::<u32>()
                .map_err(|_| format!("Invalid ping count `{value}`"))?
                .clamp(1, 200);
            continue;
        }
        if let Some(value) = lower.strip_prefix("timeout=") {
            request.timeout_secs = value
                .parse::<u32>()
                .map_err(|_| format!("Invalid ping timeout `{value}`"))?
                .clamp(1, 30);
            continue;
        }
        if let Some(value) = lower
            .strip_prefix("interval_ms=")
            .or_else(|| lower.strip_prefix("interval="))
        {
            request.interval_ms = value
                .parse::<u32>()
                .map_err(|_| format!("Invalid ping interval `{value}`"))?
                .clamp(200, 5000);
            continue;
        }
        if let Some(value) = lower
            .strip_prefix("deadline=")
            .or_else(|| lower.strip_prefix("window="))
        {
            request.deadline_secs = value
                .parse::<u32>()
                .map_err(|_| format!("Invalid ping deadline `{value}`"))?
                .clamp(2, 900);
            continue;
        }
    }

    if request.target.trim().is_empty() {
        return Err("Ping target is empty".to_string());
    }
    Ok(request)
}

#[cfg(target_os = "linux")]
fn parse_trace_diag_input(input: &str) -> std::result::Result<linux_netdiag::TraceRequest, String> {
    let mut request = linux_netdiag::TraceRequest {
        target: String::new(),
        protocol: linux_netdiag::TraceProtocol::Icmp,
        enable_fallback: true,
        max_hops: 20,
        timeout_secs: 2,
        per_hop_queries: 1,
        port: None,
        resolve_names: false,
    };

    for token in input
        .split([' ', ',', ';'])
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        let lower = token.to_ascii_lowercase();
        if request.target.is_empty()
            && !lower.contains('=')
            && !matches!(
                lower.as_str(),
                "icmp" | "udp" | "tcp" | "fallback" | "nofallback" | "resolve" | "names"
            )
        {
            request.target = token.to_string();
            continue;
        }

        if let Some(value) = lower
            .strip_prefix("proto=")
            .or_else(|| lower.strip_prefix("protocol="))
        {
            request.protocol = match value {
                "icmp" => linux_netdiag::TraceProtocol::Icmp,
                "udp" => linux_netdiag::TraceProtocol::Udp,
                "tcp" => linux_netdiag::TraceProtocol::Tcp,
                _ => return Err(format!("Unknown trace protocol `{value}`")),
            };
            continue;
        }
        if matches!(lower.as_str(), "icmp" | "udp" | "tcp") {
            request.protocol = match lower.as_str() {
                "icmp" => linux_netdiag::TraceProtocol::Icmp,
                "udp" => linux_netdiag::TraceProtocol::Udp,
                _ => linux_netdiag::TraceProtocol::Tcp,
            };
            continue;
        }
        if lower == "fallback" {
            request.enable_fallback = true;
            continue;
        }
        if lower == "nofallback" {
            request.enable_fallback = false;
            continue;
        }
        if let Some(value) = lower.strip_prefix("fallback=") {
            request.enable_fallback =
                parse_bool_flag(value).ok_or_else(|| format!("Invalid fallback flag `{value}`"))?;
            continue;
        }
        if lower == "resolve" || lower == "names" {
            request.resolve_names = true;
            continue;
        }
        if let Some(value) = lower.strip_prefix("resolve=") {
            request.resolve_names =
                parse_bool_flag(value).ok_or_else(|| format!("Invalid resolve flag `{value}`"))?;
            continue;
        }
        if let Some(value) = lower
            .strip_prefix("hops=")
            .or_else(|| lower.strip_prefix("max_hops="))
            .or_else(|| lower.strip_prefix("maxhop="))
        {
            request.max_hops = value
                .parse::<u8>()
                .map_err(|_| format!("Invalid max hops `{value}`"))?
                .clamp(1, 64);
            continue;
        }
        if let Some(value) = lower.strip_prefix("timeout=") {
            request.timeout_secs = value
                .parse::<u8>()
                .map_err(|_| format!("Invalid trace timeout `{value}`"))?
                .clamp(1, 10);
            continue;
        }
        if let Some(value) = lower
            .strip_prefix("q=")
            .or_else(|| lower.strip_prefix("queries="))
            .or_else(|| lower.strip_prefix("probes="))
        {
            request.per_hop_queries = value
                .parse::<u8>()
                .map_err(|_| format!("Invalid query count `{value}`"))?
                .clamp(1, 5);
            continue;
        }
        if let Some(value) = lower
            .strip_prefix("port=")
            .or_else(|| lower.strip_prefix("p="))
        {
            let port = value
                .parse::<u16>()
                .map_err(|_| format!("Invalid trace port `{value}`"))?;
            if port == 0 {
                return Err("Trace port must be > 0".to_string());
            }
            request.port = Some(port);
            continue;
        }
    }

    if request.target.trim().is_empty() {
        return Err("Trace target is empty".to_string());
    }
    Ok(request)
}

#[cfg(target_os = "linux")]
fn parse_bool_flag(value: &str) -> Option<bool> {
    match value {
        "1" | "true" | "yes" | "on" | "enabled" => Some(true),
        "0" | "false" | "no" | "off" | "disabled" => Some(false),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
fn parse_nat_mapping_input(
    input: &str,
) -> std::result::Result<(linux_netdiag::MappingProtocol, u16, u16, u32), String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok((linux_netdiag::MappingProtocol::Tcp, 8080, 8080, 120));
    }

    let mut tokens = trimmed
        .split([',', ';', ' '])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return Ok((linux_netdiag::MappingProtocol::Tcp, 8080, 8080, 120));
    }

    let protocol = match tokens.first().map(|value| value.to_ascii_lowercase()) {
        Some(value) if value == "tcp" => {
            tokens.remove(0);
            linux_netdiag::MappingProtocol::Tcp
        }
        Some(value) if value == "udp" => {
            tokens.remove(0);
            linux_netdiag::MappingProtocol::Udp
        }
        _ => linux_netdiag::MappingProtocol::Tcp,
    };

    let read_port = |idx: usize, default: u16| -> std::result::Result<u16, String> {
        if let Some(raw) = tokens.get(idx) {
            let parsed = raw
                .parse::<u16>()
                .map_err(|_| format!("Invalid port value `{raw}`"))?;
            if parsed == 0 {
                return Err(format!("Port must be > 0 (got `{raw}`)"));
            }
            Ok(parsed)
        } else {
            Ok(default)
        }
    };
    let internal_port = read_port(0, 8080)?;
    let external_port = read_port(1, internal_port)?;
    let ttl_seconds = if let Some(raw) = tokens.get(2) {
        raw.parse::<u32>()
            .map_err(|_| format!("Invalid TTL value `{raw}`"))?
            .clamp(30, 3600)
    } else {
        120
    };

    Ok((protocol, internal_port, external_port, ttl_seconds))
}

#[cfg(target_os = "linux")]
fn default_port_profile() -> Vec<u16> {
    vec![22, 53, 80, 123, 443, 587, 993, 3389]
}

#[cfg(target_os = "linux")]
fn network_operation_label(op: linux_netdiag::DiagnosticsOperation) -> &'static str {
    match op {
        linux_netdiag::DiagnosticsOperation::Resolve => "Resolve",
        linux_netdiag::DiagnosticsOperation::DnsExplain => "DNS Explain",
        linux_netdiag::DiagnosticsOperation::RouteInspect => "Route Inspect",
        linux_netdiag::DiagnosticsOperation::NicDeepInfo => "NIC Deep Info",
        linux_netdiag::DiagnosticsOperation::ConnectionLab => "Connection Lab",
        linux_netdiag::DiagnosticsOperation::Ping => "Ping",
        linux_netdiag::DiagnosticsOperation::Trace => "Trace",
        linux_netdiag::DiagnosticsOperation::MtuProbe => "MTU Probe",
        linux_netdiag::DiagnosticsOperation::PortScan => "Port Scan",
        linux_netdiag::DiagnosticsOperation::NatCapabilityCheck => "NAT Capability",
        linux_netdiag::DiagnosticsOperation::MappingTest => "Mapping Test",
        linux_netdiag::DiagnosticsOperation::ExportReport => "Export Report",
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_network_parser_tests {
    use super::*;

    #[test]
    fn parse_connection_lab_input_key_value_mode() {
        let (proto, state, limit) = parse_connection_lab_input("proto=tcp state=estab limit=200");
        assert_eq!(proto.as_deref(), Some("TCP"));
        assert_eq!(state.as_deref(), Some("ESTAB"));
        assert_eq!(limit, 200);
    }

    #[test]
    fn parse_nat_mapping_input_defaults_and_override() {
        let defaults = parse_nat_mapping_input("").expect("defaults");
        assert_eq!(defaults.1, 8080);
        assert_eq!(defaults.2, 8080);
        assert_eq!(defaults.3, 120);

        let custom = parse_nat_mapping_input("udp 5353 55353 600").expect("custom");
        assert!(matches!(custom.0, linux_netdiag::MappingProtocol::Udp));
        assert_eq!(custom.1, 5353);
        assert_eq!(custom.2, 55353);
        assert_eq!(custom.3, 600);
    }

    #[test]
    fn parse_ping_diag_input_reads_profile_and_overrides() {
        let req = parse_ping_diag_input(
            "1.1.1.1 profile=latency count=12 timeout=3 interval_ms=300 continuous deadline=25",
        )
        .expect("ping req");
        assert_eq!(req.target, "1.1.1.1");
        assert!(matches!(req.profile, linux_netdiag::PingProfile::Latency));
        assert!(req.continuous);
        assert_eq!(req.count, 12);
        assert_eq!(req.timeout_secs, 3);
        assert_eq!(req.interval_ms, 300);
        assert_eq!(req.deadline_secs, 25);
    }

    #[test]
    fn parse_trace_diag_input_reads_protocol_and_fallback() {
        let req = parse_trace_diag_input(
            "example.org proto=tcp fallback=off hops=30 timeout=3 q=2 port=443 resolve=on",
        )
        .expect("trace req");
        assert_eq!(req.target, "example.org");
        assert!(matches!(req.protocol, linux_netdiag::TraceProtocol::Tcp));
        assert!(!req.enable_fallback);
        assert_eq!(req.max_hops, 30);
        assert_eq!(req.timeout_secs, 3);
        assert_eq!(req.per_hop_queries, 2);
        assert_eq!(req.port, Some(443));
        assert!(req.resolve_names);
    }
}

#[cfg(target_os = "linux")]
fn network_result_detail_lines(result: &linux_netdiag::NetworkDiagnosticsResult) -> Vec<String> {
    let mut lines = Vec::new();
    match result {
        linux_netdiag::NetworkDiagnosticsResult::Resolve(r) => {
            lines.push(format!("query: {}", r.query));
            lines.push(format!("host: {}", r.host));
            if r.addresses.is_empty() {
                lines.push("addresses: none returned".to_string());
            } else {
                let ipv4: Vec<_> = r.addresses.iter().filter(|a| !a.contains(':')).collect();
                let ipv6: Vec<_> = r.addresses.iter().filter(|a| a.contains(':')).collect();
                lines.push(format!(
                    "total: {} addresses ({} IPv4, {} IPv6)",
                    r.addresses.len(),
                    ipv4.len(),
                    ipv6.len()
                ));
                if !ipv4.is_empty() {
                    lines.push("IPv4:".to_string());
                    for (i, addr) in ipv4.iter().enumerate() {
                        lines.push(format!("  [{}] {}", i + 1, addr));
                    }
                }
                if !ipv6.is_empty() {
                    lines.push("IPv6:".to_string());
                    for (i, addr) in ipv6.iter().enumerate() {
                        lines.push(format!("  [{}] {}", i + 1, addr));
                    }
                }
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::DnsExplain(r) => {
            lines.push(format!("resolver mode: {}", r.resolver_mode));
            lines.push(format!("resolv.conf: {}", r.resolv_conf_path));
            if let Some(mode) = &r.network_manager_dns_mode {
                lines.push(format!("NM dns mode: {mode}"));
            }
            if !r.dns_servers.is_empty() {
                lines.push(String::new());
                lines.push(format!("dns servers: {} configured", r.dns_servers.len()));
                for entry in &r.dns_servers {
                    lines.push(format!("  {} (source: {})", entry.address, entry.source));
                }
            }
            if !r.search_domains.is_empty() {
                lines.push(format!("search domains: {}", r.search_domains.join(", ")));
            }
            if !r.split_dns_domains.is_empty() {
                lines.push(String::new());
                lines.push(format!("split DNS: {} domains", r.split_dns_domains.len()));
                for d in &r.split_dns_domains {
                    lines.push(format!("  {d}"));
                }
            }
            if !r.default_gateways.is_empty() {
                lines.push(String::new());
                lines.push(format!("gateways: {} found", r.default_gateways.len()));
                for gw in &r.default_gateways {
                    let port_str = gw.port.map(|p| format!(":{p}")).unwrap_or_default();
                    let metric_str = gw
                        .metric
                        .map(|m| format!(" metric={m}"))
                        .unwrap_or_default();
                    lines.push(format!(
                        "  {} {}{}{}",
                        gw.interface, gw.address, port_str, metric_str
                    ));
                }
            }
            if !r.conflicts.is_empty() {
                lines.push(String::new());
                lines.push(format!("CONFLICTS: {} found", r.conflicts.len()));
                for c in &r.conflicts {
                    lines.push(format!("  {c}"));
                }
            }
            if !r.warnings.is_empty() {
                for w in &r.warnings {
                    lines.push(format!("warning: {w}"));
                }
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::RouteInspect(r) => {
            if let Some(egress) = &r.egress {
                lines.push(format!("egress: {}", egress.output));
            }
            lines.push(format!("default routes: {}", r.default_routes.len()));
            lines.push(format!("policy rules: {}", r.policy_rules.len()));
            lines.push(String::new());
            for (i, route) in r.default_routes.iter().enumerate() {
                let gw = route
                    .gateway
                    .clone()
                    .unwrap_or_else(|| "direct".to_string());
                let dev = route.interface.clone().unwrap_or_else(|| "n/a".to_string());
                let metric = route
                    .metric
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "n/a".to_string());
                let proto = route.protocol.clone().unwrap_or_else(|| "-".to_string());
                let scope = route.scope.clone().unwrap_or_else(|| "-".to_string());
                lines.push(format!(
                    "  route[{}]: {} via {} dev {} metric={} proto={} scope={}",
                    i, route.family, gw, dev, metric, proto, scope,
                ));
            }
            if !r.policy_rules.is_empty() {
                lines.push(String::new());
                lines.push("Policy rules:".to_string());
                for rule in &r.policy_rules {
                    let prio = rule
                        .priority
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| "?".to_string());
                    let tbl = rule.table.clone().unwrap_or_else(|| "?".to_string());
                    let act = rule.action.clone().unwrap_or_else(|| "lookup".to_string());
                    lines.push(format!(
                        "  prio={} {} table={} action={}",
                        prio, rule.family, tbl, act
                    ));
                }
            }
            if !r.warnings.is_empty() {
                lines.push(String::new());
                for w in &r.warnings {
                    lines.push(format!("warning: {w}"));
                }
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::NicDeepInfo(r) => {
            lines.push(format!("interfaces: {}", r.interfaces.len()));
            lines.push(String::new());
            for iface in &r.interfaces {
                lines.push(format!("{}: {}", iface.interface, iface.status));
                lines.push(format!(
                    "  speed:   {}",
                    iface.speed.clone().unwrap_or_else(|| "n/a".to_string())
                ));
                lines.push(format!(
                    "  duplex:  {}",
                    iface.duplex.clone().unwrap_or_else(|| "n/a".to_string())
                ));
                lines.push(format!("  mtu:     {}", iface.mtu));
                lines.push(format!(
                    "  driver:  {}",
                    iface.driver.clone().unwrap_or_else(|| "n/a".to_string())
                ));
                lines.push(format!("  mac:     {}", iface.mac_address));
                if let Some(fw) = &iface.firmware {
                    lines.push(format!("  firmware: {fw}"));
                }
                if let Some(bus) = &iface.bus_info {
                    lines.push(format!("  bus:     {bus}"));
                }
                // Error counters
                let rx_err = iface.rx_errors.unwrap_or(0);
                let tx_err = iface.tx_errors.unwrap_or(0);
                let rx_drop = iface.rx_dropped.unwrap_or(0);
                let tx_drop = iface.tx_dropped.unwrap_or(0);
                if rx_err > 0 || tx_err > 0 || rx_drop > 0 || tx_drop > 0 {
                    lines.push(format!("  rx errors:  {}", rx_err));
                    lines.push(format!("  tx errors:  {}", tx_err));
                    lines.push(format!("  rx dropped: {}", rx_drop));
                    lines.push(format!("  tx dropped: {}", tx_drop));
                } else {
                    lines.push("  errors: none".to_string());
                }
                // Offloads
                if !iface.offloads.is_empty() {
                    let on: Vec<_> = iface
                        .offloads
                        .iter()
                        .filter(|o| o.enabled)
                        .map(|o| o.name.as_str())
                        .collect();
                    let off: Vec<_> = iface
                        .offloads
                        .iter()
                        .filter(|o| !o.enabled)
                        .map(|o| o.name.as_str())
                        .collect();
                    if !on.is_empty() {
                        lines.push(format!("  offload ON: {}", on.join(", ")));
                    }
                    if !off.is_empty() {
                        lines.push(format!("  offload OFF: {}", off.join(", ")));
                    }
                }
                // Wi-Fi info
                if let Some(wifi) = &iface.wifi {
                    lines.push("  Wi-Fi link:".to_string());
                    if let Some(ssid) = &wifi.ssid {
                        lines.push(format!("    SSID: {ssid}"));
                    }
                    if let Some(freq) = wifi.frequency_mhz {
                        let band = if freq < 3000 {
                            "2.4GHz"
                        } else if freq < 6000 {
                            "5GHz"
                        } else {
                            "6GHz"
                        };
                        lines.push(format!("    freq: {} MHz ({})", freq, band));
                    }
                    if let Some(signal) = wifi.signal_dbm {
                        let quality = if signal > -50.0 {
                            "Excellent"
                        } else if signal > -60.0 {
                            "Good"
                        } else if signal > -70.0 {
                            "Fair"
                        } else {
                            "Weak"
                        };
                        lines.push(format!("    signal: {:.0} dBm ({})", signal, quality));
                    }
                    if let Some(tx_rate) = &wifi.tx_bitrate {
                        lines.push(format!("    tx rate: {tx_rate}"));
                    }
                }
                // Notes
                if !iface.notes.is_empty() {
                    for note in &iface.notes {
                        lines.push(format!("  note: {note}"));
                    }
                }
                lines.push(String::new());
            }
            if !r.warnings.is_empty() {
                for w in &r.warnings {
                    lines.push(format!("warning: {w}"));
                }
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::ConnectionLab(r) => {
            let established = r
                .entries
                .iter()
                .filter(|e| e.state.eq_ignore_ascii_case("ESTAB"))
                .count();
            let listen = r
                .entries
                .iter()
                .filter(|e| e.state.eq_ignore_ascii_case("LISTEN"))
                .count();
            let time_wait = r
                .entries
                .iter()
                .filter(|e| e.state.eq_ignore_ascii_case("TIME-WAIT"))
                .count();
            let close_wait = r
                .entries
                .iter()
                .filter(|e| e.state.eq_ignore_ascii_case("CLOSE-WAIT"))
                .count();
            lines.push(format!("total entries: {}", r.entries.len()));
            lines.push(format!("established: {}", established));
            lines.push(format!("listening: {}", listen));
            lines.push(format!("time-wait: {}", time_wait));
            lines.push(format!("close-wait: {}", close_wait));
            lines.push(format!("permission limited: {}", r.permission_limited));
            lines.push(String::new());
            for entry in &r.entries {
                let proc_name = entry
                    .process_name
                    .clone()
                    .unwrap_or_else(|| "?".to_string());
                let pid = entry
                    .pid
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "-".to_string());
                let mut extra = String::new();
                if entry.recv_q > 0 || entry.send_q > 0 {
                    extra.push_str(&format!(" rq={} sq={}", entry.recv_q, entry.send_q));
                }
                if let Some(retx) = entry.retransmits {
                    if retx > 0 {
                        extra.push_str(&format!(" retx={}", retx));
                    }
                }
                if let (Some(tx), Some(rx)) = (entry.bytes_sent, entry.bytes_received) {
                    if tx > 0 || rx > 0 {
                        extra.push_str(&format!(
                            " tx={} rx={}",
                            fmt_bytes_short(tx),
                            fmt_bytes_short(rx)
                        ));
                    }
                }
                lines.push(format!(
                    "{} [{}] pid={} {}:{} -> {}:{} state={}{}",
                    proc_name,
                    entry.protocol,
                    pid,
                    entry.local_address,
                    entry.local_port,
                    entry.remote_address,
                    entry.remote_port,
                    entry.state,
                    extra,
                ));
                if !entry.notes.is_empty() {
                    for note in &entry.notes {
                        lines.push(format!("    note: {note}"));
                    }
                }
            }
            if !r.warnings.is_empty() {
                for w in &r.warnings {
                    lines.push(format!("warning: {w}"));
                }
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::Ping(r) => {
            lines.push(format!("target: {}", r.target));
            lines.push(format!("profile: {:?}", r.profile));
            lines.push(format!(
                "mode: {}",
                if r.continuous {
                    "continuous"
                } else {
                    "counted"
                }
            ));
            lines.push(String::new());
            lines.push(format!("transmitted: {}", r.transmitted));
            lines.push(format!("received: {}", r.received));
            lines.push(format!("loss: {:.1}%", r.packet_loss_percent));
            lines.push(format!("samples: {}", r.samples_collected));
            lines.push(String::new());
            lines.push("Latency distribution:".to_string());
            lines.push(format!("  min: {:>10} ms", fmt_opt_ms(r.min_latency_ms)));
            lines.push(format!("  avg: {:>10} ms", fmt_opt_ms(r.avg_latency_ms)));
            lines.push(format!("  p50: {:>10} ms", fmt_opt_ms(r.p50_latency_ms)));
            lines.push(format!("  p95: {:>10} ms", fmt_opt_ms(r.p95_latency_ms)));
            lines.push(format!("  p99: {:>10} ms", fmt_opt_ms(r.p99_latency_ms)));
            lines.push(format!("  max: {:>10} ms", fmt_opt_ms(r.max_latency_ms)));
            lines.push(String::new());
            lines.push(format!("  jitter: {:>7} ms", fmt_opt_ms(r.jitter_ms)));
            // Latency spread indicator
            if let (Some(min), Some(max)) = (r.min_latency_ms, r.max_latency_ms) {
                let spread = max - min;
                let stability = if spread < 5.0 {
                    "Very stable"
                } else if spread < 20.0 {
                    "Stable"
                } else if spread < 50.0 {
                    "Moderate variance"
                } else {
                    "High variance"
                };
                lines.push(format!("  spread: {:.2} ms ({})", spread, stability));
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::Trace(r) => {
            lines.push(format!("target: {}", r.target));
            lines.push(format!("protocol requested: {:?}", r.requested_protocol));
            lines.push(format!("protocol used: {:?}", r.used_protocol));
            if r.fallback_used {
                lines.push("fallback used: true (original protocol failed)".to_string());
            }
            lines.push(format!("reached target: {}", r.reached_target));
            lines.push(format!("total hops: {}", r.hops.len()));
            lines.push(format!("timeout ratio: {:.1}%", r.timeout_ratio * 100.0));
            lines.push(String::new());
            // Attempts detail
            if r.attempts.len() > 1 || r.fallback_used {
                lines.push("Attempts:".to_string());
                for attempt in &r.attempts {
                    lines.push(format!(
                        "  {:?}: hops={} timeouts={} reached={}{}",
                        attempt.protocol,
                        attempt.hops_collected,
                        attempt.timeout_hops,
                        attempt.reached_target,
                        attempt
                            .warning
                            .as_ref()
                            .map(|w| format!("  WARN: {w}"))
                            .unwrap_or_default()
                    ));
                }
                lines.push(String::new());
            }
            // Hop table
            lines.push(format!(
                "{:<6} {:<36} {:>10} {:>8} {:>8} {:>8} {}",
                "Hop", "Endpoint", "Avg RTT", "Min", "Max", "Probes", "Flags"
            ));
            lines.push(format!("{}", "\u{2500}".repeat(86)));
            for hop in &r.hops {
                let endpoint = hop
                    .host
                    .as_ref()
                    .or(hop.address.as_ref())
                    .cloned()
                    .unwrap_or_else(|| "*".to_string());
                let (rtt_avg, rtt_min, rtt_max) = if hop.rtt_ms.is_empty() {
                    ("*".to_string(), "*".to_string(), "*".to_string())
                } else {
                    let avg = hop.rtt_ms.iter().sum::<f32>() / hop.rtt_ms.len() as f32;
                    let min = hop.rtt_ms.iter().cloned().fold(f32::MAX, f32::min);
                    let max = hop.rtt_ms.iter().cloned().fold(f32::MIN, f32::max);
                    (
                        format!("{:.1}ms", avg),
                        format!("{:.1}", min),
                        format!("{:.1}", max),
                    )
                };
                let flags = format!(
                    "{}{}",
                    if hop.timed_out { " !T" } else { "" },
                    if hop.blocked_suspected { " !B" } else { "" },
                );
                lines.push(format!(
                    "{:<6} {:<36} {:>10} {:>8} {:>8} {:>5}/{:<2} {}",
                    format!("{:02}", hop.hop),
                    if endpoint.len() > 35 {
                        format!("{}...", &endpoint[..32])
                    } else {
                        endpoint
                    },
                    rtt_avg,
                    rtt_min,
                    rtt_max,
                    hop.probes_responded,
                    hop.probes_sent,
                    flags,
                ));
            }
            if !r.blocked_indicators.is_empty() {
                lines.push(String::new());
                lines.push(format!(
                    "blocked indicators: {}",
                    r.blocked_indicators.join(" | ")
                ));
            }
            if !r.warnings.is_empty() {
                for w in &r.warnings {
                    lines.push(format!("warning: {w}"));
                }
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::MtuProbe(r) => {
            lines.push(format!("target: {}", r.target));
            match r.path_mtu {
                Some(pmtu) => {
                    lines.push(format!("path MTU: {} bytes", pmtu));
                    let overhead_note = if pmtu == 1500 {
                        " (standard Ethernet)"
                    } else if pmtu > 1500 {
                        " (jumbo frames)"
                    } else if pmtu >= 1400 {
                        " (minor overhead, likely VPN/tunnel)"
                    } else {
                        " (significant overhead)"
                    };
                    lines.push(format!("  {}", overhead_note.trim()));
                }
                None => lines.push("path MTU: could not determine".to_string()),
            }
            if !r.interfaces.is_empty() {
                lines.push(String::new());
                lines.push("Interface MTU comparison:".to_string());
                for iface_mtu in &r.interfaces {
                    let mtu_note = if let Some(pmtu) = r.path_mtu {
                        if iface_mtu.mtu < pmtu {
                            " (BOTTLENECK)"
                        } else if iface_mtu.mtu == pmtu {
                            " (matches path)"
                        } else {
                            ""
                        }
                    } else {
                        ""
                    };
                    lines.push(format!(
                        "  {} ({}) ipv4={} mtu={}{}",
                        iface_mtu.interface,
                        iface_mtu.status,
                        iface_mtu.ipv4,
                        iface_mtu.mtu,
                        mtu_note,
                    ));
                }
            }
            if let Some(warning) = &r.warning {
                lines.push(format!("warning: {warning}"));
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::PortScan(r) => {
            lines.push(format!("target: {}", r.target));
            lines.push(format!("ports scanned: {}", r.scanned_ports.len()));
            lines.push(format!(
                "duration: {} ms ({:.1}s)",
                r.duration_ms,
                r.duration_ms as f64 / 1000.0
            ));
            lines.push(format!(
                "scan rate: {:.0} ports/sec",
                r.scanned_ports.len() as f64 / (r.duration_ms as f64 / 1000.0).max(0.001)
            ));
            lines.push(String::new());
            if !r.open_ports.is_empty() {
                lines.push(format!("OPEN: {} ports", r.open_ports.len()));
                for port in &r.open_ports {
                    let svc = well_known_port_service(*port);
                    if svc.is_empty() {
                        lines.push(format!("  {:>5}  open", port));
                    } else {
                        lines.push(format!("  {:>5}  open  {}", port, svc));
                    }
                }
            } else {
                lines.push("OPEN: none".to_string());
            }
            let closed: Vec<u16> = r
                .scanned_ports
                .iter()
                .filter(|p| !r.open_ports.contains(p))
                .copied()
                .collect();
            if !closed.is_empty() {
                lines.push(String::new());
                lines.push(format!("CLOSED/FILTERED: {} ports", closed.len()));
                for port in &closed {
                    let svc = well_known_port_service(*port);
                    if svc.is_empty() {
                        lines.push(format!("  {:>5}  closed", port));
                    } else {
                        lines.push(format!("  {:>5}  closed  {}", port, svc));
                    }
                }
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::NatCapabilityCheck(r) => {
            if let Some(ext_ip) = &r.external_ip {
                lines.push(format!("external IP: {}", ext_ip));
            }
            lines.push(String::new());
            lines.push("NAT traversal methods:".to_string());
            for m in &r.methods {
                let (state_label, state_icon) = match m.state {
                    linux_netdiag::CapabilityState::Supported => ("Supported", "+"),
                    linux_netdiag::CapabilityState::Unavailable => ("Unavailable", "-"),
                    linux_netdiag::CapabilityState::PermissionDenied => ("PermDenied", "!"),
                    linux_netdiag::CapabilityState::MissingDependency => ("MissingDep", "?"),
                    linux_netdiag::CapabilityState::Unknown => ("Unknown", "~"),
                };
                lines.push(format!(
                    "  [{}] {:<16} {}",
                    state_icon, m.method, state_label
                ));
                if !m.details.is_empty() {
                    lines.push(format!("      {}", m.details));
                }
            }
            if !r.warnings.is_empty() {
                lines.push(String::new());
                for w in &r.warnings {
                    lines.push(format!("warning: {w}"));
                }
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::MappingTest(r) => {
            lines.push(format!(
                "protocol: {}",
                match r.protocol {
                    linux_netdiag::MappingProtocol::Tcp => "TCP",
                    linux_netdiag::MappingProtocol::Udp => "UDP",
                }
            ));
            if let Some(local) = &r.local_address {
                lines.push(format!("local address: {}", local));
            }
            lines.push(format!("external port: {}", r.external_port));
            lines.push(format!("internal port: {}", r.internal_port));
            lines.push(String::new());
            lines.push(format!("created: {}", r.created));
            lines.push(format!("visible in gw: {}", r.visible_in_gateway_table));
            lines.push(format!("removed: {}", r.removed));
            if !r.details.is_empty() {
                lines.push(String::new());
                lines.push("Details:".to_string());
                for d in &r.details {
                    lines.push(format!("  {d}"));
                }
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::ExportReport(r) => {
            lines.push(format!("format: {:?}", r.format));
            lines.push(format!("report entries: {}", r.entries));
            lines.push("Report has been exported successfully.".to_string());
        }
    }
    lines
}

/// Format bytes in short human-readable form for connection details
#[cfg(target_os = "linux")]
fn fmt_bytes_short(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1}G", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1}M", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{}B", bytes)
    }
}

/// Map well-known ports to service names
#[cfg(target_os = "linux")]
fn well_known_port_service(port: u16) -> &'static str {
    match port {
        20 => "(FTP data)",
        21 => "(FTP)",
        22 => "(SSH)",
        23 => "(Telnet)",
        25 => "(SMTP)",
        53 => "(DNS)",
        80 => "(HTTP)",
        110 => "(POP3)",
        143 => "(IMAP)",
        443 => "(HTTPS)",
        465 => "(SMTPS)",
        587 => "(SMTP/submission)",
        993 => "(IMAPS)",
        995 => "(POP3S)",
        3306 => "(MySQL)",
        3389 => "(RDP)",
        5432 => "(PostgreSQL)",
        5900 => "(VNC)",
        6379 => "(Redis)",
        8080 => "(HTTP alt)",
        8443 => "(HTTPS alt)",
        27017 => "(MongoDB)",
        _ => "",
    }
}

#[cfg(target_os = "linux")]
fn network_result_log_lines(result: &linux_netdiag::NetworkDiagnosticsResult) -> Vec<String> {
    let mut lines = network_result_detail_lines(result);
    lines.truncate(6);
    lines
}

/// Extract raw stdout/stderr-style lines from a diagnostics result
#[cfg(target_os = "linux")]
fn network_result_raw_lines(
    result: &linux_netdiag::NetworkDiagnosticsResult,
) -> (Vec<String>, Vec<String>) {
    let stdout = network_result_detail_lines(result);
    let mut stderr: Vec<String> = Vec::new();
    match result {
        linux_netdiag::NetworkDiagnosticsResult::DnsExplain(r) => {
            for w in &r.warnings {
                stderr.push(format!("WARN: {w}"));
            }
            for c in &r.conflicts {
                stderr.push(format!("CONFLICT: {c}"));
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::RouteInspect(r) => {
            for w in &r.warnings {
                stderr.push(format!("WARN: {w}"));
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::NicDeepInfo(r) => {
            for w in &r.warnings {
                stderr.push(format!("WARN: {w}"));
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::ConnectionLab(r) => {
            for w in &r.warnings {
                stderr.push(format!("WARN: {w}"));
            }
            if r.permission_limited {
                stderr.push(
                    "NOTE: Results limited by permissions — run with sudo for full info"
                        .to_string(),
                );
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::Trace(r) => {
            for w in &r.warnings {
                stderr.push(format!("WARN: {w}"));
            }
            for b in &r.blocked_indicators {
                stderr.push(format!("BLOCKED: {b}"));
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::Ping(r) => {
            if r.packet_loss_percent > 0.0 {
                stderr.push(format!(
                    "LOSS: {:.1}% packet loss detected",
                    r.packet_loss_percent
                ));
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::MtuProbe(r) => {
            if let Some(w) = &r.warning {
                stderr.push(format!("WARN: {w}"));
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::NatCapabilityCheck(r) => {
            for w in &r.warnings {
                stderr.push(format!("WARN: {w}"));
            }
        }
        _ => {}
    }
    (stdout, stderr)
}

/// Generate advice lines based on diagnostics result
#[cfg(target_os = "linux")]
fn network_result_advice_lines(result: &linux_netdiag::NetworkDiagnosticsResult) -> Vec<String> {
    let mut advice = Vec::new();
    match result {
        linux_netdiag::NetworkDiagnosticsResult::Ping(r) => {
            if r.packet_loss_percent > 0.0 && r.packet_loss_percent < 5.0 {
                advice.push("Minor packet loss detected.".to_string());
                advice.push("Recommended: retry with profile=loss deadline=30".to_string());
            } else if r.packet_loss_percent >= 5.0 {
                advice.push("Significant packet loss!".to_string());
                advice.push("1) Check physical connection".to_string());
                advice.push("2) Run trace+ to find the lossy hop".to_string());
                advice.push("3) Run MTU probe to check PMTU black-hole".to_string());
            } else {
                advice.push("Connection is stable with no packet loss.".to_string());
            }
            if let Some(jitter) = r.jitter_ms {
                if jitter > 10.0 {
                    advice.push(format!("High jitter ({jitter:.1}ms) detected."));
                    advice.push("Consider checking for network congestion.".to_string());
                }
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::Trace(r) => {
            if !r.reached_target {
                advice.push("Target not reached.".to_string());
                if r.fallback_used {
                    advice.push("Fallback protocol was used.".to_string());
                }
                if r.timeout_ratio > 0.5 {
                    advice.push("High timeout ratio — path may be filtered.".to_string());
                    advice.push("Try: proto=tcp port=443 for HTTPS-friendly probing".to_string());
                }
            } else {
                advice.push("Target reached successfully.".to_string());
            }
            for indicator in &r.blocked_indicators {
                advice.push(format!("Blocked indicator: {indicator}"));
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::MtuProbe(r) => {
            if let Some(pmtu) = r.path_mtu {
                advice.push(format!("Path MTU: {} bytes", pmtu));
                if pmtu < 1472 {
                    advice.push("Low PMTU detected — possible tunnel or VPN overhead.".to_string());
                    advice.push(format!(
                        "Consider: MSS clamp to {} if using VPN",
                        pmtu.saturating_sub(40)
                    ));
                }
            } else {
                advice.push("Could not determine path MTU.".to_string());
                advice.push("Check if ICMP is allowed on the path.".to_string());
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::PortScan(r) => {
            if !r.open_ports.is_empty() {
                advice.push(format!("Open ports: {:?}", r.open_ports));
            }
            let closed: Vec<u16> = r
                .scanned_ports
                .iter()
                .filter(|p| !r.open_ports.contains(p))
                .copied()
                .collect();
            if !closed.is_empty() {
                advice.push(format!("Closed/filtered: {:?}", closed));
                advice.push("These ports may be behind a firewall.".to_string());
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::DnsExplain(r) => {
            if !r.conflicts.is_empty() {
                advice.push("DNS configuration conflicts detected:".to_string());
                for conflict in &r.conflicts {
                    advice.push(format!("  - {conflict}"));
                }
            }
            if r.dns_servers.is_empty() {
                advice.push("No DNS servers found! Check resolv.conf.".to_string());
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::ConnectionLab(r) => {
            if r.permission_limited {
                advice.push("Results are permission-limited.".to_string());
                advice.push("Run TUI+ with sudo for full socket details.".to_string());
            }
            let estab = r
                .entries
                .iter()
                .filter(|e| e.state.eq_ignore_ascii_case("ESTAB"))
                .count();
            if estab > 50 {
                advice.push(format!("High connection count ({estab} established)."));
                advice.push("Check for connection leaks or misbehaving processes.".to_string());
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::Resolve(r) => {
            if r.addresses.is_empty() {
                advice.push("No addresses resolved — check DNS config.".to_string());
                advice.push("Run DNS Explain to inspect resolver chain.".to_string());
            } else if r.addresses.len() > 1 {
                advice.push(format!(
                    "Multiple addresses ({}) — load-balanced or CDN.",
                    r.addresses.len()
                ));
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::RouteInspect(r) => {
            if r.default_routes.is_empty() {
                advice.push("No default routes found — connectivity may be limited.".to_string());
            }
            if !r.warnings.is_empty() {
                advice.push("Route warnings detected — review for misconfigs.".to_string());
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::NicDeepInfo(r) => {
            for iface in &r.interfaces {
                if iface.mtu < 1500 && !iface.interface.starts_with("lo") {
                    advice.push(format!(
                        "{}: Low MTU ({}) — may cause fragmentation.",
                        iface.interface, iface.mtu
                    ));
                }
            }
            if !r.warnings.is_empty() {
                for w in &r.warnings {
                    advice.push(format!("Check: {w}"));
                }
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::NatCapabilityCheck(r) => {
            let any_supported = r
                .methods
                .iter()
                .any(|m| m.state == linux_netdiag::CapabilityState::Supported);
            let any_missing = r
                .methods
                .iter()
                .any(|m| m.state == linux_netdiag::CapabilityState::MissingDependency);
            if any_supported {
                advice.push("NAT traversal available — port forwarding is possible.".to_string());
            } else {
                advice.push("No NAT traversal methods available.".to_string());
                advice.push("Consider manual port forwarding on router.".to_string());
            }
            if any_missing {
                advice.push(String::new());
                advice.push("Missing dependencies detected:".to_string());
                for m in &r.methods {
                    if m.state == linux_netdiag::CapabilityState::MissingDependency {
                        advice.push(format!("  {} — {}", m.method, m.details));
                    }
                }
                advice.push(String::new());
                advice.push("Install: sudo apt install miniupnpc".to_string());
                advice.push("  (provides upnpc and natpmpc commands)".to_string());
                advice.push("Or: sudo dnf install miniupnpc".to_string());
                advice.push("Or: sudo pacman -S miniupnpc".to_string());
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::MappingTest(r) => {
            if r.created {
                advice.push("Port mapping created successfully.".to_string());
                if !r.removed {
                    advice.push("Mapping was not auto-removed — remember to clean up.".to_string());
                }
            } else {
                advice.push("Port mapping creation failed.".to_string());
                advice.push("Check NAT Capability to verify available methods.".to_string());
            }
        }
        linux_netdiag::NetworkDiagnosticsResult::ExportReport(_) => {
            advice.push("Report exported successfully.".to_string());
            advice.push("Check working directory for the exported file.".to_string());
        }
    }
    advice
}

#[cfg(target_os = "linux")]
fn fmt_opt_ms(value: Option<f32>) -> String {
    value
        .map(|v| format!("{v:.2}"))
        .unwrap_or_else(|| "n/a".to_string())
}

pub(crate) fn sort_ollama_models(
    models: &mut Vec<OllamaModel>,
    column: OllamaModelSortColumn,
    ascending: bool,
) {
    models.sort_by(|a, b| {
        let ordering = match column {
            OllamaModelSortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            OllamaModelSortColumn::Params => {
                let (a_rank, a_val) = params_sort_key(a.params_unit, a.params_value);
                let (b_rank, b_val) = params_sort_key(b.params_unit, b.params_value);
                match a_rank.cmp(&b_rank) {
                    Ordering::Equal => a_val.partial_cmp(&b_val).unwrap_or(Ordering::Equal),
                    other => other,
                }
            }
            OllamaModelSortColumn::Modified => {
                a.modified.to_lowercase().cmp(&b.modified.to_lowercase())
            }
        };
        if ascending {
            ordering
        } else {
            ordering.reverse()
        }
    });
}

pub(crate) fn sort_ollama_running(
    models: &mut Vec<RunningModel>,
    column: OllamaRunningSortColumn,
    ascending: bool,
    paused_chats: &[ChatSession],
    active_chat_model: Option<&str>,
    active_messages: &[ChatMessage],
) {
    let mut paused_map = HashMap::new();
    for session in paused_chats {
        paused_map.insert(session.model.clone(), session.paused_at);
    }

    let mut message_count_map = HashMap::new();
    for session in paused_chats {
        let count = session
            .messages
            .iter()
            .filter(|message| message.role == ChatRole::Assistant)
            .count();
        message_count_map.insert(session.model.clone(), count);
    }
    if let Some(model) = active_chat_model {
        let count = active_messages
            .iter()
            .filter(|message| message.role == ChatRole::Assistant)
            .count();
        message_count_map.insert(model.to_string(), count);
    }

    models.sort_by(|a, b| {
        let ordering = match column {
            OllamaRunningSortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            OllamaRunningSortColumn::Params => {
                let (a_rank, a_val) = params_sort_key(a.params_unit, a.params_value);
                let (b_rank, b_val) = params_sort_key(b.params_unit, b.params_value);
                match a_rank.cmp(&b_rank) {
                    Ordering::Equal => a_val.partial_cmp(&b_val).unwrap_or(Ordering::Equal),
                    other => other,
                }
            }
            OllamaRunningSortColumn::PausedAt => {
                let a_paused = paused_map.get(&a.name).copied().unwrap_or(u64::MAX);
                let b_paused = paused_map.get(&b.name).copied().unwrap_or(u64::MAX);
                a_paused.cmp(&b_paused)
            }
            OllamaRunningSortColumn::MessageCount => {
                let a_count = message_count_map.get(&a.name).copied().unwrap_or(0);
                let b_count = message_count_map.get(&b.name).copied().unwrap_or(0);
                a_count.cmp(&b_count)
            }
        };
        if ascending {
            ordering
        } else {
            ordering.reverse()
        }
    });
}

fn params_sort_key(unit: Option<char>, value: Option<f64>) -> (u8, f64) {
    let rank = match unit.map(|u| u.to_ascii_uppercase()) {
        Some('M') => 0,
        Some('B') => 1,
        Some('T') => 2,
        _ => u8::MAX,
    };
    let val = value.unwrap_or(f64::MAX);
    (rank, val)
}
