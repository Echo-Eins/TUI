use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Row, Table},
    Frame,
};

use crate::app::App;
use crate::app::state::GpuProcessSortColumn;
use crate::monitors::gpu::GpuProcessInfo;
use crate::ui::theme::Theme;
use crate::utils::format::format_bytes;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let gpu_data = app.state.gpu_data.read();
    let gpu_error = app.state.gpu_error.read();

    if let Some(message) = gpu_error.as_ref() {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);
        let block = Block::default()
            .title("GPU Monitor")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.warning_color));

        let text = Paragraph::new(format!("GPU monitor unavailable: {}", message))
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(text, area);
    } else if let Some(data) = gpu_data.as_ref() {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);

        if app.state.compact_mode {
            render_compact(f, area, data, &theme);
        } else {
            render_full(f, area, data, app, &theme);
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

fn render_full(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::GpuData,
    app: &App,
    theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Length(3), // Overall usage
            Constraint::Length(7), // Performance metrics
            Constraint::Length(5), // VRAM usage
            Constraint::Length(7), // GPU Processes
        ])
        .split(area);

    // Header
    let header = format!(
        "GPU {}: {}  Bus: {}  Driver: {}  CUDA: {}  Temp: {:.1}°C",
        data.gpu_index,
        data.name,
        if data.bus_id.is_empty() { "N/A" } else { &data.bus_id },
        data.driver_version,
        if data.cuda_version.is_empty() { "N/A" } else { &data.cuda_version },
        data.temperature
    );

    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.gpu_color));

    let header_text = Paragraph::new(header).block(header_block).style(
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );

    f.render_widget(header_text, chunks[0]);

    // Overall GPU usage
    let utilization_pct = data.utilization.clamp(0.0, 100.0) as u16;
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("GPU Usage"))
        .gauge_style(
            Style::default()
                .fg(theme.gpu_color)
                .add_modifier(Modifier::BOLD),
        )
        .percent(utilization_pct)
        .label(format!("{}%", utilization_pct));

    f.render_widget(gauge, chunks[1]);

    // Performance metrics
    let perf_text = vec![
        Line::from(vec![
            Span::raw("  GPU Clock: "),
            Span::styled(
                format!("{} MHz", data.clock_speed),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  │  Memory Clock: "),
            Span::styled(
                format!("{} MHz", data.memory_clock),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("  Power Draw: "),
            Span::styled(
                format!("{:.0}W/{:.0}W", data.power_usage, data.power_limit),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  │  Fan Speed: "),
            Span::styled(
                if data.fan_speed < 0.0 {
                    "-".to_string()
                } else {
                    format!("{:.0}%", data.fan_speed)
                },
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("  Temperature: "),
            Span::styled(
                format!("{:.1}°C", data.temperature),
                Style::default().fg(theme.get_temp_color(data.temperature)),
            ),
            Span::raw("  │  Utilization: "),
            Span::styled(
                format!("{:.0}%", data.utilization),
                Style::default()
                    .fg(theme.gpu_color)
                    .add_modifier(Modifier::BOLD),
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
        ((data.memory_used as f64 / data.memory_total as f64) * 100.0)
            .min(100.0) as u16
    } else {
        0
    };

    let vram_gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(format!(
            "VRAM Usage ({} Total)",
            format_bytes(data.memory_total)
        )))
        .gauge_style(
            Style::default()
                .fg(theme.success_color)
                .add_modifier(Modifier::BOLD),
        )
        .percent(vram_used_pct)
        .label(format!(
            "{} / {} ({}%)",
            format_bytes(data.memory_used),
            format_bytes(data.memory_total),
            vram_used_pct
        ));

    f.render_widget(vram_gauge, chunks[3]);

    // GPU Processes
    let mut processes = data.processes.clone();
    sort_gpu_processes(
        &mut processes,
        app.state.gpu_state.sort_column,
        app.state.gpu_state.sort_ascending,
    );
    if !processes.is_empty() {
        let selected_index = app
            .state
            .gpu_state
            .selected_index
            .min(processes.len().saturating_sub(1));
        let hotkeys_height = if chunks[4].height > 2 { 1 } else { 0 };
        let visible_rows = chunks[4]
            .height
            .saturating_sub(2 + 1 + hotkeys_height)
            .max(1) as usize;
        let scroll_offset = if selected_index >= visible_rows {
            selected_index - (visible_rows - 1)
        } else {
            0
        };

        let rows: Vec<Row> = processes
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(visible_rows)
            .map(|(i, p)| {
                let is_selected = i == selected_index;
                let style = if is_selected {
                    Style::default().fg(Color::Black).bg(Color::Green)
                } else {
                    Style::default().fg(Color::White)
                };
                let gpu_text = if p.gpu_usage < 0.0 {
                    "-".to_string()
                } else {
                    format!("{:.1}%", p.gpu_usage)
                };
                Row::new(vec![
                    format!("{}", i + 1),
                    format!("{}", p.pid),
                    p.process_type.clone(),
                    p.name.clone(),
                    gpu_text,
                    format_bytes(p.vram),
                ])
                .style(style)
            })
            .collect();

        let sort_indicator = if app.state.gpu_state.sort_ascending {
            "↑"
        } else {
            "↓"
        };
        let header = Row::new(vec![
            "№".to_string(),
            if app.state.gpu_state.sort_column == GpuProcessSortColumn::Pid {
                format!("PID {sort_indicator}")
            } else {
                "PID".to_string()
            },
            if app.state.gpu_state.sort_column == GpuProcessSortColumn::Type {
                format!("Type {sort_indicator}")
            } else {
                "Type".to_string()
            },
            if app.state.gpu_state.sort_column == GpuProcessSortColumn::Name {
                format!("Name {sort_indicator}")
            } else {
                "Name".to_string()
            },
            if app.state.gpu_state.sort_column == GpuProcessSortColumn::Gpu {
                format!("GPU% {sort_indicator}")
            } else {
                "GPU%".to_string()
            },
            if app.state.gpu_state.sort_column == GpuProcessSortColumn::Memory {
                format!("VRAM {sort_indicator}")
            } else {
                "VRAM".to_string()
            },
        ])
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));

        let table = Table::new(
            rows,
            &[
                Constraint::Length(4),
                Constraint::Length(8),
                Constraint::Length(10),
                Constraint::Min(18),
                Constraint::Length(8),
                Constraint::Length(12),
            ],
        )
        .header(header)
        .block(
            Block::default()
                .title("GPU Processes")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.gpu_color)),
        )
        .column_spacing(1)
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Green));

        f.render_widget(table, chunks[4]);

        let hotkeys = vec![Line::from(vec![
            Span::styled("↑/↓", Style::default().fg(Color::Cyan)),
            Span::raw(": Navigate  "),
            Span::styled("p/n/g/m/t", Style::default().fg(Color::Cyan)),
            Span::raw(": Sort by PID/Name/GPU/Memory/Type  "),
            Span::styled("PgUp/PgDn", Style::default().fg(Color::Cyan)),
            Span::raw(": Page Up/Down"),
        ])];
        if hotkeys_height > 0 {
            let hotkeys_area = Rect {
                x: chunks[4].x + 2,
                y: chunks[4].y + chunks[4].height - 2,
                width: chunks[4].width.saturating_sub(4),
                height: 1,
            };
            let hotkeys_paragraph = Paragraph::new(hotkeys);
            f.render_widget(hotkeys_paragraph, hotkeys_area);
        }
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

fn sort_gpu_processes(
    processes: &mut Vec<GpuProcessInfo>,
    column: GpuProcessSortColumn,
    ascending: bool,
) {
    processes.sort_by(|a, b| {
        let cmp = match column {
            GpuProcessSortColumn::Pid => a.pid.cmp(&b.pid),
            GpuProcessSortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            GpuProcessSortColumn::Gpu => a
                .gpu_usage
                .partial_cmp(&b.gpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal),
            GpuProcessSortColumn::Memory => a.vram.cmp(&b.vram),
            GpuProcessSortColumn::Type => a
                .process_type
                .to_lowercase()
                .cmp(&b.process_type.to_lowercase()),
        };

        if ascending {
            cmp
        } else {
            cmp.reverse()
        }
    });
}

fn render_compact(f: &mut Frame, area: Rect, data: &crate::monitors::GpuData, theme: &Theme) {
    let compact_text = format!(
        "GPU: {} │ {}% │ {}/{} │ {:.1}°C │ {:.0}W/{:.0}W",
        data.name
            .split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" "),
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
