use anyhow::Result;
use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use parking_lot::RwLock;
use std::sync::Arc;

use super::{Config, TabManager, TabType};
use crate::monitors::{CpuData, GpuData, RamData, DiskData, NetworkData, ProcessData};
use crate::integrations::ollama::OllamaData;
use crate::utils::command_history::CommandHistory;

pub struct AppState {
    pub config: Arc<RwLock<Config>>,
    pub tab_manager: TabManager,
    pub compact_mode: bool,

    // Monitor data
    pub cpu_data: Arc<RwLock<Option<CpuData>>>,
    pub gpu_data: Arc<RwLock<Option<GpuData>>>,
    pub ram_data: Arc<RwLock<Option<RamData>>>,
    pub disk_data: Arc<RwLock<Option<DiskData>>>,
    pub network_data: Arc<RwLock<Option<NetworkData>>>,
    pub process_data: Arc<RwLock<Option<ProcessData>>>,

    // Ollama integration
    pub ollama_data: Arc<RwLock<Option<OllamaData>>>,

    // UI state
    pub command_menu_active: bool,
    pub command_history: CommandHistory,
    pub command_input: String,
    pub selected_section: Option<String>,
}

impl AppState {
    pub async fn new(config: Config) -> Result<Self> {
        let tab_manager = TabManager::new(
            config.tabs.enabled.clone(),
            &config.tabs.default,
        );

        let command_history = CommandHistory::new(config.ui.command_history.max_entries);

        Ok(Self {
            config: Arc::new(RwLock::new(config)),
            tab_manager,
            compact_mode: false,

            cpu_data: Arc::new(RwLock::new(None)),
            gpu_data: Arc::new(RwLock::new(None)),
            ram_data: Arc::new(RwLock::new(None)),
            disk_data: Arc::new(RwLock::new(None)),
            network_data: Arc::new(RwLock::new(None)),
            process_data: Arc::new(RwLock::new(None)),

            ollama_data: Arc::new(RwLock::new(None)),

            command_menu_active: false,
            command_history,
            command_input: String::new(),
            selected_section: None,
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
                // Navigate command history with arrow keys
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
                    self.command_history.handle_mouse_click(mouse.column, mouse.row);
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

        // TODO: Execute PowerShell command or Ollama command
        log::info!("Executing command: {}", self.command_input);

        Ok(())
    }
}
