pub mod config;
pub mod console_state;
pub mod extensions;
pub mod history;
pub mod math;
pub mod monitors_task;
pub mod state;
pub mod sudo;
pub mod suggestions;
pub mod syntax;
pub mod tabs;

pub use config::{Config, ConfigManager};
pub use console_state::ConsoleState;
pub use state::AppState;
pub use tabs::{TabManager, TabType};

use anyhow::Result;
use crossterm::event::Event as CrosstermEvent;
use std::fs;
use std::sync::Arc;

use std::env;

fn is_tui_config_file(path: &std::path::Path) -> bool {
    if !path.exists() {
        return false;
    }

    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(_) => return false,
    };

    content.contains("[general]") && content.contains("[tabs]")
}

pub struct App {
    pub state: AppState,
    #[allow(dead_code)]
    pub config_manager: Option<Arc<ConfigManager>>,
}

impl App {
    pub async fn new() -> Result<Self> {
        let exe_tui_config_path = {
            let mut path = env::current_exe()?;
            path.set_file_name("tui-config.toml");
            path
        };

        let config_path = match env::current_dir() {
            Ok(cwd) => {
                let cwd_tui_config = cwd.join("tui-config.toml");
                let cwd_legacy_config = cwd.join("config.toml");
                let exe_legacy_config = exe_tui_config_path
                    .parent()
                    .map(|dir| dir.join("config.toml"));

                if cwd_tui_config.exists() {
                    cwd_tui_config
                } else if is_tui_config_file(&cwd_legacy_config) {
                    cwd_legacy_config
                } else if exe_tui_config_path.exists() {
                    exe_tui_config_path.clone()
                } else if exe_legacy_config
                    .as_ref()
                    .is_some_and(|path| is_tui_config_file(path))
                {
                    exe_legacy_config.expect("checked is_some_and")
                } else {
                    cwd_tui_config
                }
            }
            Err(_) => exe_tui_config_path.clone(),
        };

        let config = Config::load_or_default(&config_path)?;
        let config = Arc::new(parking_lot::RwLock::new(config));

        // Create config manager with hot reload
        let config_manager = ConfigManager::new(Arc::clone(&config), config_path);

        // Start watching for config changes
        if let Err(e) = config_manager.clone().watch() {
            log::warn!("Failed to start config hot reload: {}", e);
        } else {
            log::info!("Config hot reload enabled");
        }

        let state = AppState::new(Arc::clone(&config)).await?;

        Ok(Self {
            state,
            config_manager: Some(config_manager),
        })
    }

    pub async fn handle_event(&mut self, event: CrosstermEvent) -> Result<bool> {
        self.state
            .apply_config_updates(self.config_manager.as_deref());
        self.state.handle_event(event).await
    }

    /// Called on every tick to poll async updates (diagnostics, etc.)
    /// without requiring a user input event.
    pub fn tick(&mut self) {
        self.state.apply_async_updates();
        self.state.tick_console_sessions();
    }
}
