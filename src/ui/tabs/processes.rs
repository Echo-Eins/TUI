use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
    Frame,
};
use std::cmp::Ordering;

use crate::app::{state::ProcessSortColumn, App};
use crate::monitors::processes::ProcessEntry;
use crate::ui::theme::Theme;
use crate::utils::format::format_bytes;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let process_data = app.state.process_data.read();
    let process_error = app.state.process_error.read();

    if let Some(message) = process_error.as_ref() {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);
        let block = Block::default()
            .title("Process Monitor")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.warning_color));

        let text = Paragraph::new(format!("Process monitor unavailable: {}", message))
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(text, area);
    } else if let Some(data) = process_data.as_ref() {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);

        if app.state.compact_mode {
            render_compact(f, area, data, app, &theme);
        } else {
            render_full(f, area, data, app, &theme);
        }
    } else {
        let block = Block::default()
            .title("Process Monitor")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));

        let text = Paragraph::new("Loading process data...")
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(text, area);
    }
}

fn render_full(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::ProcessData,
    app: &App,
    theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header with stats
            Constraint::Min(10),    // Process table
            Constraint::Length(10), // Details panel
        ])
        .split(area);

    // Render header
    render_header(f, chunks[0], data, theme);

    // Render process table
    render_process_table(f, chunks[1], data, app, theme);

    // Render details panel
    render_details_panel(f, chunks[2], data, app, theme);
}

fn render_compact(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::ProcessData,
    app: &App,
    theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(8),    // Process table (compact)
        ])
        .split(area);

    // Render header
    render_header(f, chunks[0], data, theme);

    // Render process table
    render_process_table(f, chunks[1], data, app, theme);
}

