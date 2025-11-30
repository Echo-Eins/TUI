use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Row, Table},
    Frame,
};

use crate::app::App;
use crate::utils::format::{create_progress_bar, format_bytes};
use crate::ui::theme::Theme;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let ram_data = app.state.ram_data.read();

    if let Some(data) = ram_data.as_ref() {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);

        if app.state.compact_mode {
            render_compact(f, area, data, &theme);
        } else {
            render_full(f, area, data, &theme);
        }
    } else {
        let block = Block::default()
            .title("RAM Monitor")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red));

        let text = Paragraph::new("Loading RAM data...")
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(text, area);
    }
}

fn render_full(f: &mut Frame, area: Rect, data: &crate::monitors::RamData, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header
            Constraint::Length(3),  // Overall usage
            Constraint::Length(3),  // Committed memory
            Constraint::Length(9),  // Memory breakdown
            Constraint::Min(8),     // Top processes
        ])
        .split(area);

    // Header
    let header = format!(
        "RAM: {} Total  |  Type: {}  |  Speed: {}",
        format_bytes(data.total),
        data.type_name,
        data.speed
    );

    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.ram_color));

    let header_text = Paragraph::new(header)
        .block(header_block)
        .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD));

    f.render_widget(header_text, chunks[0]);

    // Overall usage gauge
    let usage_percent = ((data.used as f64 / data.total as f64) * 100.0) as u16;
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("Memory Usage"))
        .gauge_style(Style::default().fg(theme.ram_color).add_modifier(Modifier::BOLD))
        .percent(usage_percent)
        .label(format!("{}% - {} / {}",
            usage_percent,
            format_bytes(data.used),
            format_bytes(data.total)
        ));

    f.render_widget(gauge, chunks[1]);

    // Committed memory gauge
    let commit_percent = data.commit_percent.min(100.0) as u16;
    let commit_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("Committed Memory"))
        .gauge_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .percent(commit_percent)
        .label(format!("{}% - {} / {} (Physical + Pagefile)",
            commit_percent,
            format_bytes(data.committed),
            format_bytes(data.commit_limit)
        ));

    f.render_widget(commit_gauge, chunks[2]);

    // Memory breakdown
    render_memory_breakdown(f, chunks[3], data, theme);

    // Top processes
    render_top_processes(f, chunks[4], data, theme);
}

fn render_compact(f: &mut Frame, area: Rect, data: &crate::monitors::RamData, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header
            Constraint::Min(5),     // Memory info
        ])
        .split(area);

    // Header
    let header = format!("RAM: {} / {}", format_bytes(data.used), format_bytes(data.total));
    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.ram_color));

    let header_text = Paragraph::new(header)
        .block(header_block)
        .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD));

    f.render_widget(header_text, chunks[0]);

    // Compact memory info
    let usage_percent = ((data.used as f64 / data.total as f64) * 100.0) as f32;
    let commit_percent = data.commit_percent as f32;

    let info_text = vec![
        Line::from(vec![
            Span::raw("Usage:     "),
            Span::styled(
                format!("{}%  {}", usage_percent as u16, create_progress_bar(usage_percent, 20)),
                Style::default().fg(theme.ram_color)
            ),
        ]),
        Line::from(vec![
            Span::raw("Committed: "),
            Span::styled(
                format!("{}%  {}", commit_percent as u16, create_progress_bar(commit_percent, 20)),
                Style::default().fg(Color::Yellow)
            ),
        ]),
    ];

    let info_block = Block::default()
        .borders(Borders::ALL)
        .title("Memory Info");

    let info_para = Paragraph::new(info_text)
        .block(info_block)
        .style(Style::default().fg(Color::White));

    f.render_widget(info_para, chunks[1]);
}

fn render_memory_breakdown(f: &mut Frame, area: Rect, data: &crate::monitors::RamData, theme: &Theme) {
    let breakdown_text = vec![
        Line::from(vec![
            Span::raw("  In Use:     "),
            Span::styled(
                format!("{:>12}  ", format_bytes(data.in_use)),
                Style::default().fg(theme.ram_color).add_modifier(Modifier::BOLD)
            ),
            Span::raw(create_progress_bar(
                ((data.in_use as f64 / data.total as f64) * 100.0) as f32,
                30
            )),
        ]),
        Line::from(vec![
            Span::raw("  Available:  "),
            Span::styled(
                format!("{:>12}  ", format_bytes(data.available)),
                Style::default().fg(Color::Green)
            ),
            Span::raw(create_progress_bar(
                ((data.available as f64 / data.total as f64) * 100.0) as f32,
                30
            )),
        ]),
        Line::from(vec![
            Span::raw("  Cached:     "),
            Span::styled(
                format!("{:>12}  ", format_bytes(data.cached)),
                Style::default().fg(Color::Cyan)
            ),
            Span::raw(create_progress_bar(
                ((data.cached as f64 / data.total as f64) * 100.0) as f32,
                30
            )),
        ]),
        Line::from(vec![
            Span::raw("  Standby:    "),
            Span::styled(
                format!("{:>12}  ", format_bytes(data.standby)),
                Style::default().fg(Color::Blue)
            ),
            Span::raw(create_progress_bar(
                ((data.standby as f64 / data.total as f64) * 100.0) as f32,
                30
            )),
        ]),
        Line::from(vec![
            Span::raw("  Free:       "),
            Span::styled(
                format!("{:>12}  ", format_bytes(data.free)),
                Style::default().fg(Color::Gray)
            ),
            Span::raw(create_progress_bar(
                ((data.free as f64 / data.total as f64) * 100.0) as f32,
                30
            )),
        ]),
        Line::from(vec![
            Span::raw("  Modified:   "),
            Span::styled(
                format!("{:>12}  ", format_bytes(data.modified)),
                Style::default().fg(Color::Yellow)
            ),
            Span::raw(create_progress_bar(
                ((data.modified as f64 / data.total as f64) * 100.0) as f32,
                30
            )),
        ]),
    ];

    let breakdown_block = Block::default()
        .borders(Borders::ALL)
        .title("Memory Breakdown")
        .border_style(Style::default().fg(theme.ram_color));

    let breakdown_para = Paragraph::new(breakdown_text)
        .block(breakdown_block)
        .style(Style::default().fg(Color::White));

    f.render_widget(breakdown_para, area);
}

fn render_top_processes(f: &mut Frame, area: Rect, data: &crate::monitors::RamData, theme: &Theme) {
    let header_cells = ["PID", "Process Name", "Working Set", "Private Bytes"]
        .iter()
        .map(|h| ratatui::widgets::Cell::from(*h).style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1).bottom_margin(0);

    let rows: Vec<Row> = data.top_processes.iter().map(|proc| {
        let cells = vec![
            ratatui::widgets::Cell::from(proc.pid.to_string()),
            ratatui::widgets::Cell::from(proc.name.clone()),
            ratatui::widgets::Cell::from(format_bytes(proc.working_set)),
            ratatui::widgets::Cell::from(format_bytes(proc.private_bytes)),
        ];
        Row::new(cells).height(1)
    }).collect();

    let table = Table::new(rows, [
        Constraint::Length(8),
        Constraint::Min(20),
        Constraint::Length(15),
        Constraint::Length(15),
    ])
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Top Memory Consumers")
                .border_style(Style::default().fg(theme.ram_color))
        )
        .style(Style::default().fg(Color::White));

    f.render_widget(table, area);
}
