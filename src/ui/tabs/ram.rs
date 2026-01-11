use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Row, Table},
    Frame,
};

use crate::app::App;
use crate::app::state::{RamPanelFocus, RamProcessSortColumn};
use crate::monitors::ram::ProcessMemoryInfo;
use crate::ui::theme::Theme;
use crate::utils::format::{create_progress_bar, format_bytes};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let ram_data = app.state.ram_data.read();
    let ram_error = app.state.ram_error.read();

    if let Some(message) = ram_error.as_ref() {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);
        let block = Block::default()
            .title("RAM Monitor")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.warning_color));

        let text = Paragraph::new(format!("RAM monitor unavailable: {}", message))
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(text, area);
    } else if let Some(data) = ram_data.as_ref() {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);

        if app.state.compact_mode {
            render_compact(f, area, data, &theme);
        } else {
            render_full(f, area, data, app, &theme);
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

fn render_full(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::RamData,
    app: &App,
    theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Length(3), // Overall usage
            Constraint::Length(3), // Committed memory
            Constraint::Length(3), // Pagefile gauge
            Constraint::Length(9), // Memory breakdown
            Constraint::Min(8),    // Top processes
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

    let header_text = Paragraph::new(header).block(header_block).style(
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );

    f.render_widget(header_text, chunks[0]);

    // Overall usage gauge
    let usage_percent = if data.total > 0 {
        ((data.used as f64 / data.total as f64) * 100.0).min(100.0) as u16
    } else {
        0
    };
    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("Memory Usage"))
        .gauge_style(
            Style::default()
                .fg(theme.ram_color)
                .add_modifier(Modifier::BOLD),
        )
        .percent(usage_percent)
        .label(format!(
            "{}% - {} / {}",
            usage_percent,
            format_bytes(data.used),
            format_bytes(data.total)
        ));

    f.render_widget(gauge, chunks[1]);

    // Committed memory gauge
    let commit_percent = data.commit_percent.min(100.0) as u16;
    let commit_gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Committed Memory"),
        )
        .gauge_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .percent(commit_percent)
        .label(format!(
            "{}% - {} / {} (Physical + Pagefile)",
            commit_percent,
            format_bytes(data.committed),
            format_bytes(data.commit_limit)
        ));

    f.render_widget(commit_gauge, chunks[2]);

    // Pagefile gauge
    render_pagefile_gauge(f, chunks[3], data, theme);

    // Memory breakdown
    let breakdown_focused = app.state.ram_state.focused_panel == RamPanelFocus::Breakdown;
    render_memory_breakdown(f, chunks[4], data, theme, breakdown_focused);

    // Top processes
    let processes_focused = app.state.ram_state.focused_panel == RamPanelFocus::TopProcesses;
    render_top_processes(f, chunks[5], data, app, theme, processes_focused);
}

fn render_compact(f: &mut Frame, area: Rect, data: &crate::monitors::RamData, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(5),    // Memory info
        ])
        .split(area);

    // Header
    let header = format!(
        "RAM: {} / {}",
        format_bytes(data.used),
        format_bytes(data.total)
    );
    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.ram_color));

    let header_text = Paragraph::new(header).block(header_block).style(
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );

    f.render_widget(header_text, chunks[0]);

    // Compact memory info
    let usage_percent = if data.total > 0 {
        ((data.used as f64 / data.total as f64) * 100.0).min(100.0) as f32
    } else {
        0.0
    };
    let commit_percent = data.commit_percent as f32;

    let mut info_text = vec![
        Line::from(vec![
            Span::raw("Usage:     "),
            Span::styled(
                format!(
                    "{}%  {}",
                    usage_percent as u16,
                    create_progress_bar(usage_percent, 20)
                ),
                Style::default().fg(theme.ram_color),
            ),
        ]),
        Line::from(vec![
            Span::raw("Committed: "),
            Span::styled(
                format!(
                    "{}%  {}",
                    commit_percent as u16,
                    create_progress_bar(commit_percent, 20)
                ),
                Style::default().fg(Color::Yellow),
            ),
        ]),
    ];

    // Add pagefile info if configured
    if data.total_pagefile_size > 0 {
        let pagefile_percent = if data.total_pagefile_size > 0 {
            ((data.total_pagefile_used as f64 / data.total_pagefile_size as f64) * 100.0) as f32
        } else {
            0.0
        };

        info_text.push(Line::from(vec![
            Span::raw("Pagefile:  "),
            Span::styled(
                format!(
                    "{}%  {}",
                    pagefile_percent as u16,
                    create_progress_bar(pagefile_percent, 20)
                ),
                Style::default().fg(Color::Magenta),
            ),
        ]));
    }

    let info_block = Block::default().borders(Borders::ALL).title("Memory Info");

    let info_para = Paragraph::new(info_text)
        .block(info_block)
        .style(Style::default().fg(Color::White));

    f.render_widget(info_para, chunks[1]);
}

