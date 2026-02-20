pub mod config;
pub mod monitors_task;
pub mod state;
pub mod tabs;

pub use config::{Config, ConfigManager};
pub use state::AppState;
pub use tabs::{TabManager, TabType};

use anyhow::Result;
use crossterm::event::Event as CrosstermEvent;
use std::sync::Arc;

use std::env;

pub struct App {
    pub state: AppState,
    #[allow(dead_code)]
    pub config_manager: Option<Arc<ConfigManager>>,
}

impl App {
    pub async fn new() -> Result<Self> {
        let exe_tui_config_path = {
            let mut path = env::current_exe()?;
            path.set_file_name("tui.toml");
            path
        };

        let cwd_tui_config = env::current_dir().ok().map(|cwd| cwd.join("tui.toml"));

        let config_path = if let Some(path) = cwd_tui_config {
            path
        } else {
            exe_tui_config_path.clone()
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
}
