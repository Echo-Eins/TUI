use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Stdout};

#[path = "../app/mod.rs"]
mod app;
#[path = "../events/mod.rs"]
mod events;
#[path = "../integrations/mod.rs"]
mod integrations;
#[path = "../monitors/mod.rs"]
mod monitors;
#[path = "../ui/mod.rs"]
mod ui;
#[path = "../utils/mod.rs"]
mod utils;

use app::App;
use events::{AppEvent, EventHandler};

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        Ok(Self { terminal })
    }

    fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        );
        let _ = self.terminal.show_cursor();
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut app = App::new().await?;
    let mut terminal = TerminalGuard::new()?;
    let mut events = EventHandler::new(250);

    loop {
        terminal
            .terminal_mut()
            .draw(|frame| ui::render(frame, &app))?;

        match events.next().await {
            AppEvent::Input(event) => {
                if let Event::Key(_) | Event::Mouse(_) | Event::Resize(_, _) = event {
                    if !app.handle_event(event).await? {
                        break;
                    }
                }
            }
            AppEvent::Tick => {
                app.state.apply_config_updates(app.config_manager.as_deref());
            }
        }
    }

    Ok(())
}
