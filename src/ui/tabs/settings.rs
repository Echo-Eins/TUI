use ratatui::{layout::Rect, style::{Color, Style}, widgets::{Block, Borders, Paragraph}, Frame};
use crate::app::App;

pub fn render(f: &mut Frame, area: Rect, _app: &App) {
    let block = Block::default().title("Settings").borders(Borders::ALL).border_style(Style::default().fg(Color::Gray));
    let text = Paragraph::new("Settings - Coming soon").block(block);
    f.render_widget(text, area);
}
