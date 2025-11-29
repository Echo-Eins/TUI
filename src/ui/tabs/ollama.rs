use ratatui::{layout::Rect, style::{Color, Style}, widgets::{Block, Borders, Paragraph}, Frame};
use crate::app::App;

pub fn render(f: &mut Frame, area: Rect, _app: &App) {
    let block = Block::default().title("Ollama Manager").borders(Borders::ALL).border_style(Style::default().fg(Color::Magenta));
    let text = Paragraph::new("Ollama manager - Coming soon").block(block);
    f.render_widget(text, area);
}
