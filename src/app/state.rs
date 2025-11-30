use anyhow::Result;
use crossterm::event::{
    Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent,
    MouseEventKind,
};
use parking_lot::RwLock;
use std::sync::Arc;

use super::{monitors_task, Config, TabManager, TabType};
use crate::integrations::{OllamaData, PowerShellExecutor};
use crate::monitors::{CpuData, DiskData, GpuData, NetworkData, ProcessData, RamData, ServiceData};
use crate::utils::command_history::CommandHistory;

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
    pub selected_section: Option<String>,

    // Processes UI state
    pub processes_state: ProcessesUIState,

    // Services UI state
    pub services_state: ServicesUIState,

    // Ollama UI state
    pub ollama_state: OllamaUIState,
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

pub struct ServicesUIState {
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub sort_column: ServiceSortColumn,
    pub sort_ascending: bool,
    pub status_filter: ServiceStatusFilter,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OllamaView {
    Models,
    Running,
}

pub struct OllamaUIState {
    pub selected_model_index: usize,
    pub selected_running_index: usize,
    pub current_view: OllamaView,
    pub command_input: String,
    pub show_command_input: bool,
}

impl AppState {
    pub async fn new(config: Config) -> Result<Self> {
        let tab_manager = TabManager::new(config.tabs.enabled.clone(), &config.tabs.default);

        let command_history = CommandHistory::new(config.ui.command_history.max_entries);

        let cpu_data = Arc::new(RwLock::new(None));
        let cpu_error = Arc::new(RwLock::new(None));
        let gpu_data = Arc::new(RwLock::new(None));
        let gpu_error = Arc::new(RwLock::new(None));
        let ram_data = Arc::new(RwLock::new(None));
        let ram_error = Arc::new(RwLock::new(None));
        let disk_data = Arc::new(RwLock::new(None));
        let disk_error = Arc::new(RwLock::new(None));
        let network_data = Arc::new(RwLock::new(None));
        let network_error = Arc::new(RwLock::new(None));
        let process_data = Arc::new(RwLock::new(None));
        let process_error = Arc::new(RwLock::new(None));
        let service_data = Arc::new(RwLock::new(None));
        let service_error = Arc::new(RwLock::new(None));

        let ollama_data = Arc::new(RwLock::new(None));
        let ollama_error = Arc::new(RwLock::new(None));

        // Start monitor tasks
        monitors_task::spawn_monitor_tasks(
            Arc::clone(&cpu_data),
            Arc::clone(&cpu_error),
            Arc::clone(&gpu_data),
            Arc::clone(&gpu_error),
            Arc::clone(&ram_data),
            Arc::clone(&ram_error),
            Arc::clone(&disk_data),
            Arc::clone(&disk_error),
            Arc::clone(&network_data),
            Arc::clone(&network_error),
            Arc::clone(&process_data),
            Arc::clone(&process_error),
            Arc::clone(&service_data),
            Arc::clone(&service_error),
            Arc::clone(&ollama_data),
            Arc::clone(&ollama_error),
            config.powershell.executable.clone(),
            config.powershell.timeout_seconds,
            config.powershell.cache_ttl_seconds,
            config.powershell.use_cache,
        );

        Ok(Self {
            config: Arc::new(RwLock::new(config)),
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
            },

            ollama_state: OllamaUIState {
                selected_model_index: 0,
                selected_running_index: 0,
                current_view: OllamaView::Models,
                command_input: String::new(),
                show_command_input: false,
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
        let is_initial_press = matches!(key.kind, KeyEventKind::Press);
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

        // Services tab hotkeys
        if self.tab_manager.current() == TabType::Services {
            match key.code {
                KeyCode::Up => {
                    if self.services_state.selected_index > 0 {
                        self.services_state.selected_index -= 1;
                        if self.services_state.selected_index < self.services_state.scroll_offset {
                            self.services_state.scroll_offset = self.services_state.selected_index;
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Down => {
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
                    if self.services_state.selected_index >= 10 {
                        self.services_state.selected_index -= 10;
                    } else {
                        self.services_state.selected_index = 0;
                    }
                    self.services_state.scroll_offset = self.services_state.selected_index;
                    return Ok(true);
                }
                KeyCode::PageDown => {
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
                    self.services_state.sort_column = ServiceSortColumn::Name;
                    self.services_state.sort_ascending = !self.services_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('d') => {
                    self.services_state.sort_column = ServiceSortColumn::DisplayName;
                    self.services_state.sort_ascending = !self.services_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('s') => {
                    self.services_state.sort_column = ServiceSortColumn::Status;
                    self.services_state.sort_ascending = !self.services_state.sort_ascending;
                    return Ok(true);
                }
                KeyCode::Char('t') => {
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
            // Handle command input mode
            if self.ollama_state.show_command_input {
                match key.code {
                    KeyCode::Enter => {
                        // Execute ollama command (to be implemented in execute_command)
                        self.ollama_state.show_command_input = false;
                        // Command will be executed via execute_command later
                    }
                    KeyCode::Esc => {
                        self.ollama_state.show_command_input = false;
                        self.ollama_state.command_input.clear();
                    }
                    KeyCode::Backspace => {
                        self.ollama_state.command_input.pop();
                    }
                    KeyCode::Char(c) => {
                        self.ollama_state.command_input.push(c);
                    }
                    _ => {}
                }
                return Ok(true);
            }

            match key.code {
                KeyCode::Up => {
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
                    return Ok(true);
                }
                KeyCode::Down => {
                    match self.ollama_state.current_view {
                        OllamaView::Models => {
                            let model_count = self
                                .ollama_data
                                .read()
                                .as_ref()
                                .map(|d| d.models.len())
                                .unwrap_or(0);
                            if self.ollama_state.selected_model_index + 1 < model_count {
                                self.ollama_state.selected_model_index += 1;
                            }
                        }
                        OllamaView::Running => {
                            let running_count = self
                                .ollama_data
                                .read()
                                .as_ref()
                                .map(|d| d.running_models.len())
                                .unwrap_or(0);
                            if self.ollama_state.selected_running_index + 1 < running_count {
                                self.ollama_state.selected_running_index += 1;
                            }
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Char('v') => {
                    // Toggle between Models and Running view
                    self.ollama_state.current_view = match self.ollama_state.current_view {
                        OllamaView::Models => OllamaView::Running,
                        OllamaView::Running => OllamaView::Models,
                    };
                    return Ok(true);
                }
                KeyCode::Char('c') => {
                    // Open command input
                    self.ollama_state.show_command_input = true;
                    return Ok(true);
                }
                KeyCode::Char('r') => {
                    // Run selected model
                    if let Some(ollama) = self.ollama_data.read().as_ref() {
                        if let Some(model) =
                            ollama.models.get(self.ollama_state.selected_model_index)
                        {
                            let model_name = model.name.clone();
                            tokio::spawn(async move {
                                use crate::integrations::OllamaClient;
                                if let Ok(mut client) = OllamaClient::new(None) {
                                    let _ = client.run_model(&model_name).await;
                                }
                            });
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Char('s') => {
                    // Stop selected running model
                    if let Some(ollama) = self.ollama_data.read().as_ref() {
                        if let Some(running) = ollama
                            .running_models
                            .get(self.ollama_state.selected_running_index)
                        {
                            let model_name = running.name.clone();
                            tokio::spawn(async move {
                                use crate::integrations::OllamaClient;
                                if let Ok(client) = OllamaClient::new(None) {
                                    let _ = client.stop_model(&model_name).await;
                                }
                            });
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Char('d') => {
                    // Delete selected model
                    if let Some(ollama) = self.ollama_data.read().as_ref() {
                        if let Some(model) =
                            ollama.models.get(self.ollama_state.selected_model_index)
                        {
                            let model_name = model.name.clone();
                            tokio::spawn(async move {
                                use crate::integrations::OllamaClient;
                                if let Ok(client) = OllamaClient::new(None) {
                                    let _ = client.remove_model(&model_name).await;
                                }
                            });
                        }
                    }
                    return Ok(true);
                }
                KeyCode::Char('p') => {
                    // Pull model (open command input with "pull ")
                    self.ollama_state.show_command_input = true;
                    self.ollama_state.command_input = "pull ".to_string();
                    return Ok(true);
                }
                KeyCode::Char('l') => {
                    // Refresh list (force re-fetch)
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
