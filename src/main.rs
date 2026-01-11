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
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;
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
    init_logging();

    set_console_utf8();

    // Setup terminal with proper error handling
    if let Err(e) = setup_terminal().await {
        eprintln!("Failed to setup terminal: {}", e);
        return Err(e);
    }

    Ok(())
}

fn init_logging() {
    let mut builder = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    );
    builder.format_timestamp_secs();

    let log_path = std::env::var("TUI_PLUS_LOG")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "logs/tui-plus.log".to_string());

    if let Some(parent) = Path::new(&log_path).parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                eprintln!("Failed to create log directory {:?}: {}", parent, err);
            }
        }
    }

    match OpenOptions::new().create(true).append(true).open(&log_path) {
        Ok(file) => {
            builder.target(env_logger::Target::Pipe(Box::new(file)));
        }
        Err(err) => {
            eprintln!("Failed to open log file {}: {}", log_path, err);
        }
    }

    builder.init();
}

#[cfg(windows)]
fn set_console_utf8() {
    use windows_sys::Win32::System::Console::{SetConsoleCP, SetConsoleOutputCP};

    unsafe {
        if SetConsoleOutputCP(65001) == 0 {
            log::warn!("Failed to set console output codepage to UTF-8");
        }
        if SetConsoleCP(65001) == 0 {
            log::warn!("Failed to set console input codepage to UTF-8");
        }
    }
}

#[cfg(not(windows))]
fn set_console_utf8() {}

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