fn render_header(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::ProcessData,
    _theme: &Theme,
) {
    let total_processes = data.processes.len();
    let total_memory: u64 = data.processes.iter().map(|p| p.memory).sum();
    let total_threads: usize = data.processes.iter().map(|p| p.threads).sum();

    let header_text = vec![Line::from(vec![
        Span::styled("Total Processes: ", Style::default().fg(Color::Gray)),
        Span::styled(
            format!("{}", total_processes),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("Total Memory: ", Style::default().fg(Color::Gray)),
        Span::styled(
            format_bytes(total_memory),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("Total Threads: ", Style::default().fg(Color::Gray)),
        Span::styled(
            format!("{}", total_threads),
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
    ])];

    let block = Block::default()
        .title("Process Monitor")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(header_text).block(block);
    f.render_widget(paragraph, area);
}

fn render_process_table(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::ProcessData,
    app: &App,
    _theme: &Theme,
) {
    // Sort and filter processes
    let mut processes = data.processes.clone();

    // Apply filter if any
    if !app.state.processes_state.filter.is_empty() {
        let filter = app.state.processes_state.filter.to_lowercase();
        processes.retain(|p| {
            p.name.to_lowercase().contains(&filter)
                || p.user.to_lowercase().contains(&filter)
                || p.pid.to_string().contains(&filter)
        });
    }

    // Apply sorting
    sort_processes(
        &mut processes,
        app.state.processes_state.sort_column,
        app.state.processes_state.sort_ascending,
    );

    let selected_index = if processes.is_empty() {
        0
    } else {
        app.state
            .processes_state
            .selected_index
            .min(processes.len().saturating_sub(1))
    };

    let content_height = area.height.saturating_sub(2);
    let footer_height = if area.height > 2 { 1 } else { 0 };
    let header_height = 1u16;
    let visible_rows = content_height
        .saturating_sub(header_height + footer_height) as usize;

    let mut scroll_offset = app.state.processes_state.scroll_offset;
    if selected_index < scroll_offset {
        scroll_offset = selected_index;
    } else if visible_rows > 0 && selected_index >= scroll_offset + visible_rows {
        scroll_offset = selected_index + 1 - visible_rows;
    }
    if visible_rows == 0 {
        scroll_offset = 0;
    } else if processes.len() > visible_rows {
        scroll_offset = scroll_offset.min(processes.len() - visible_rows);
    } else {
        scroll_offset = 0;
    }

    // Create table header with sort indicators
    let sort_indicator = if app.state.processes_state.sort_ascending {
        "↑"
    } else {
        "↓"
    };

    let headers = vec![
        Cell::from(
            if app.state.processes_state.sort_column == ProcessSortColumn::Pid {
                format!("PID {}", sort_indicator)
            } else {
                "PID".to_string()
            },
        )
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from(
            if app.state.processes_state.sort_column == ProcessSortColumn::Name {
                format!("Name {}", sort_indicator)
            } else {
                "Name".to_string()
            },
        )
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from(
            if app.state.processes_state.sort_column == ProcessSortColumn::Cpu {
                format!("CPU% {}", sort_indicator)
            } else {
                "CPU%".to_string()
            },
        )
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from(
            if app.state.processes_state.sort_column == ProcessSortColumn::Memory {
                format!("Memory {}", sort_indicator)
            } else {
                "Memory".to_string()
            },
        )
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from(
            if app.state.processes_state.sort_column == ProcessSortColumn::Threads {
                format!("Threads {}", sort_indicator)
            } else {
                "Threads".to_string()
            },
        )
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from(
            if app.state.processes_state.sort_column == ProcessSortColumn::User {
                format!("User {}", sort_indicator)
            } else {
                "User".to_string()
            },
        )
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    let header = Row::new(headers).height(1);

    // Create table rows
    let rows: Vec<Row> = processes
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_rows.max(0))
        .map(|(i, process)| {
            let style = if i == selected_index {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };

            Row::new(vec![
                Cell::from(format!("{}", process.pid)).style(style),
                Cell::from(process.name.clone()).style(style),
                Cell::from(format!("{:.1}", process.cpu_usage)).style(style),
                Cell::from(format_bytes(process.memory)).style(style),
                Cell::from(format!("{}", process.threads)).style(style),
                Cell::from(process.user.clone()).style(style),
            ])
        })
        .collect();

    // Hotkeys hint
    let hotkeys = vec![Line::from(vec![
        Span::styled("↑/↓", Style::default().fg(Color::Cyan)),
        Span::raw(": Navigate  "),
        Span::styled("p/n/c/m/t/u", Style::default().fg(Color::Cyan)),
        Span::raw(": Sort by PID/Name/CPU/Memory/Threads/User  "),
        Span::styled("PgUp/PgDn", Style::default().fg(Color::Cyan)),
        Span::raw(": Page Up/Down"),
    ])];

    let block = Block::default()
        .title("Processes")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    // Calculate constraints for table columns
    let widths = [
        Constraint::Length(8),  // PID
        Constraint::Min(20),    // Name
        Constraint::Length(8),  // CPU%
        Constraint::Length(12), // Memory
        Constraint::Length(10), // Threads
        Constraint::Min(15),    // User
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1)
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan));

    f.render_widget(table, area);

    // Render hotkeys at the bottom of the area
    if area.height > 2 {
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

fn render_details_panel(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::ProcessData,
    app: &App,
    _theme: &Theme,
) {
    // Sort and filter processes (same as in table)
    let mut processes = data.processes.clone();

    if !app.state.processes_state.filter.is_empty() {
        let filter = app.state.processes_state.filter.to_lowercase();
        processes.retain(|p| {
            p.name.to_lowercase().contains(&filter)
                || p.user.to_lowercase().contains(&filter)
                || p.pid.to_string().contains(&filter)
        });
    }

    sort_processes(
        &mut processes,
        app.state.processes_state.sort_column,
        app.state.processes_state.sort_ascending,
    );

    let selected_index = if processes.is_empty() {
        0
    } else {
        app.state
            .processes_state
            .selected_index
            .min(processes.len().saturating_sub(1))
    };

    // Get selected process
    if let Some(process) = processes.get(selected_index) {
        let mut details = Vec::new();

        details.push(Line::from(vec![Span::styled(
            "Process Details",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]));

        details.push(Line::from(""));

        details.push(Line::from(vec![
            Span::styled("PID: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{}", process.pid),
                Style::default().fg(Color::White),
            ),
            Span::raw("  "),
            Span::styled("Name: ", Style::default().fg(Color::Gray)),
            Span::styled(
                &process.name,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        details.push(Line::from(vec![
            Span::styled("User: ", Style::default().fg(Color::Gray)),
            Span::styled(&process.user, Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled("Threads: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{}", process.threads),
                Style::default().fg(Color::White),
            ),
            Span::raw("  "),
            Span::styled("Handles: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{}", process.handle_count),
                Style::default().fg(Color::White),
            ),
        ]));

        details.push(Line::from(vec![
            Span::styled("CPU: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{:.2}%", process.cpu_usage),
                Style::default().fg(Color::Green),
            ),
            Span::raw("  "),
            Span::styled("Memory: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format_bytes(process.memory),
                Style::default().fg(Color::Yellow),
            ),
        ]));

        details.push(Line::from(vec![
            Span::styled("I/O Read: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format_bytes(process.io_read_bytes),
                Style::default().fg(Color::Blue),
            ),
            Span::raw("  "),
            Span::styled("I/O Write: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format_bytes(process.io_write_bytes),
                Style::default().fg(Color::Magenta),
            ),
        ]));

        if let Some(start_time) = &process.start_time {
            details.push(Line::from(vec![
                Span::styled("Start Time: ", Style::default().fg(Color::Gray)),
                Span::styled(start_time, Style::default().fg(Color::White)),
            ]));
        }

        if let Some(cmd) = &process.command_line {
            details.push(Line::from(""));
            details.push(Line::from(vec![Span::styled(
                "Command Line:",
                Style::default().fg(Color::Gray),
            )]));
            details.push(Line::from(vec![Span::styled(
                cmd,
                Style::default().fg(Color::White),
            )]));
        }

        let block = Block::default()
            .title("Process Details")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let paragraph = Paragraph::new(details)
            .block(block)
            .wrap(Wrap { trim: true });

        f.render_widget(paragraph, area);
    } else {
        let block = Block::default()
            .title("Process Details")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let text = Paragraph::new("No process selected")
            .block(block)
            .style(Style::default().fg(Color::Gray));

        f.render_widget(text, area);
    }
}

fn sort_processes(processes: &mut Vec<ProcessEntry>, column: ProcessSortColumn, ascending: bool) {
    processes.sort_by(|a, b| {
        let cmp = match column {
            ProcessSortColumn::Pid => a.pid.cmp(&b.pid),
            ProcessSortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            ProcessSortColumn::Cpu => a
                .cpu_usage
                .partial_cmp(&b.cpu_usage)
                .unwrap_or(Ordering::Equal),
            ProcessSortColumn::Memory => a.memory.cmp(&b.memory),
            ProcessSortColumn::Threads => a.threads.cmp(&b.threads),
            ProcessSortColumn::User => a.user.to_lowercase().cmp(&b.user.to_lowercase()),
        };

        if ascending {
            cmp
        } else {
            cmp.reverse()
        }
    });
}
