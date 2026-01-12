use anyhow::Result;
use chrono::Local;
use crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent,
    MouseEventKind,
};
use crossterm::terminal;
use parking_lot::RwLock;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{monitors_task, Config, TabManager, TabType};
use crate::integrations::{ChatLogMetadata, OllamaClient, OllamaData, PowerShellExecutor};
use crate::integrations::ollama::{OllamaModel, RunningModel};
use crate::monitors::{
    CpuData, DiskAnalyzerData, DiskData, GpuData, NetworkData, ProcessData, RamData, ServiceData,
};
use crate::utils::command_history::CommandHistory;
use std::fs;

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
    pub command_menu_active: bool,
    pub command_history: CommandHistory,
    pub command_input: String,
    #[allow(dead_code)]
    pub selected_section: Option<String>,
    pub last_nav_input: Option<Instant>,
    pub last_horizontal_nav_input: Option<Instant>,
    pub last_sort_input: Option<Instant>,
    pub last_widget_scroll_input: Option<Instant>,
    pub last_view_toggle_input: Option<Instant>,
    pub last_text_input: Option<Instant>,
    pub last_backspace_input: Option<Instant>,
    pub terminal_size: (u16, u16),

    // GPU UI state
    pub gpu_state: GpuUIState,

    // RAM UI state
    pub ram_state: RamUIState,

    // Processes UI state
    pub processes_state: ProcessesUIState,

    // Services UI state
    pub services_state: ServicesUIState,

    // Ollama UI state
    pub ollama_state: OllamaUIState,

    // Disk Analyzer UI state
    pub disk_analyzer_state: DiskAnalyzerUIState,

    // Monitor control
    pub monitors_running: Arc<RwLock<bool>>,
    pub ms_keys_pressed: Option<Instant>,
    pub pressed_keys: HashSet<KeyCode>,
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
}

#[derive(Debug, Clone)]
pub enum OllamaDeleteTarget {
    Model(String),
    ChatLog(crate::integrations::ollama::ChatLogEntry),
}

