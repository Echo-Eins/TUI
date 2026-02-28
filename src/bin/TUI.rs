use anyhow::Result;
use cardputer_remote::{
    app::App,
    events::{AppEvent, EventHandler},
    ui,
};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Stdout};

struct TerminalGuard {
    stdout: Stdout,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        Ok(Self { stdout })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.stdout, LeaveAlternateScreen, DisableMouseCapture);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let _terminal_guard = TerminalGuard::enter()?;

    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut app = App::new().await?;
    let mut events = EventHandler::new(200);

    loop {
        terminal.draw(|frame| ui::render(frame, &mut app))?;

        match events.next().await {
            AppEvent::Input(event) => {
                if !app.handle_event(event).await? {
                    break;
                }
            }
            AppEvent::Tick => {
                // Poll async updates (diagnostics results, etc.) on every tick
                app.tick();
            }
        }
    }

    Ok(())
}
