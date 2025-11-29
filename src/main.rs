use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use std::io;
use tokio::sync::RwLock;
use std::sync::Arc;

mod app;
mod ui;
mod monitors;
mod integrations;
mod events;
mod utils;

use app::{App, AppState};
use events::EventHandler;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logger
    env_logger::init();

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app
    let app = App::new().await?;
    let app_state = Arc::new(RwLock::new(app));

    // Create event handler
    let event_handler = EventHandler::new();

    // Run the application
    let res = run_app(&mut terminal, app_state.clone(), event_handler).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("Error: {:?}", err);
    }

    Ok(())
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app_state: Arc<RwLock<App>>,
    mut event_handler: EventHandler,
) -> Result<()> {
    loop {
        // Render UI
        {
            let app = app_state.read().await;
            terminal.draw(|f| {
                ui::render(f, &app);
            })?;
        }

        // Handle events
        if let Some(event) = event_handler.next().await {
            let mut app = app_state.write().await;
            if !app.handle_event(event).await? {
                break;
            }
        }
    }

    Ok(())
}
