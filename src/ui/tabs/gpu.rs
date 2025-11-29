use ratatui::{layout::Rect, style::{Color, Style}, widgets::{Block, Borders, Paragraph}, Frame};
use crate::app::App;

pub fn render(f: &mut Frame, area: Rect, _app: &App) {
    let block = Block::default().title("GPU Monitor").borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan));
    let text = Paragraph::new("GPU monitor - Coming soon").block(block);
    f.render_widget(text, area);
}
