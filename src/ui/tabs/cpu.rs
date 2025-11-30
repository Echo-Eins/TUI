use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Row, Table},
    Frame,
};

use crate::app::App;
use crate::ui::theme::Theme;
use crate::utils::format::{create_progress_bar, format_bytes, format_percentage};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let cpu_data = app.state.cpu_data.read();
    let cpu_error = app.state.cpu_error.read();

    if let Some(message) = cpu_error.as_ref() {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);
        let block = Block::default()
            .title("CPU Monitor")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.warning_color));

        let text = Paragraph::new(format!("CPU monitor unavailable: {}", message))
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(text, area);
    } else if let Some(data) = cpu_data.as_ref() {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);

        if app.state.compact_mode {
            render_compact(f, area, data, &theme);
        } else {
            render_full(f, area, data, &theme);
        }
    } else {
        let block = Block::default()
            .title("CPU Monitor")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red));

        let text = Paragraph::new("Loading CPU data...")
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(text, area);
    }
}

fn render_full(f: &mut Frame, area: Rect, data: &crate::monitors::CpuData, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Length(3), // Overall usage
            Constraint::Min(8),    // Core usage
            Constraint::Length(5), // Frequency & Power
            Constraint::Length(9), // Top Processes
        ])
        .split(area);

    // Header
    let header = format!(
        "CPU: {}{}",
        data.name,
        if let Some(temp) = data.temperature {
            format!("  Temp: {:.1}°C", temp)
        } else {
            String::new()
        }
    );

    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.cpu_color));

    let header_text = Paragraph::new(header).block(header_block).style(
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );

    f.render_widget(header_text, chunks[0]);

    // Overall usage
    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Overall Usage"),
        )
        .gauge_style(
            Style::default()
                .fg(theme.cpu_color)
                .add_modifier(Modifier::BOLD),
        )
        .percent(data.overall_usage as u16)
        .label(format!(
            "{}% - Cores: {}/{}",
            data.overall_usage as u16, data.core_count, data.thread_count
        ));

    f.render_widget(gauge, chunks[1]);

    // Core usage
    let core_text: Vec<Line> = data
        .core_usage
        .chunks(2)
        .map(|chunk| {
            let spans: Vec<Span> = chunk
                .iter()
                .map(|core| {
                    let bar = create_progress_bar(core.usage, 15);
                    Span::raw(format!(
                        "  Core {:2} [{}] {:>5}     ",
                        core.core_id,
                        bar,
                        format_percentage(core.usage)
                    ))
                })
                .collect();
            Line::from(spans)
        })
        .collect();

    let core_block = Block::default()
        .title("Core Usage")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.cpu_color));

    let core_paragraph = Paragraph::new(core_text)
        .block(core_block)
        .style(Style::default().fg(Color::White));

    f.render_widget(core_paragraph, chunks[2]);

    // Frequency & Power
    let freq_text = vec![
        Line::from(vec![
            Span::raw("  Avg Frequency: "),
            Span::styled(
                format!("{:.2} GHz", data.frequency.avg_frequency),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  │  Max Frequency: "),
            Span::styled(
                format!("{:.2} GHz", data.frequency.max_frequency),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("  Base Clock: "),
            Span::styled(
                format!("{:.2} GHz", data.frequency.base_clock),
                Style::default().fg(Color::White),
            ),
            Span::raw("  │  Power: "),
            Span::styled(
                format!(
                    "{:.0}W/{:.0}W",
                    data.power.current_power, data.power.max_power
                ),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    let freq_block = Block::default()
        .title("Frequency & Power")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.cpu_color));

    let freq_paragraph = Paragraph::new(freq_text)
        .block(freq_block)
        .style(Style::default().fg(Color::White));

    f.render_widget(freq_paragraph, chunks[3]);

    // Top Processes
    let rows: Vec<Row> = data
        .top_processes
        .iter()
        .map(|p| {
            Row::new(vec![
                format!("{}", p.pid),
                p.name.clone(),
                format!("{:.1}%", p.cpu_usage),
                format!("{}", p.threads),
                format_bytes(p.memory),
            ])
            .style(Style::default().fg(Color::White))
        })
        .collect();

    let table = Table::new(
        rows,
        &[
            Constraint::Length(8),
            Constraint::Min(20),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(12),
        ],
    )
    .header(
        Row::new(vec!["PID", "Name", "CPU%", "Threads", "Memory"]).style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .title("Top Processes")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.cpu_color)),
    );

    f.render_widget(table, chunks[4]);
}

fn render_compact(f: &mut Frame, area: Rect, data: &crate::monitors::CpuData, theme: &Theme) {
    let compact_text = format!(
        "CPU: {} │ {}% │ {:.2} GHz │ {}°C │ {:.0}W/{:.0}W",
        data.name.split_whitespace().next().unwrap_or("CPU"),
        data.overall_usage as u16,
        data.frequency.avg_frequency,
        data.temperature.unwrap_or(0.0),
        data.power.current_power,
        data.power.max_power
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.cpu_color));

    let paragraph = Paragraph::new(compact_text)
        .block(block)
        .style(Style::default().fg(theme.foreground));

    f.render_widget(paragraph, area);
}
