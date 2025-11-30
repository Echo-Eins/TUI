use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Row, Table},
    Frame,
};

use crate::app::App;
use crate::utils::format::format_bytes;
use crate::ui::theme::Theme;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let gpu_data = app.state.gpu_data.read();

    if let Some(data) = gpu_data.as_ref() {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);

        if app.state.compact_mode {
            render_compact(f, area, data, &theme);
        } else {
            render_full(f, area, data, &theme);
        }
    } else {
        let block = Block::default()
            .title("GPU Monitor")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red));

        let text = Paragraph::new("Loading GPU data...")
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(text, area);
    }
}

fn render_full(f: &mut Frame, area: Rect, data: &crate::monitors::GpuData, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header
            Constraint::Length(3),  // Overall usage
            Constraint::Length(7),  // Performance metrics
            Constraint::Length(5),  // VRAM usage
            Constraint::Length(7),  // GPU Processes
        ])
        .split(area);

    // Header
    let header = format!(
        "GPU: {}  Driver: {}  Temp: {:.1}°C",
        data.name,
        data.driver_version,
        data.temperature
    );

    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.gpu_color));

    let header_text = Paragraph::new(header)
        .block(header_block)
        .style(Style::default().fg(Color::White).add_modifier(Modifier::BOLD));

    f.render_widget(header_text, chunks[0]);

    // Overall GPU usage
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("GPU Usage"))
        .gauge_style(Style::default().fg(theme.gpu_color).add_modifier(Modifier::BOLD))
        .percent(data.utilization as u16)
        .label(format!("{}%", data.utilization as u16));

    f.render_widget(gauge, chunks[1]);

    // Performance metrics
    let perf_text = vec![
        Line::from(vec![
            Span::raw("  GPU Clock: "),
            Span::styled(
                format!("{} MHz", data.clock_speed),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            ),
            Span::raw("  │  Memory Clock: "),
            Span::styled(
                format!("{} MHz", data.memory_clock),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            ),
        ]),
        Line::from(vec![
            Span::raw("  Power Draw: "),
            Span::styled(
                format!("{:.0}W/{:.0}W", data.power_usage, data.power_limit),
                Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)
            ),
            Span::raw("  │  Fan Speed: "),
            Span::styled(
                format!("{:.0}%", data.fan_speed),
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
            ),
        ]),
        Line::from(vec![
            Span::raw("  Temperature: "),
            Span::styled(
                format!("{:.1}°C", data.temperature),
                Style::default().fg(theme.get_temp_color(data.temperature))
            ),
            Span::raw("  │  Utilization: "),
            Span::styled(
                format!("{:.0}%", data.utilization),
                Style::default().fg(theme.gpu_color).add_modifier(Modifier::BOLD)
            ),
        ]),
    ];

    let perf_block = Block::default()
        .title("Performance Metrics")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.gpu_color));

    let perf_paragraph = Paragraph::new(perf_text)
        .block(perf_block)
        .style(Style::default().fg(Color::White));

    f.render_widget(perf_paragraph, chunks[2]);

    // VRAM Usage
    let vram_used_pct = if data.memory_total > 0 {
        (data.memory_used as f64 / data.memory_total as f64 * 100.0) as u16
    } else {
        0
    };

    let vram_gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("VRAM Usage ({} Total)", format_bytes(data.memory_total)))
        )
        .gauge_style(Style::default().fg(theme.success_color).add_modifier(Modifier::BOLD))
        .percent(vram_used_pct)
        .label(format!(
            "{} / {} ({}%)",
            format_bytes(data.memory_used),
            format_bytes(data.memory_total),
            vram_used_pct
        ));

    f.render_widget(vram_gauge, chunks[3]);

    // GPU Processes
    if !data.processes.is_empty() {
        let rows: Vec<Row> = data.processes
            .iter()
            .map(|p| {
                Row::new(vec![
                    format!("{}", p.pid),
                    p.name.clone(),
                    format!("{:.1}%", p.gpu_usage),
                    format_bytes(p.vram),
                    p.process_type.clone(),
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
                Constraint::Length(12),
                Constraint::Length(18),
            ],
        )
        .header(
            Row::new(vec!["PID", "Name", "GPU%", "VRAM", "Type"])
                .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        )
        .block(
            Block::default()
                .title("GPU Processes")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.gpu_color))
        );

        f.render_widget(table, chunks[4]);
    } else {
        let block = Block::default()
            .title("GPU Processes")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.gpu_color));

        let text = Paragraph::new("No GPU processes running")
            .block(block)
            .style(Style::default().fg(Color::Gray));

        f.render_widget(text, chunks[4]);
    }
}

fn render_compact(f: &mut Frame, area: Rect, data: &crate::monitors::GpuData, theme: &Theme) {
    let compact_text = format!(
        "GPU: {} │ {}% │ {}/{} │ {:.1}°C │ {:.0}W/{:.0}W",
        data.name.split_whitespace().take(2).collect::<Vec<_>>().join(" "),
        data.utilization as u16,
        format_bytes(data.memory_used),
        format_bytes(data.memory_total),
        data.temperature,
        data.power_usage,
        data.power_limit
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.gpu_color));

    let paragraph = Paragraph::new(compact_text)
        .block(block)
        .style(Style::default().fg(theme.foreground));

    f.render_widget(paragraph, area);
}
