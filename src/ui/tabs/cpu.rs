use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Row, Table},
    Frame,
};

use crate::app::state::CpuProcessSortColumn;
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
            render_full(f, area, data, &theme, app);
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

fn render_full(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::CpuData,
    theme: &Theme,
    app: &App,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Length(3), // Overall usage
            Constraint::Min(8),    // Core usage
            Constraint::Length(5), // Frequency & Power
            Constraint::Min(12),   // Processes (sortable, scrollable)
        ])
        .split(area);

    // Header
    let header = format!(
        "CPU: {}  |  {} cores / {} threads{}",
        data.name,
        data.core_count,
        data.thread_count,
        if let Some(temp) = data.temperature {
            format!("  |  Temp: {:.1}\u{00b0}C", temp)
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
        .percent(data.overall_usage.clamp(0.0, 100.0) as u16)
        .label(format!(
            "{}% - Cores: {}/{}",
            data.overall_usage as u16, data.core_count, data.thread_count
        ));

    f.render_widget(gauge, chunks[1]);

    // Core/Thread usage
    let is_hyperthreaded = data.thread_count > data.core_count;
    let core_text: Vec<Line> = data
        .core_usage
        .chunks(2)
        .map(|chunk| {
            let spans: Vec<Span> = chunk
                .iter()
                .map(|core| {
                    let bar = create_progress_bar(core.usage, 15);
                    let label = if is_hyperthreaded { "Thread" } else { "Core" };
                    Span::raw(format!(
                        "  {} {:2} [{}] {:>5}     ",
                        label,
                        core.core_id,
                        bar,
                        format_percentage(core.usage)
                    ))
                })
                .collect();
            Line::from(spans)
        })
        .collect();

    let core_title = if is_hyperthreaded {
        format!(
            "Thread Usage ({} cores / {} threads)",
            data.core_count, data.thread_count
        )
    } else {
        format!("Core Usage ({} cores)", data.core_count)
    };
    let core_block = Block::default()
        .title(core_title)
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
            Span::raw("  \u{2502}  Max Frequency: "),
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
            Span::raw("  \u{2502}  Power: "),
            Span::styled(
                format!(
                    "{:.0}W/{:.0}W",
                    data.power.current_power, data.power.max_power
                ),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ),
            if data.frequency.boost_active {
                Span::styled(
                    "  [BOOST]",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::raw("")
            },
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

    // Processes - sortable, scrollable
    let sort_col = app.state.cpu_state.sort_column;
    let sort_asc = app.state.cpu_state.sort_ascending;
    let selected_idx = app.state.cpu_state.selected_index;

    // Sort processes
    let mut processes = data.top_processes.clone();
    processes.sort_by(|a, b| {
        let ord = match sort_col {
            CpuProcessSortColumn::Pid => a.pid.cmp(&b.pid),
            CpuProcessSortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            CpuProcessSortColumn::Cpu => a
                .cpu_usage
                .partial_cmp(&b.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal),
            CpuProcessSortColumn::Memory => a.memory.cmp(&b.memory),
            CpuProcessSortColumn::Threads => a.threads.cmp(&b.threads),
        };
        if sort_asc {
            ord
        } else {
            ord.reverse()
        }
    });

    // Calculate visible area (subtract 3 for block border + header row)
    let visible_rows = chunks[4].height.saturating_sub(3) as usize;
    let scroll_offset = if selected_idx >= visible_rows {
        selected_idx - visible_rows + 1
    } else {
        0
    };

    let rows: Vec<Row> = processes
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_rows.max(1))
        .map(|(idx, p)| {
            let style = if idx == selected_idx {
                Style::default().fg(Color::Black).bg(theme.cpu_color)
            } else {
                Style::default().fg(Color::White)
            };
            Row::new(vec![
                format!("{}", p.pid),
                p.name.clone(),
                format!("{:.1}%", p.cpu_usage),
                format!("{}", p.threads),
                format_bytes(p.memory),
            ])
            .style(style)
        })
        .collect();

    // Build header with sort indicators
    let sort_indicator = |col: CpuProcessSortColumn| -> &str {
        if sort_col == col {
            if sort_asc {
                " \u{25b2}"
            } else {
                " \u{25bc}"
            }
        } else {
            ""
        }
    };

    let header_row = Row::new(vec![
        format!("PID(p){}", sort_indicator(CpuProcessSortColumn::Pid)),
        format!("Name(n){}", sort_indicator(CpuProcessSortColumn::Name)),
        format!("CPU%(c){}", sort_indicator(CpuProcessSortColumn::Cpu)),
        format!(
            "Threads(t){}",
            sort_indicator(CpuProcessSortColumn::Threads)
        ),
        format!("Memory(m){}", sort_indicator(CpuProcessSortColumn::Memory)),
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    let process_title = format!(
        "Processes [{}/{}]  \u{2191}\u{2193}:Navigate  p/n/c/t/m:Sort",
        processes.len(),
        processes.len()
    );

    let table = Table::new(
        rows,
        &[
            Constraint::Length(8),
            Constraint::Min(20),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Length(12),
        ],
    )
    .header(header_row)
    .block(
        Block::default()
            .title(process_title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.cpu_color)),
    );

    f.render_widget(table, chunks[4]);
}

fn render_compact(f: &mut Frame, area: Rect, data: &crate::monitors::CpuData, theme: &Theme) {
    let compact_text = format!(
        "CPU: {} \u{2502} {}% \u{2502} {:.2} GHz \u{2502} {}°C \u{2502} {:.0}W/{:.0}W",
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
