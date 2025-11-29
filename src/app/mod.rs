pub mod state;
pub mod config;
pub mod tabs;

pub use state::AppState;
pub use config::{Config, ConfigManager};
pub use tabs::{TabType, TabManager};

use anyhow::Result;
use crossterm::event::Event as CrosstermEvent;

pub struct App {
    pub state: AppState,
}

impl App {
    pub async fn new() -> Result<Self> {
        let config = Config::load("config.toml")?;
        let state = AppState::new(config).await?;
        Ok(Self { state })
    }

    pub async fn handle_event(&mut self, event: CrosstermEvent) -> Result<bool> {
        self.state.handle_event(event).await
    }
}
