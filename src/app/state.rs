use anyhow::Result;
use chrono::{DateTime, Local};
use crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind,
};
use parking_lot::RwLock;
use std::sync::Arc;

use super::{monitors_task, Config, TabManager, TabType};
use crate::integrations::{ollama::OllamaData, PowerShellExecutor};
use crate::monitors::{CpuData, DiskData, GpuData, NetworkData, ProcessData, RamData};
use crate::utils::command_history::CommandHistory;

#[derive(Debug, Clone)]
pub enum MonitorStatus {
    Loading,
    Ready,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct MonitorState<T> {
    pub status: MonitorStatus,
    pub data: Option<T>,
    pub last_updated: Option<DateTime<Local>>,
}

impl<T> MonitorState<T> {
    pub fn new() -> Self {
        Self {
            status: MonitorStatus::Loading,
            data: None,
            last_updated: None,
        }
    }
}

pub struct AppState {
    pub config: Arc<RwLock<Config>>,
    pub tab_manager: TabManager,
    pub compact_mode: bool,

    // Monitor data
    pub cpu_data: Arc<RwLock<MonitorState<CpuData>>>,
    pub gpu_data: Arc<RwLock<MonitorState<GpuData>>>,
    pub ram_data: Arc<RwLock<MonitorState<RamData>>>,
    pub disk_data: Arc<RwLock<MonitorState<DiskData>>>,
    pub network_data: Arc<RwLock<MonitorState<NetworkData>>>,
    pub process_data: Arc<RwLock<MonitorState<ProcessData>>>,

    // Ollama integration
    pub ollama_data: Arc<RwLock<Option<OllamaData>>>,

    // UI state
    pub command_menu_active: bool,
    pub command_history: CommandHistory,
    pub command_input: String,
    pub selected_section: Option<String>,

    // Processes UI state
    pub processes_state: ProcessesUIState,
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

impl AppState {
    pub async fn new(config: Config) -> Result<Self> {
        let tab_manager = TabManager::new(config.tabs.enabled.clone(), &config.tabs.default);

        let command_history = CommandHistory::new(config.ui.command_history.max_entries);

        let cpu_data = Arc::new(RwLock::new(MonitorState::new()));
        let gpu_data = Arc::new(RwLock::new(MonitorState::new()));
        let ram_data = Arc::new(RwLock::new(MonitorState::new()));
        let disk_data = Arc::new(RwLock::new(MonitorState::new()));
        let network_data = Arc::new(RwLock::new(MonitorState::new()));
        let process_data = Arc::new(RwLock::new(MonitorState::new()));

        // Start monitor tasks
        monitors_task::spawn_monitor_tasks(
            Arc::clone(&cpu_data),
            Arc::clone(&gpu_data),
            Arc::clone(&ram_data),
            Arc::clone(&disk_data),
            Arc::clone(&network_data),
            Arc::clone(&process_data),
            config.powershell.executable.clone(),
            config.powershell.timeout_seconds,
            config.powershell.cache_ttl_seconds,
        );

        Ok(Self {
            config: Arc::new(RwLock::new(config)),
            tab_manager,
            compact_mode: false,

            cpu_data,
            gpu_data,
            ram_data,
            disk_data,
            network_data,
            process_data,

            ollama_data: Arc::new(RwLock::new(None)),

            command_menu_active: false,
            command_history,
            command_input: String::new(),
            selected_section: None,

            processes_state: ProcessesUIState {
                selected_index: 0,
                scroll_offset: 0,
                sort_column: ProcessSortColumn::Cpu,
                sort_ascending: false,
                filter: String::new(),
            },
        })
    }

    pub async fn handle_event(&mut self, event: CrosstermEvent) -> Result<bool> {
        match event {
            CrosstermEvent::Key(key_event) => self.handle_key_event(key_event).await,
            CrosstermEvent::Mouse(mouse_event) => self.handle_mouse_event(mouse_event).await,
            _ => Ok(true),
        }
    }

    async fn handle_key_event(&mut self, key: KeyEvent) -> Result<bool> {
        // Handle Ctrl+C to quit
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Ok(false);
        }

        // Handle Ctrl+F to open command history menu
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('f') {
            self.command_menu_active = !self.command_menu_active;
            return Ok(true);
        }

        // If command menu is active, handle navigation
        if self.command_menu_active {
            match key.code {
                KeyCode::Esc => {
                    self.command_menu_active = false;
                }
                KeyCode::Enter => {
                    // First Enter: insert command into input
                    if let Some(cmd) = self.command_history.get_selected() {
                        self.command_input = cmd.clone();
                        self.command_menu_active = false;
                    }
                }
                KeyCode::Up => {
                    self.command_history.previous();
                }
                KeyCode::Down => {
                    self.command_history.next();
                }
                KeyCode::Tab => {
                    self.command_history.next();
                }
                KeyCode::BackTab => {
                    self.command_history.previous();
                }
                _ => {}
            }
            return Ok(true);
        }

        // Handle command input
        if !self.command_input.is_empty() {
            match key.code {
                KeyCode::Enter => {
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
                    let process_count = self
                        .process_data
                        .read()
                        .data
                        .as_ref()
                        .map(|d| d.processes.len())
                        .unwrap_or(0);
                    if self.processes_state.selected_index + 1 < process_count {
                        self.processes_state.selected_index += 1;
                    }
                    return Ok(true);
                }
                KeyCode::PageUp => {
                    if self.processes_state.selected_index >= 10 {
                        self.processes_state.selected_index -= 10;
                    } else {
                        self.processes_state.selected_index = 0;
                    }
                    self.processes_state.scroll_offset = self.processes_state.selected_index;
                    return Ok(true);
                }
                KeyCode::PageDown => {
                    let process_count = self
                        .process_data
                        .read()
                        .data
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
                    self.processes_state.sort_column = ProcessSortColumn::Pid;
                    self.processes_state.sort_ascending = !self.processes_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('n') => {
                    self.processes_state.sort_column = ProcessSortColumn::Name;
                    self.processes_state.sort_ascending = !self.processes_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('c') => {
                    self.processes_state.sort_column = ProcessSortColumn::Cpu;
                    self.processes_state.sort_ascending = !self.processes_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('m') => {
                    self.processes_state.sort_column = ProcessSortColumn::Memory;
                    self.processes_state.sort_ascending = !self.processes_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('t') => {
                    self.processes_state.sort_column = ProcessSortColumn::Threads;
                    self.processes_state.sort_ascending = !self.processes_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('u') => {
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

        // Handle global hotkeys
        match key.code {
            KeyCode::F(2) => {
                self.compact_mode = !self.compact_mode;
            }
            KeyCode::Tab => {
                self.tab_manager.next();
            }
            KeyCode::BackTab => {
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
            KeyCode::Up => {
                // Navigate command history with arrow keys (only when not on Processes tab)
                self.command_history.previous();
                if let Some(cmd) = self.command_history.get_selected() {
                    self.command_input = cmd.clone();
                }
            }
            KeyCode::Down => {
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
