use ratatui::{layout::Rect, style::{Color, Style}, widgets::{Block, Borders, Paragraph}, Frame};
use crate::app::App;

pub fn render(f: &mut Frame, area: Rect, _app: &App) {
    let block = Block::default().title("Disk Monitor").borders(Borders::ALL).border_style(Style::default().fg(Color::Green));
    let text = Paragraph::new("Disk monitor - Coming soon").block(block);
    f.render_widget(text, area);
}