// Disk Analyzer UI State
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskAnalyzerSortColumn {
    Name,
    Size,
    Type,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskAnalyzerTypeFilter {
    All,
    Folders,
    Files,
}

#[derive(Debug, Clone)]
pub struct TreeNode {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub depth: usize,
    pub expanded: bool,
    pub has_children: bool,
    pub loading: bool,
    pub is_file: bool,
    pub extension: Option<String>,
    pub file_count: Option<usize>,
    pub folder_count: Option<usize>,
    pub extension_counts: Option<std::collections::HashMap<String, usize>>,
}

impl TreeNode {
    pub fn from_root_folder(folder: &crate::monitors::RootFolderInfo) -> Self {
        Self {
            path: folder.path.clone(),
            name: folder.name.clone(),
            size: folder.size,
            depth: 0,
            expanded: false,
            has_children: true,
            loading: false,
            is_file: false,
            extension: None,
            file_count: None,
            folder_count: None,
            extension_counts: None,
        }
    }

    pub fn from_file(name: String, path: String, size: u64, depth: usize) -> Self {
        let extension = std::path::Path::new(&name)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{}", e.to_lowercase()));
        Self {
            path,
            name,
            size,
            depth,
            expanded: false,
            has_children: false,
            loading: false,
            is_file: true,
            extension,
            file_count: None,
            folder_count: None,
            extension_counts: None,
        }
    }
}

pub struct DiskAnalyzerUIState {
    pub selected_drive: usize,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub horizontal_offset: usize,
    pub in_tree_mode: bool,
    pub sort_column: DiskAnalyzerSortColumn,
    pub sort_ascending: bool,
    pub extended_view: bool,
    pub show_files: bool,
    pub type_filter: DiskAnalyzerTypeFilter,
    pub trees: std::collections::HashMap<String, Vec<TreeNode>>,
    pub path_copied_at: Option<(String, Instant)>,
    pub collapse_keys_pressed: Option<Instant>,
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
        Self::allow_with_throttle(
            &mut self.last_view_toggle_input,
            Duration::from_millis(200),
        )
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
        let (params_value, params_unit, params_display) =
            Self::parse_params_from_name(model_name);
        let is_cloud = model_name.to_ascii_lowercase().contains("cloud");
        RunningModel {
            name: model_name.to_string(),
            size_bytes: 0,
            size_display: "-".to_string(),
            gpu_memory_mb: None,
            gpu_memory_display: if is_cloud { "cloud".to_string() } else { "-".to_string() },
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
        Self::allow_with_throttle(&mut self.last_text_input, Duration::from_millis(35))
    }

    fn allow_backspace_input(&mut self) -> bool {
        Self::allow_with_throttle(&mut self.last_backspace_input, Duration::from_millis(50))
    }

    fn suggested_chat_prompt_height(&self, rows: u16) -> u16 {
        let fixed = if self.compact_mode { 3 } else { 3 + 8 + 5 };
        let min_main = 10;
        let available = rows.saturating_sub(fixed);
        let half = available / 2;
        let max_prompt = rows
            .saturating_sub(fixed.saturating_add(min_main))
            .max(3);
        half.max(3).min(max_prompt)
    }

    fn max_chat_prompt_height(&self) -> u16 {
        let (_, rows) = self.terminal_size;
        let reserved = if self.compact_mode { 3 + 6 } else { 3 + 8 + 5 + 10 };
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

    fn allow_with_throttle(
        last_input: &mut Option<Instant>,
        min_delay: Duration,
    ) -> bool {
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
                ChatRole::User => Self::append_chat_lines(&mut prompt, "Запрос: ", &message.text),
                ChatRole::Assistant => {
                    Self::append_chat_lines(&mut prompt, "Ответ: ", &message.text)
                }
            }
        }
        Self::append_chat_lines(&mut prompt, "Запрос: ", new_prompt);
        prompt.push_str("Ответ: ");
        prompt
    }

    fn build_chat_log(&self) -> String {
        let mut log = String::new();
        for message in &self.ollama_state.chat_messages {
            match message.role {
                ChatRole::User => Self::append_chat_lines(&mut log, "Запрос: ", &message.text),
                ChatRole::Assistant => Self::append_chat_lines(&mut log, "Ответ: ", &message.text),
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
        const USER_PREFIXES: [&str; 3] = ["Запрос:", "Р—Р°РїСЂРѕСЃ:", "Request:"];
        const ASSIST_PREFIXES: [&str; 3] = ["Ответ:", "РћС‚РІРµС‚:", "Response:"];

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

    async fn send_ollama_chat_prompt(&mut self, prompt: String) -> Result<()> {
        let model_name = match self.ollama_state.active_chat_model.clone() {
            Some(name) => name,
            None => return Ok(()),
        };

        let full_prompt = self.build_chat_prompt(&prompt);
        self.ollama_state.chat_messages.push(ChatMessage {
            role: ChatRole::User,
            text: prompt,
        });

        let response = OllamaClient::new(None)?
            .run_model(&model_name, &full_prompt)
            .await
            .unwrap_or_default()
            .trim()
            .to_string();
        let response = Self::normalize_model_response(&response);

        if !response.is_empty() {
            self.ollama_state.chat_messages.push(ChatMessage {
                role: ChatRole::Assistant,
                text: response,
            });
        }

        self.ollama_state.chat_scroll = usize::MAX;
        Ok(())
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
        self.ollama_state.input_mode = OllamaInputMode::None;
        self.ollama_state.input_buffer.clear();
        self.ollama_state.focused_panel = OllamaPanelFocus::Main;
        self.ollama_state.activity_view = OllamaActivityView::List;
        self.ollama_state.activity_log_lines.clear();
        self.ollama_state.activity_log_title.clear();
        self.ollama_state.activity_log_scroll = 0;
        self.close_activity_additions();
    }

    async fn run_ollama_command(&mut self, command: String) {
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

        self.ollama_state.activity_view = OllamaActivityView::Log;
        self.ollama_state.activity_log_lines = lines;
        self.ollama_state.activity_log_title = title;
        self.ollama_state.activity_log_scroll = 0;
        self.ollama_state.focused_panel = OllamaPanelFocus::Activity;
        self.close_activity_additions();
    }

    // Disk Analyzer helper methods
    fn get_current_drive_letter(&self) -> Option<String> {
        self.disk_analyzer_data
            .read()
            .as_ref()
            .and_then(|data| data.drives.get(self.disk_analyzer_state.selected_drive))
            .map(|drive| drive.letter.clone())
    }

    fn get_drive_count(&self) -> usize {
        self.disk_analyzer_data
            .read()
            .as_ref()
            .map(|data| data.drives.len())
            .unwrap_or(0)
    }

    pub fn has_any_expanded_folder(&self) -> bool {
        if let Some(drive_letter) = self.get_current_drive_letter() {
            if let Some(tree) = self.disk_analyzer_state.trees.get(&drive_letter) {
                return tree.iter().any(|node| node.expanded);
            }
        }
        false
    }

    pub fn get_tree_item_count(&self) -> usize {
        if let Some(drive_letter) = self.get_current_drive_letter() {
            if let Some(tree) = self.disk_analyzer_state.trees.get(&drive_letter) {
                return tree.len();
            }
        }
        // Fallback to root folders count
        self.disk_analyzer_data
            .read()
            .as_ref()
            .and_then(|data| data.drives.get(self.disk_analyzer_state.selected_drive))
            .map(|drive| drive.root_folders.len())
            .unwrap_or(0)
    }

    pub fn get_selected_tree_node(&self) -> Option<TreeNode> {
        if let Some(drive_letter) = self.get_current_drive_letter() {
            if let Some(tree) = self.disk_analyzer_state.trees.get(&drive_letter) {
                return tree.get(self.disk_analyzer_state.selected_index).cloned();
            }
        }
        None
    }

    async fn toggle_folder_expansion(&mut self) {
        let drive_letter = match self.get_current_drive_letter() {
            Some(letter) => letter,
            None => return,
        };

        let selected_idx = self.disk_analyzer_state.selected_index;

        // Initialize tree if needed
        if !self.disk_analyzer_state.trees.contains_key(&drive_letter) {
            let root_folders = self.disk_analyzer_data
                .read()
                .as_ref()
                .and_then(|data| data.drives.get(self.disk_analyzer_state.selected_drive))
                .map(|drive| drive.root_folders.clone())
                .unwrap_or_default();

            let tree: Vec<TreeNode> = root_folders
                .iter()
                .map(TreeNode::from_root_folder)
                .collect();

            self.disk_analyzer_state.trees.insert(drive_letter.clone(), tree);
        }

        let tree = match self.disk_analyzer_state.trees.get_mut(&drive_letter) {
            Some(tree) => tree,
            None => return,
        };

        if selected_idx >= tree.len() {
            return;
        }

        let node = &tree[selected_idx];
        let is_expanded = node.expanded;
        let node_path = node.path.clone();
        let node_depth = node.depth;

        if is_expanded {
            // Collapse: remove all children
            tree[selected_idx].expanded = false;
            let mut remove_indices = Vec::new();
            for (i, child) in tree.iter().enumerate().skip(selected_idx + 1) {
                if child.depth > node_depth {
                    remove_indices.push(i);
                } else {
                    break;
                }
            }
            for i in remove_indices.into_iter().rev() {
                tree.remove(i);
            }
        } else {
            // Expand: fetch and insert children
            tree[selected_idx].loading = true;
            tree[selected_idx].expanded = true;

            // Enter tree mode
            self.disk_analyzer_state.in_tree_mode = true;

            // Get extensions to track from config
            let extensions = self.config
                .read()
                .integrations
                .disk_analyzer
                .show_extensions
                .clone();

            // Query subfolders using Everything
            let es_exe = self.config.read().integrations.everything.es_executable.clone();
            let timeout = self.config.read().integrations.everything.refresh_interval_ms / 1000;

            if let Ok(monitor) = crate::monitors::DiskAnalyzerMonitor::new(
                crate::integrations::PowerShellExecutor::new(
                    self.config.read().powershell.executable.clone(),
                    self.config.read().powershell.timeout_seconds,
                    self.config.read().powershell.cache_ttl_seconds,
                    self.config.read().powershell.use_cache,
                ),
                es_exe,
                0,
                timeout.max(5),
            ) {
                match monitor.query_folder_contents(&node_path, &extensions).await {
                    Ok(contents) => {
                        // Get mutable reference again after async
                        if let Some(tree) = self.disk_analyzer_state.trees.get_mut(&drive_letter) {
                            if selected_idx < tree.len() {
                                tree[selected_idx].loading = false;
                                tree[selected_idx].expanded = true; // Ensure expanded stays true after async
                                tree[selected_idx].file_count = Some(contents.file_count);
                                tree[selected_idx].folder_count = Some(contents.folder_count);
                                tree[selected_idx].extension_counts = Some(contents.extension_counts);
                                // has_children indicates if there's content, but expanded shows user expanded it
                                let has_content = !contents.subfolders.is_empty() || !contents.files.is_empty();
                                tree[selected_idx].has_children = has_content;

                                // Build folder nodes
                                let mut folder_nodes: Vec<TreeNode> = contents
                                    .subfolders
                                    .iter()
                                    .map(|folder| {
                                        let mut node = TreeNode::from_root_folder(folder);
                                        node.depth = node_depth + 1;
                                        node
                                    })
                                    .collect();

                                // Build file nodes
                                let mut file_nodes: Vec<TreeNode> = contents
                                    .files
                                    .iter()
                                    .map(|file| {
                                        TreeNode::from_file(
                                            file.name.clone(),
                                            file.path.clone(),
                                            file.size,
                                            node_depth + 1,
                                        )
                                    })
                                    .collect();

                                // Sort folders
                                match self.disk_analyzer_state.sort_column {
                                    DiskAnalyzerSortColumn::Size => {
                                        if self.disk_analyzer_state.sort_ascending {
                                            folder_nodes.sort_by(|a, b| a.size.cmp(&b.size));
                                        } else {
                                            folder_nodes.sort_by(|a, b| b.size.cmp(&a.size));
                                        }
                                    }
                                    DiskAnalyzerSortColumn::Name | DiskAnalyzerSortColumn::Type => {
                                        // For folders, Type sort is same as Name sort
                                        if self.disk_analyzer_state.sort_ascending {
                                            folder_nodes.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                                        } else {
                                            folder_nodes.sort_by(|a, b| b.name.to_lowercase().cmp(&a.name.to_lowercase()));
                                        }
                                    }
                                }

                                // Sort files
                                match self.disk_analyzer_state.sort_column {
                                    DiskAnalyzerSortColumn::Size => {
                                        if self.disk_analyzer_state.sort_ascending {
                                            file_nodes.sort_by(|a, b| a.size.cmp(&b.size));
                                        } else {
                                            file_nodes.sort_by(|a, b| b.size.cmp(&a.size));
                                        }
                                    }
                                    DiskAnalyzerSortColumn::Name => {
                                        if self.disk_analyzer_state.sort_ascending {
                                            file_nodes.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                                        } else {
                                            file_nodes.sort_by(|a, b| b.name.to_lowercase().cmp(&a.name.to_lowercase()));
                                        }
                                    }
                                    DiskAnalyzerSortColumn::Type => {
                                        // Sort by extension
                                        if self.disk_analyzer_state.sort_ascending {
                                            file_nodes.sort_by(|a, b| {
                                                let ext_a = a.extension.as_deref().unwrap_or("");
                                                let ext_b = b.extension.as_deref().unwrap_or("");
                                                ext_a.to_lowercase().cmp(&ext_b.to_lowercase())
                                            });
                                        } else {
                                            file_nodes.sort_by(|a, b| {
                                                let ext_a = a.extension.as_deref().unwrap_or("");
                                                let ext_b = b.extension.as_deref().unwrap_or("");
                                                ext_b.to_lowercase().cmp(&ext_a.to_lowercase())
                                            });
                                        }
                                    }
                                }

                                // Insert folders first, then files (always load files for F toggle)
                                let mut insert_pos = selected_idx + 1;
                                for node in folder_nodes {
                                    tree.insert(insert_pos, node);
                                    insert_pos += 1;
                                }

                                // Always insert files (visibility controlled by show_files in UI)
                                for node in file_nodes {
                                    tree.insert(insert_pos, node);
                                    insert_pos += 1;
                                }
                            }
                        }
                    }
                    Err(_) => {
                        if let Some(tree) = self.disk_analyzer_state.trees.get_mut(&drive_letter) {
                            if selected_idx < tree.len() {
                                tree[selected_idx].loading = false;
                                tree[selected_idx].expanded = true; // Keep expanded even on error
                                tree[selected_idx].has_children = false;
                            }
                        }
                    }
                }
            }
        }
    }

    fn open_selected_in_explorer(&self) {
        if let Some(node) = self.get_selected_tree_node() {
            #[cfg(windows)]
            {
                let _ = std::process::Command::new("explorer")
                    .arg(&node.path)
                    .spawn();
            }
        }
    }

    fn copy_selected_path(&mut self) {
        if let Some(node) = self.get_selected_tree_node() {
            // For directories, copy the path directly
            // For files, copy parent directory (but we only have directories here)
            let path_to_copy = node.path.trim_end_matches('\\').to_string();

            #[cfg(windows)]
            {
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    let _ = clipboard.set_text(&path_to_copy);
                    self.disk_analyzer_state.path_copied_at = Some((path_to_copy, Instant::now()));
                }
            }

            #[cfg(not(windows))]
            {
                self.disk_analyzer_state.path_copied_at = Some((path_to_copy, Instant::now()));
            }
        }
    }

    pub async fn new(config: Config) -> Result<Self> {
        let tab_manager = TabManager::new(config.tabs.enabled.clone(), &config.tabs.default);

        let command_history = CommandHistory::new(config.ui.command_history.max_entries);

        let config = Arc::new(RwLock::new(config));

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

        let monitors_running = Arc::new(RwLock::new(true));

        // Start monitor tasks
        monitors_task::spawn_monitor_tasks(
            Arc::clone(&config),
            Arc::clone(&monitors_running),
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

        Ok(Self {
            config,
            tab_manager,
            compact_mode: false,

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

            command_menu_active: false,
            command_history,
            command_input: String::new(),
            selected_section: None,
            last_nav_input: None,
            last_horizontal_nav_input: None,
            last_sort_input: None,
            last_widget_scroll_input: None,
            last_view_toggle_input: None,
            last_text_input: None,
            last_backspace_input: None,
            terminal_size: terminal::size().unwrap_or((120, 40)),

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
            },

            disk_analyzer_state: DiskAnalyzerUIState {
                selected_drive: 0,
                selected_index: 0,
                scroll_offset: 0,
                horizontal_offset: 0,
                in_tree_mode: false,
                sort_column: DiskAnalyzerSortColumn::Size,
                sort_ascending: false,
                extended_view: false,
                show_files: true,
                type_filter: DiskAnalyzerTypeFilter::All,
                trees: std::collections::HashMap::new(),
                path_copied_at: None,
                collapse_keys_pressed: None,
            },

            monitors_running,
            ms_keys_pressed: None,
            pressed_keys: HashSet::new(),
        })
    }

    pub async fn handle_event(&mut self, event: CrosstermEvent) -> Result<bool> {
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
        let is_release = matches!(key.kind, KeyEventKind::Release);

        // Track M and S key presses for monitor toggle
        match key.code {
            KeyCode::Char('m') | KeyCode::Char('M') => {
                if is_initial_press && !key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.pressed_keys.insert(KeyCode::Char('m'));
                } else if is_release {
                    self.pressed_keys.remove(&KeyCode::Char('m'));
                    self.ms_keys_pressed = None;
                }
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                if is_initial_press && !key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.pressed_keys.insert(KeyCode::Char('s'));
                } else if is_release {
                    self.pressed_keys.remove(&KeyCode::Char('s'));
                    self.ms_keys_pressed = None;
                }
            }
            _ => {}
        }

        // Check if both M and S are pressed
        if self.pressed_keys.contains(&KeyCode::Char('m'))
            && self.pressed_keys.contains(&KeyCode::Char('s'))
        {
            if self.ms_keys_pressed.is_none() {
                self.ms_keys_pressed = Some(Instant::now());
            } else if let Some(start_time) = self.ms_keys_pressed {
                if start_time.elapsed() >= Duration::from_secs(3) {
                    // Toggle monitors
                    let mut running = self.monitors_running.write();
                    *running = !*running;
                    self.ms_keys_pressed = None;
                    self.pressed_keys.clear();
                    log::info!("Monitors toggled: {}", *running);
                }
            }
        }

        // Handle Ctrl+C to quit
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Ok(false);
        }

        // Handle Ctrl+F to open command history menu
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('f') {
            if is_initial_press {
                self.command_menu_active = !self.command_menu_active;
            }
            return Ok(true);
        }

        // If command menu is active, handle navigation
        if self.command_menu_active {
            match key.code {
                KeyCode::Esc => {
                    self.command_menu_active = false;
                }
                KeyCode::Enter if is_initial_press => {
                    // First Enter: insert command into input
                    if let Some(cmd) = self.command_history.get_selected() {
                        self.command_input = cmd.clone();
                        self.command_menu_active = false;
                    }
                }
                KeyCode::Up if is_initial_press => {
                    self.command_history.previous();
                }
                KeyCode::Down if is_initial_press => {
                    self.command_history.next();
                }
                KeyCode::Tab if is_initial_press => {
                    self.command_history.next();
                }
                KeyCode::BackTab if is_initial_press => {
                    self.command_history.previous();
                }
                _ => {}
            }
            return Ok(true);
        }

        // Handle command input
        if !self.command_input.is_empty() {
            match key.code {
                KeyCode::Enter if is_initial_press => {
                    // Execute command
                    self.execute_command().await?;
                    self.command_input.clear();
                }
                KeyCode::Esc => {
                    self.command_input.clear();
                }
                KeyCode::Backspace => {
                    self.command_input.pop();
                }
                KeyCode::Char(c) => {
                    self.command_input.push(c);
                }
                _ => {}
            }
            return Ok(true);
        }

        // Handle tab-specific hotkeys first
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
                        self.gpu_state.selected_index =
                            next.min(process_count.saturating_sub(1));
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
                        self.ram_state.selected_index =
                            next.min(process_count.saturating_sub(1));
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
                        self.services_state.focused_panel = match self.services_state.focused_panel {
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

        // Disk Analyzer tab hotkeys
        if self.tab_manager.current() == TabType::DiskAnalyzer {
            // Check for Ctrl+S hold (2 seconds) to collapse tree
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
                if is_initial_press {
                    self.disk_analyzer_state.collapse_keys_pressed = Some(Instant::now());
                }
            } else if is_release {
                self.disk_analyzer_state.collapse_keys_pressed = None;
            }

            // Check if collapse timer expired
            if let Some(start) = self.disk_analyzer_state.collapse_keys_pressed {
                if start.elapsed() >= Duration::from_secs(2) {
                    // Collapse all trees
                    let current_drive = self.get_current_drive_letter();
                    if let Some(drive_letter) = current_drive {
                        if let Some(tree) = self.disk_analyzer_state.trees.get_mut(&drive_letter) {
                            for node in tree.iter_mut() {
                                node.expanded = false;
                            }
                            // Remove all non-root nodes
                            tree.retain(|node| node.depth == 0);
                        }
                    }
                    self.disk_analyzer_state.in_tree_mode = false;
                    self.disk_analyzer_state.collapse_keys_pressed = None;
                    self.disk_analyzer_state.selected_index = 0;
                    self.disk_analyzer_state.scroll_offset = 0;
                    self.disk_analyzer_state.horizontal_offset = 0;
                    return Ok(true);
                }
            }

            let any_expanded = self.has_any_expanded_folder();

            match key.code {
                KeyCode::Esc => {
                    if self.disk_analyzer_state.in_tree_mode {
                        self.disk_analyzer_state.in_tree_mode = false;
                        return Ok(true);
                    }
                }
                KeyCode::Left => {
                    if !self.allow_horizontal_nav() {
                        return Ok(true);
                    }
                    if any_expanded && self.disk_analyzer_state.in_tree_mode {
                        // Horizontal scroll in tree mode
                        self.disk_analyzer_state.horizontal_offset =
                            self.disk_analyzer_state.horizontal_offset.saturating_sub(4);
                    } else if !any_expanded {
                        // Switch to previous drive
                        let drive_count = self.get_drive_count();
                        if drive_count > 0 && self.disk_analyzer_state.selected_drive > 0 {
                            self.disk_analyzer_state.selected_drive -= 1;
                            self.disk_analyzer_state.selected_index = 0;
                            self.disk_analyzer_state.scroll_offset = 0;
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Right => {
                    if !self.allow_horizontal_nav() {
                        return Ok(true);
                    }
                    if any_expanded && self.disk_analyzer_state.in_tree_mode {
                        // Horizontal scroll in tree mode
                        self.disk_analyzer_state.horizontal_offset += 4;
                    } else if !any_expanded {
                        // Switch to next drive
                        let drive_count = self.get_drive_count();
                        if drive_count > 0 && self.disk_analyzer_state.selected_drive + 1 < drive_count {
                            self.disk_analyzer_state.selected_drive += 1;
                            self.disk_analyzer_state.selected_index = 0;
                            self.disk_analyzer_state.scroll_offset = 0;
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Up => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    if self.disk_analyzer_state.selected_index > 0 {
                        self.disk_analyzer_state.selected_index -= 1;
                        // Keep 3-row buffer at top for anticipation scroll
                        let buffer = 3usize;
                        let position_in_view = self.disk_analyzer_state.selected_index
                            .saturating_sub(self.disk_analyzer_state.scroll_offset);
                        // If selected is within buffer rows from top, scroll
                        if position_in_view < buffer {
                            self.disk_analyzer_state.scroll_offset =
                                self.disk_analyzer_state.selected_index.saturating_sub(buffer);
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Down => {
                    if !self.allow_nav() {
                        return Ok(true);
                    }
                    let item_count = self.get_tree_item_count();
                    if self.disk_analyzer_state.selected_index + 1 < item_count {
                        self.disk_analyzer_state.selected_index += 1;
                        // Keep 3-row buffer at bottom for anticipation scroll
                        // Layout: drive tabs (3) + usage bar (3) + tree content (variable) + footer (2) + borders (2) = ~10
                        let visible_height = self.terminal_size.1.saturating_sub(10) as usize;
                        let buffer = 3usize;
                        if visible_height > buffer + 1 {
                            // Calculate position relative to visible area
                            let position_in_view = self.disk_analyzer_state.selected_index
                                .saturating_sub(self.disk_analyzer_state.scroll_offset);
                            // If selected is within buffer rows from bottom, scroll
                            if position_in_view + buffer >= visible_height {
                                self.disk_analyzer_state.scroll_offset =
                                    self.disk_analyzer_state.selected_index
                                        .saturating_sub(visible_height.saturating_sub(buffer).saturating_sub(1));
                            }
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Home => {
                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                        // Go to drive root (collapse all)
                        self.disk_analyzer_state.selected_index = 0;
                        self.disk_analyzer_state.scroll_offset = 0;
                    } else {
                        // Go to beginning of list
                        self.disk_analyzer_state.selected_index = 0;
                        self.disk_analyzer_state.scroll_offset = 0;
                    }
                    return Ok(true);
                }
                KeyCode::End => {
                    let item_count = self.get_tree_item_count();
                    if item_count > 0 {
                        self.disk_analyzer_state.selected_index = item_count - 1;
                        // Update scroll to show last item (with buffer at top)
                        let visible_height = self.terminal_size.1.saturating_sub(10) as usize;
                        if item_count > visible_height {
                            self.disk_analyzer_state.scroll_offset = item_count.saturating_sub(visible_height);
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Enter => {
                    if !is_initial_press {
                        return Ok(true);
                    }
                    // Toggle expand/collapse folder
                    self.toggle_folder_expansion().await;
                    return Ok(true);
                }
                KeyCode::Char('s') => {
                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                        // Handled above for collapse
                        return Ok(true);
                    }
                    // Don't trigger sort if M is pressed (M+S is global action for monitor toggle)
                    if self.pressed_keys.contains(&KeyCode::Char('m')) {
                        return Ok(true);
                    }
                    if !is_initial_press || !self.allow_sort_toggle() {
                        return Ok(true);
                    }
                    // Toggle sort
                    if self.disk_analyzer_state.sort_column == DiskAnalyzerSortColumn::Size {
                        self.disk_analyzer_state.sort_column = DiskAnalyzerSortColumn::Name;
                    } else {
                        self.disk_analyzer_state.sort_column = DiskAnalyzerSortColumn::Size;
                    }
                    self.disk_analyzer_state.sort_ascending = !self.disk_analyzer_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('e') | KeyCode::Char('E') => {
                    if !is_initial_press {
                        return Ok(true);
                    }
                    // Toggle extended view
                    self.disk_analyzer_state.extended_view = !self.disk_analyzer_state.extended_view;
                    return Ok(true);
                }
                KeyCode::Char('o') | KeyCode::Char('O') => {
                    if !is_initial_press {
                        return Ok(true);
                    }
                    // Open in explorer
                    self.open_selected_in_explorer();
                    return Ok(true);
                }
                KeyCode::Char('c') | KeyCode::Char('C') => {
                    if !is_initial_press {
                        return Ok(true);
                    }
                    // Copy path
                    self.copy_selected_path();
                    return Ok(true);
                }
                KeyCode::Char('f') | KeyCode::Char('F') => {
                    if !is_initial_press {
                        return Ok(true);
                    }
                    // Toggle show files
                    self.disk_analyzer_state.show_files = !self.disk_analyzer_state.show_files;
                    return Ok(true);
                }
                KeyCode::Char('t') | KeyCode::Char('T') => {
                    if !is_initial_press {
                        return Ok(true);
                    }
                    // Cycle through type filter
                    self.disk_analyzer_state.type_filter = match self.disk_analyzer_state.type_filter {
                        DiskAnalyzerTypeFilter::All => DiskAnalyzerTypeFilter::Folders,
                        DiskAnalyzerTypeFilter::Folders => DiskAnalyzerTypeFilter::Files,
                        DiskAnalyzerTypeFilter::Files => DiskAnalyzerTypeFilter::All,
                    };
                    // Reset selection when filter changes
                    self.disk_analyzer_state.selected_index = 0;
                    self.disk_analyzer_state.scroll_offset = 0;
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
                                        data.chat_logs
                                            .retain(|item| item.path != entry.path);
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
                                self.run_ollama_command(command).await;
                            }
                            self.ollama_state.input_buffer.clear();
                            self.ollama_state.input_mode = OllamaInputMode::None;
                        }
                        OllamaInputMode::Chat => {
                            // Only process on initial press, not repeat
                            if key.kind == KeyEventKind::Press {
                                let prompt = self.ollama_state.input_buffer.trim().to_string();
                                if !prompt.is_empty() {
                                    let _ = self.send_ollama_chat_prompt(prompt).await;
                                }
                                self.ollama_state.input_buffer.clear();
                                self.ollama_state.chat_prompt_scroll = 0;
                            }
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
                        // Add throttle for backspace in Chat mode
                        let allow = if self.ollama_state.input_mode == OllamaInputMode::Chat {
                            matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
                                && self.allow_backspace_input()
                        } else {
                            true
                        };
                        if allow {
                            self.ollama_state.input_buffer.pop();
                        }
                    }
                    KeyCode::Up | KeyCode::Down if self.ollama_state.input_mode == OllamaInputMode::Chat => {
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
                        let allow_input = if self.ollama_state.input_mode == OllamaInputMode::Chat
                        {
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
                                self.ollama_state.activity_selected = self
                                    .ollama_state
                                    .activity_selected
                                    .saturating_sub(step);
                                if self.ollama_state.activity_selected != prev {
                                    self.reset_activity_expand_state();
                                }
                            }
                            OllamaActivityView::Log => {
                                if !self.allow_widget_scroll() {
                                    return Ok(true);
                                }
                                self.ollama_state.activity_log_scroll = self
                                    .ollama_state
                                    .activity_log_scroll
                                    .saturating_sub(step);
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
                        self.ollama_state.pending_delete =
                            Some(OllamaDeleteTarget::Model(name));
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
            KeyCode::Up if is_initial_press => {
                // Navigate command history with arrow keys (only when not on Processes tab)
                self.command_history.previous();
                if let Some(cmd) = self.command_history.get_selected() {
                    self.command_input = cmd.clone();
                }
            }
            KeyCode::Down if is_initial_press => {
                self.command_history.next();
                if let Some(cmd) = self.command_history.get_selected() {
                    self.command_input = cmd.clone();
                }
            }
            _ => {}
        }

        Ok(true)
    }

    async fn handle_mouse_event(&mut self, mouse: MouseEvent) -> Result<bool> {
        match mouse.kind {
            MouseEventKind::Down(_) => {
                // Handle mouse clicks for radial menu
                if self.command_menu_active {
                    self.command_history
                        .handle_mouse_click(mouse.column, mouse.row);
                }
            }
            _ => {}
        }

        Ok(true)
    }

    async fn execute_command(&mut self) -> Result<()> {
        if self.command_input.is_empty() {
            return Ok(());
        }

        // Add to history
        self.command_history.add(self.command_input.clone());

        // Execute PowerShell command
        let ps = PowerShellExecutor::new(
            self.config.read().powershell.executable.clone(),
            self.config.read().powershell.timeout_seconds,
            self.config.read().powershell.cache_ttl_seconds,
            self.config.read().powershell.use_cache,
        );

        match ps.execute(&self.command_input).await {
            Ok(output) => {
                log::info!("Command output: {}", output);
            }
            Err(e) => {
                log::error!("Command failed: {}", e);
            }
        }

        Ok(())
    }
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
                    Ordering::Equal => a_val
                        .partial_cmp(&b_val)
                        .unwrap_or(Ordering::Equal),
                    other => other,
                }
            }
            OllamaModelSortColumn::Modified => a
                .modified
                .to_lowercase()
                .cmp(&b.modified.to_lowercase()),
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
                    Ordering::Equal => a_val
                        .partial_cmp(&b_val)
                        .unwrap_or(Ordering::Equal),
                    other => other,
                }
            }
            OllamaRunningSortColumn::PausedAt => {
                let a_paused = paused_map
                    .get(&a.name)
                    .copied()
                    .unwrap_or(u64::MAX);
                let b_paused = paused_map
                    .get(&b.name)
                    .copied()
                    .unwrap_or(u64::MAX);
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