fn render_memory_breakdown(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::RamData,
    theme: &Theme,
    focused: bool,
) {
    let mut breakdown_text = vec![
        Line::from(vec![
            Span::raw("  In Use:     "),
            Span::styled(
                format!("{:>12}  ", format_bytes(data.in_use)),
                Style::default()
                    .fg(theme.ram_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(create_progress_bar(
                ((data.in_use as f64 / data.total as f64) * 100.0) as f32,
                30,
            )),
        ]),
        Line::from(vec![
            Span::raw("  Available:  "),
            Span::styled(
                format!("{:>12}  ", format_bytes(data.available)),
                Style::default().fg(Color::Green),
            ),
            Span::raw(create_progress_bar(
                ((data.available as f64 / data.total as f64) * 100.0) as f32,
                30,
            )),
        ]),
        Line::from(vec![
            Span::raw("  Cached:     "),
            Span::styled(
                format!("{:>12}  ", format_bytes(data.cached)),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw(create_progress_bar(
                ((data.cached as f64 / data.total as f64) * 100.0) as f32,
                30,
            )),
        ]),
        Line::from(vec![
            Span::raw("  Standby:    "),
            Span::styled(
                format!("{:>12}  ", format_bytes(data.standby)),
                Style::default().fg(Color::Blue),
            ),
            Span::raw(create_progress_bar(
                ((data.standby as f64 / data.total as f64) * 100.0) as f32,
                30,
            )),
        ]),
        Line::from(vec![
            Span::raw("  Modified:   "),
            Span::styled(
                format!("{:>12}  ", format_bytes(data.modified)),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw(create_progress_bar(
                ((data.modified as f64 / data.total as f64) * 100.0) as f32,
                30,
            )),
        ]),
    ];

    // Add pagefile details if configured
    if !data.pagefiles.is_empty() {
        breakdown_text.push(Line::from("")); // Empty line for spacing
        for (i, pf) in data.pagefiles.iter().enumerate() {
            let pf_name = if data.pagefiles.len() > 1 {
                format!("  PF{}:       ", i + 1)
            } else {
                "  Pagefile:  ".to_string()
            };

            breakdown_text.push(Line::from(vec![
                Span::raw(pf_name),
                Span::styled(
                    format!("{:>12}  ", format_bytes(pf.current_usage)),
                    Style::default().fg(Color::Magenta),
                ),
                Span::raw(create_progress_bar(pf.usage_percent as f32, 30)),
                Span::styled(
                    format!(" / {}", format_bytes(pf.total_size)),
                    Style::default().fg(Color::Gray),
                ),
            ]));
        }
    }

    let breakdown_block = Block::default()
        .borders(Borders::ALL)
        .title("Memory Breakdown")
        .border_style(Style::default().fg(if focused {
            Color::Yellow
        } else {
            theme.ram_color
        }));

    let breakdown_para = Paragraph::new(breakdown_text)
        .block(breakdown_block)
        .style(Style::default().fg(Color::White));

    f.render_widget(breakdown_para, area);
}

fn render_pagefile_gauge(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::RamData,
    theme: &Theme,
) {
    if data.total_pagefile_size == 0 {
        // No pagefile configured
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Pagefile")
            .border_style(Style::default().fg(Color::Gray));

        let text = Paragraph::new("No pagefile configured")
            .block(block)
            .style(Style::default().fg(Color::Gray));

        f.render_widget(text, area);
    } else if data.pagefiles.len() == 1 {
        // Single pagefile
        let pf = &data.pagefiles[0];
        let pagefile_percent = pf.usage_percent.min(100.0) as u16;

        let gauge = Gauge::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Pagefile: {}", pf.name))
                    .border_style(Style::default().fg(theme.disk_color)),
            )
            .gauge_style(
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )
            .percent(pagefile_percent)
            .label(format!(
                "{}% - {} / {}",
                pagefile_percent,
                format_bytes(pf.current_usage),
                format_bytes(pf.total_size)
            ));

        f.render_widget(gauge, area);
    } else {
        // Multiple pagefiles - show total
        let total_percent = if data.total_pagefile_size > 0 {
            ((data.total_pagefile_used as f64 / data.total_pagefile_size as f64) * 100.0).min(100.0)
                as u16
        } else {
            0
        };

        let gauge = Gauge::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Pagefile (Total: {} files)", data.pagefiles.len()))
                    .border_style(Style::default().fg(theme.disk_color)),
            )
            .gauge_style(
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )
            .percent(total_percent)
            .label(format!(
                "{}% - {} / {}",
                total_percent,
                format_bytes(data.total_pagefile_used),
                format_bytes(data.total_pagefile_size)
            ));

        f.render_widget(gauge, area);
    }
}

fn render_top_processes(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::RamData,
    app: &App,
    theme: &Theme,
    focused: bool,
) {
    let mut processes = data.top_processes.clone();
    sort_ram_processes(
        &mut processes,
        app.state.ram_state.sort_column,
        app.state.ram_state.sort_ascending,
    );
    if processes.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Top Memory Consumers")
            .border_style(Style::default().fg(if focused {
                Color::Yellow
            } else {
                theme.ram_color
            }));
        let text = Paragraph::new("No memory consumers detected")
            .block(block)
            .style(Style::default().fg(Color::Gray));
        f.render_widget(text, area);
        return;
    }
    let selected_index = app
        .state
        .ram_state
        .selected_index
        .min(processes.len().saturating_sub(1));

    let hotkeys_height = if area.height > 2 { 1 } else { 0 };
    let visible_rows = area
        .height
        .saturating_sub(2 + 1 + hotkeys_height)
        .max(1) as usize;
    let scroll_offset = if selected_index >= visible_rows {
        selected_index - (visible_rows - 1)
    } else {
        0
    };

    let sort_indicator = if app.state.ram_state.sort_ascending {
        "↑"
    } else {
        "↓"
    };
    let header_cells = vec![
        ratatui::widgets::Cell::from(
            if app.state.ram_state.sort_column == RamProcessSortColumn::Pid {
                format!("PID {sort_indicator}")
            } else {
                "PID".to_string()
            },
        ),
        ratatui::widgets::Cell::from(
            if app.state.ram_state.sort_column == RamProcessSortColumn::Name {
                format!("Process Name {sort_indicator}")
            } else {
                "Process Name".to_string()
            },
        ),
        ratatui::widgets::Cell::from(
            if app.state.ram_state.sort_column == RamProcessSortColumn::WorkingSet {
                format!("Working Set {sort_indicator}")
            } else {
                "Working Set".to_string()
            },
        ),
        ratatui::widgets::Cell::from(
            if app.state.ram_state.sort_column == RamProcessSortColumn::PrivateBytes {
                format!("Private Bytes {sort_indicator}")
            } else {
                "Private Bytes".to_string()
            },
        ),
    ]
    .into_iter()
    .map(|cell| {
        cell.style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    });
    let header = Row::new(header_cells).height(1).bottom_margin(0);

    let rows: Vec<Row> = processes
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_rows)
        .map(|(i, proc)| {
            let is_selected = i == selected_index;
            let style = if is_selected {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };
            let cells = vec![
                ratatui::widgets::Cell::from(proc.pid.to_string()),
                ratatui::widgets::Cell::from(proc.name.clone()),
                ratatui::widgets::Cell::from(format_bytes(proc.working_set)),
                ratatui::widgets::Cell::from(format_bytes(proc.private_bytes)),
            ];
            Row::new(cells).height(1).style(style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Min(20),
            Constraint::Length(15),
            Constraint::Length(15),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Top Memory Consumers")
            .border_style(Style::default().fg(if focused {
                Color::Yellow
            } else {
                theme.ram_color
            })),
    )
    .style(Style::default().fg(Color::White))
    .column_spacing(1)
    .highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan));

    f.render_widget(table, area);

    let hotkeys = vec![Line::from(vec![
        Span::styled("↑/↓", Style::default().fg(Color::Cyan)),
        Span::raw(": Navigate  "),
        Span::styled("p/n/w/b", Style::default().fg(Color::Cyan)),
        Span::raw(": Sort by PID/Name/Working Set/Private Bytes  "),
        Span::styled("PgUp/PgDn", Style::default().fg(Color::Cyan)),
        Span::raw(": Page Up/Down  "),
        Span::styled("←/→", Style::default().fg(Color::Cyan)),
        Span::raw(": Focus"),
    ])];
    if hotkeys_height > 0 {
        let hotkeys_area = Rect {
            x: area.x + 2,
            y: area.y + area.height - 2,
            width: area.width.saturating_sub(4),
            height: 1,
        };
        let hotkeys_paragraph = Paragraph::new(hotkeys);
        f.render_widget(hotkeys_paragraph, hotkeys_area);
    }
}

fn sort_ram_processes(
    processes: &mut Vec<ProcessMemoryInfo>,
    column: RamProcessSortColumn,
    ascending: bool,
) {
    processes.sort_by(|a, b| {
        let cmp = match column {
            RamProcessSortColumn::Pid => a.pid.cmp(&b.pid),
            RamProcessSortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            RamProcessSortColumn::WorkingSet => a.working_set.cmp(&b.working_set),
            RamProcessSortColumn::PrivateBytes => a.private_bytes.cmp(&b.private_bytes),
        };

        if ascending {
            cmp
        } else {
            cmp.reverse()
        }
    });
}
