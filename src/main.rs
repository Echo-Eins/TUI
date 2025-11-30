use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event as CrosstermEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};
use std::io::{self, Write};
use std::sync::Arc;
use tokio::sync::Mutex;  // Use tokio Mutex for async compatibility

mod app;
mod ui;
mod monitors;
mod integrations;
mod events;
mod utils;

use app::App;
use events::{EventHandler, AppEvent};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logger
    env_logger::init();

    // Setup terminal with proper error handling
    if let Err(e) = setup_terminal().await {
        eprintln!("Failed to setup terminal: {}", e);
        return Err(e);
    }

    Ok(())
}

async fn setup_terminal() -> Result<()> {
    enable_raw_mode()?;

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // CRITICAL: Force clear to trigger initial full render
    terminal.clear()?;

    // Create app
    let app = match App::new().await {
        Ok(app) => app,
        Err(e) => {
            // Cleanup terminal before returning error
            cleanup_terminal(&mut terminal)?;
            return Err(e);
        }
    };

    let tick_rate_ms = app.state.config.read().general.refresh_rate_ms;

    // Use tokio::sync::Mutex for proper async support
    let app_state = Arc::new(Mutex::new(app));

    // Create event handler
    let event_handler = EventHandler::new(tick_rate_ms.max(50)); // At least 20fps

    // Run the application
    let res = run_app(&mut terminal, app_state, event_handler).await;

    // Always cleanup terminal
    cleanup_terminal(&mut terminal)?;

    res
}

fn cleanup_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app_state: Arc<Mutex<App>>,
    mut event_handler: EventHandler,
) -> Result<()> {
    // Force initial draw
    {
        let app = app_state.lock().await;
        terminal.draw(|f| {
            ui::render(f, &app);
        })?;
        // Explicit flush
        io::stdout().flush()?;
    }

    let mut needs_clear = false;

    loop {
        // Wait for event
        let event = event_handler.next().await;

        // Check if we need to force clear (after resize)
        if needs_clear {
            terminal.clear()?;
            needs_clear = false;
        }

        // Process event
        let should_continue = match event {
            AppEvent::Input(crossterm_event) => {
                // Check for resize to force full redraw
                if matches!(crossterm_event, CrosstermEvent::Resize(_, _)) {
                    needs_clear = true;
                }

                // Handle event with async lock
                let mut app = app_state.lock().await;
                app.handle_event(crossterm_event).await?
            }
            AppEvent::Tick => true,
        };

        if !should_continue {
            break;
        }

        // Render after each event
        {
            let app = app_state.lock().await;
            terminal.draw(|f| {
                ui::render(f, &app);
            })?;

            // Explicit flush to ensure immediate display
            io::stdout().flush()?;
        }
    }

    Ok(())
}