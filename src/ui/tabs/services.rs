use ratatui::{layout::Rect, style::{Color, Style}, widgets::{Block, Borders, Paragraph}, Frame};
use crate::app::App;

pub fn render(f: &mut Frame, area: Rect, _app: &App) {
    let block = Block::default().title("Services").borders(Borders::ALL).border_style(Style::default().fg(Color::LightBlue));
    let text = Paragraph::new("Services monitor - Coming soon").block(block);
    f.render_widget(text, area);
}
