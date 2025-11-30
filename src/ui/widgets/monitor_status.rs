use chrono::{DateTime, Local};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::state::MonitorStatus;

pub fn render_monitor_status(
    f: &mut Frame,
    area: ratatui::layout::Rect,
    title: &str,
    status: &MonitorStatus,
    last_updated: Option<DateTime<Local>>,
) {
    let (message, color) = match status {
        MonitorStatus::Loading => ("Loading data...", Color::Yellow),
        MonitorStatus::Ready => ("Data unavailable", Color::Gray),
        MonitorStatus::Error(err) => (err.as_str(), Color::Red),
    };

    let last_updated_text = last_updated
        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "never".to_string());

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color));

    let paragraph = Paragraph::new(vec![
        Line::from(Span::styled(
            message,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::raw("Last updated: "),
            Span::styled(
                last_updated_text,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]),
    ])
    .block(block)
    .style(Style::default().fg(Color::White));

    f.render_widget(paragraph, area);
}
