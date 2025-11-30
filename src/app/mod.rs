pub mod state;
pub mod config;
pub mod tabs;
pub mod monitors_task;

pub use state::AppState;
pub use config::{Config, ConfigManager};
pub use tabs::{TabType, TabManager};

use anyhow::Result;
use crossterm::event::Event as CrosstermEvent;
use std::path::PathBuf;
use std::sync::Arc;

use std::env;

pub struct App {
    pub state: AppState,
    pub config_manager: Option<Arc<ConfigManager>>,
}

impl App {
    pub async fn new() -> Result<Self> {
        let mut config_path = env::current_exe()?;
        config_path.set_file_name("config.toml");

        let config = Config::load_or_default(&config_path)?;

        // Create config manager with hot reload
        let config_manager = ConfigManager::new(config.clone(), config_path);

        // Start watching for config changes
        if let Err(e) = config_manager.clone().watch() {
            log::warn!("Failed to start config hot reload: {}", e);
        } else {
            log::info!("Config hot reload enabled");
        }

        let state = AppState::new(config).await?;

        Ok(Self {
            state,
            config_manager: Some(config_manager),
        })
    }

    pub async fn handle_event(&mut self, event: CrosstermEvent) -> Result<bool> {
        self.state.handle_event(event).await
    }
}
