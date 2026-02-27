use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Row, Sparkline, Table},
    Frame,
};

use crate::app::App;
use crate::ui::theme::Theme;
use crate::utils::format::{create_progress_bar, format_bytes};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let disk_data = app.state.disk_data.read();
    let disk_error = app.state.disk_error.read();

    if let Some(message) = disk_error.as_ref() {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);
        let block = Block::default()
            .title("Disk Monitor")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.warning_color));

        let text = Paragraph::new(format!("Disk monitor unavailable: {}", message))
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(text, area);
    } else if let Some(data) = disk_data.as_ref() {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);

        if app.state.compact_mode {
            render_compact(f, area, data, &theme);
        } else {
            render_full(f, area, data, &theme);
        }
    } else {
        let block = Block::default()
            .title("Disk Monitor")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red));

        let text = Paragraph::new("Loading disk data...")
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(text, area);
    }
}

fn render_full(f: &mut Frame, area: Rect, data: &crate::monitors::DiskData, theme: &Theme) {
    if data.physical_disks.is_empty() {
        let block = Block::default()
            .title("Disk Monitor")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.disk_color));

        let text = Paragraph::new("No physical disks found")
            .block(block)
            .style(Style::default().fg(Color::Gray));

        f.render_widget(text, area);
        return;
    }

    // Unified layout: disk summary table + I/O section + process table
    let disk_summary_height = (data.physical_disks.len() as u16 * 3) + 2; // 3 lines per disk + border
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(disk_summary_height.min(14)), // Disk summary with usage bars
            Constraint::Length(8),                           // I/O statistics and graphs
            Constraint::Min(8),                              // Details & Processes
        ])
        .split(area);

    // Disk summary table with usage gauges
    render_disk_summary(f, chunks[0], data, theme);

    // I/O section: show first disk's I/O stats and combined graph
    if let Some(first_disk) = data.physical_disks.first() {
        render_io_stats(f, chunks[1], first_disk, data, theme);
    }

    // Bottom: Details & Processes side by side
    render_combined_details(f, chunks[2], data, theme);
}

fn render_disk_summary(f: &mut Frame, area: Rect, data: &crate::monitors::DiskData, theme: &Theme) {
    let mut lines = Vec::new();

    for disk in &data.physical_disks {
        let health_indicator = get_health_indicator(&disk.health_status);
        let free_space = get_disk_free_space(disk, data);
        let used_space = disk.size.saturating_sub(free_space);
        let usage_pct = if disk.size > 0 {
            (used_space as f64 / disk.size as f64 * 100.0) as f32
        } else {
            0.0
        };

        let temp_str = if let Some(temp) = disk.temperature {
            format!(" {:.0} C", temp)
        } else {
            String::new()
        };

        let smart_str = if let Some(hours) = disk.power_on_hours {
            format!(" {}h", hours)
        } else {
            String::new()
        };

        // Header line: model, type, size, health
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {} ", health_indicator),
                Style::default().fg(get_health_color(&disk.health_status)),
            ),
            Span::styled(
                format!("{} ", disk.model),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} {} ", disk.media_type, disk.bus_type),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                format!("{}", format_bytes(disk.size)),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                format!("{}{}", temp_str, smart_str),
                Style::default().fg(Color::Gray),
            ),
        ]));

        // Usage bar line
        let bar = create_progress_bar(usage_pct, 30);
        lines.push(Line::from(vec![
            Span::raw("   "),
            Span::styled(bar, Style::default().fg(get_usage_color(usage_pct))),
            Span::raw(format!(
                " {:.0}%  {} / {}",
                usage_pct,
                format_bytes(used_space),
                format_bytes(disk.size)
            )),
        ]));

        // Partitions on one line
        let parts: Vec<String> = disk.partitions.iter().take(4).cloned().collect();
        if !parts.is_empty() {
            lines.push(Line::from(vec![
                Span::raw("   Partitions: "),
                Span::styled(parts.join("  "), Style::default().fg(Color::Gray)),
            ]));
        }
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("Disks ({})", data.physical_disks.len()))
        .border_style(Style::default().fg(theme.disk_color));

    let para = Paragraph::new(lines)
        .block(block)
        .style(Style::default().fg(Color::White));

    f.render_widget(para, area);
}

fn render_combined_details(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::DiskData,
    theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Left: logical drives table
    let mut drive_lines = Vec::new();
    for drive in &data.logical_drives {
        let usage_pct = if drive.total > 0 {
            (drive.used as f64 / drive.total as f64 * 100.0) as f32
        } else {
            0.0
        };
        let bar = create_progress_bar(usage_pct, 12);
        drive_lines.push(Line::from(vec![
            Span::styled(
                format!(" {:12} ", drive.letter),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                format!("{:6} ", drive.file_system),
                Style::default().fg(Color::Cyan),
            ),
            Span::raw(bar),
            Span::raw(format!(
                " {:.0}%  {} / {}",
                usage_pct,
                format_bytes(drive.used),
                format_bytes(drive.total)
            )),
        ]));
    }

    let drives_block = Block::default()
        .borders(Borders::ALL)
        .title("Logical Drives")
        .border_style(Style::default().fg(theme.disk_color));

    let drives_para = Paragraph::new(drive_lines)
        .block(drives_block)
        .style(Style::default().fg(Color::White));

    f.render_widget(drives_para, chunks[0]);

    // Right: Process table
    render_process_table(f, chunks[1], data, theme);
}

fn render_compact(f: &mut Frame, area: Rect, data: &crate::monitors::DiskData, theme: &Theme) {
    let mut info_lines = vec![];

    // Show summary of all disks
    for disk in &data.physical_disks {
        let health_indicator = get_health_indicator(&disk.health_status);
        let usage_pct = ((disk.size as f64 - get_disk_free_space(disk, data) as f64)
            / disk.size as f64
            * 100.0) as u16;

        info_lines.push(Line::from(vec![
            Span::styled(
                format!("Disk {}: ", disk.disk_number),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("{} {} ", health_indicator, disk.model)),
            Span::styled(
                format!("{}%", usage_pct),
                Style::default().fg(get_usage_color(usage_pct as f32)),
            ),
        ]));
    }

    let block = Block::default()
        .title("Disk Monitor")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.disk_color));

    let para = Paragraph::new(info_lines)
        .block(block)
        .style(Style::default().fg(Color::White));

    f.render_widget(para, area);
}

fn render_io_stats(
    f: &mut Frame,
    area: Rect,
    disk: &crate::monitors::PhysicalDiskInfo,
    all_data: &crate::monitors::DiskData,
    theme: &Theme,
) {
    // Find I/O stats for this disk
    let io_stat = all_data
        .io_stats
        .iter()
        .find(|s| s.disk_number == disk.disk_number);

    // Find I/O history for this disk
    let io_history = all_data
        .io_history
        .iter()
        .find(|h| h.disk_number == disk.disk_number);

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(40), // I/O metrics text
            Constraint::Percentage(60), // Graphs
        ])
        .split(area);

    // Left side: I/O metrics
    let mut metrics_lines = vec![];

    if let Some(stat) = io_stat {
        metrics_lines.push(Line::from(vec![Span::styled(
            "I/O Activity",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]));

        metrics_lines.push(Line::from(vec![
            Span::raw(format!("  Read:  {:.2} MB/s  ", stat.read_speed)),
            Span::styled(
                format!("{:.0} IOPS", stat.read_iops),
                Style::default().fg(Color::Green),
            ),
        ]));

        metrics_lines.push(Line::from(vec![
            Span::raw(format!("  Write: {:.2} MB/s  ", stat.write_speed)),
            Span::styled(
                format!("{:.0} IOPS", stat.write_iops),
                Style::default().fg(Color::Cyan),
            ),
        ]));

        metrics_lines.push(Line::from(vec![
            Span::raw(format!("  Queue Depth: ")),
            Span::styled(
                format!("{:.1}", stat.queue_depth),
                Style::default().fg(Color::Magenta),
            ),
        ]));

        metrics_lines.push(Line::from(vec![
            Span::raw(format!("  Avg Response: ")),
            Span::styled(
                format!("{:.2} ms", stat.avg_response_time),
                Style::default().fg(Color::Yellow),
            ),
        ]));

        metrics_lines.push(Line::from(vec![
            Span::raw(format!("  Active Time: ")),
            Span::styled(
                format!("{:.1}%", stat.active_time),
                Style::default().fg(get_usage_color(stat.active_time as f32)),
            ),
        ]));
    } else {
        metrics_lines.push(Line::from("No I/O statistics available"));
    }

    let metrics_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.disk_color))
        .title("I/O Statistics");

    let metrics_para = Paragraph::new(metrics_lines)
        .block(metrics_block)
        .style(Style::default().fg(Color::White));

    f.render_widget(metrics_para, chunks[0]);

    // Right side: Graphs
    render_io_graphs(f, chunks[1], io_history, theme);
}

fn render_io_graphs(
    f: &mut Frame,
    area: Rect,
    io_history: Option<&crate::monitors::DiskIOHistory>,
    theme: &Theme,
) {
    if let Some(history) = io_history {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(33),
                Constraint::Percentage(33),
                Constraint::Percentage(34),
            ])
            .split(area);

        // Read speed graph
        if !history.read_history.is_empty() {
            let data: Vec<u64> = history.read_history.iter().map(|&v| v as u64).collect();
            let max_value = data.iter().max().copied().unwrap_or(1).max(1);

            let sparkline = Sparkline::default()
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!("Read (max {:.1} MB/s)", max_value))
                        .border_style(Style::default().fg(Color::Green)),
                )
                .data(&data)
                .style(Style::default().fg(Color::Green))
                .max(max_value);

            f.render_widget(sparkline, chunks[0]);
        }

        // Write speed graph
        if !history.write_history.is_empty() {
            let data: Vec<u64> = history.write_history.iter().map(|&v| v as u64).collect();
            let max_value = data.iter().max().copied().unwrap_or(1).max(1);

            let sparkline = Sparkline::default()
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!("Write (max {:.1} MB/s)", max_value))
                        .border_style(Style::default().fg(Color::Cyan)),
                )
                .data(&data)
                .style(Style::default().fg(Color::Cyan))
                .max(max_value);

            f.render_widget(sparkline, chunks[1]);
        }

        // IOPS graph
        if !history.iops_history.is_empty() {
            let data: Vec<u64> = history.iops_history.iter().map(|&v| v as u64).collect();
            let max_value = data.iter().max().copied().unwrap_or(1).max(1);

            let sparkline = Sparkline::default()
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!("Total IOPS (max {})", max_value))
                        .border_style(Style::default().fg(Color::Yellow)),
                )
                .data(&data)
                .style(Style::default().fg(Color::Yellow))
                .max(max_value);

            f.render_widget(sparkline, chunks[2]);
        }
    } else {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("I/O Graphs")
            .border_style(Style::default().fg(theme.disk_color));

        let text = Paragraph::new("Building graph history...")
            .block(block)
            .style(Style::default().fg(Color::Gray));

        f.render_widget(text, area);
    }
}

fn render_process_table(
    f: &mut Frame,
    area: Rect,
    all_data: &crate::monitors::DiskData,
    theme: &Theme,
) {
    if all_data.process_activity.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Top Processes by Disk I/O")
            .border_style(Style::default().fg(theme.disk_color));

        let text = Paragraph::new("No process activity detected")
            .block(block)
            .style(Style::default().fg(Color::Gray));

        f.render_widget(text, area);
        return;
    }

    // Create table rows
    let header = Row::new(vec!["Process", "PID", "I/O/s"])
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .bottom_margin(1);

    let rows: Vec<Row> = all_data
        .process_activity
        .iter()
        .take(6)
        .map(|proc| {
            let io_formatted = if proc.io_bytes_per_sec > 1_000_000.0 {
                format!("{:.1} MB/s", proc.io_bytes_per_sec / 1_000_000.0)
            } else if proc.io_bytes_per_sec > 1_000.0 {
                format!("{:.1} KB/s", proc.io_bytes_per_sec / 1_000.0)
            } else {
                format!("{:.0} B/s", proc.io_bytes_per_sec)
            };

            Row::new(vec![
                format!(
                    "{:20}",
                    if proc.process_name.len() > 20 {
                        format!("{}...", &proc.process_name[..17])
                    } else {
                        proc.process_name.clone()
                    }
                ),
                format!("{:6}", proc.pid),
                io_formatted,
            ])
            .style(Style::default().fg(Color::White))
        })
        .collect();

    let widths = [
        Constraint::Percentage(50),
        Constraint::Percentage(20),
        Constraint::Percentage(30),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Top Processes by Disk I/O")
                .border_style(Style::default().fg(theme.disk_color)),
        )
        .column_spacing(1);

    f.render_widget(table, area);
}

fn get_health_indicator(health_status: &str) -> &'static str {
    match health_status {
        "Healthy" => "[#####]",
        "Warning" => "[####-]",
        "Unhealthy" => "[##---]",
        _ => "[###--]",
    }
}

fn get_health_color(health_status: &str) -> Color {
    match health_status {
        "Healthy" => Color::Green,
        "Warning" => Color::Yellow,
        "Unhealthy" => Color::Red,
        _ => Color::Gray,
    }
}

fn get_usage_color(usage_percent: f32) -> Color {
    if usage_percent < 70.0 {
        Color::Green
    } else if usage_percent < 85.0 {
        Color::Yellow
    } else {
        Color::Red
    }
}

fn get_disk_free_space(
    disk: &crate::monitors::PhysicalDiskInfo,
    all_data: &crate::monitors::DiskData,
) -> u64 {
    all_data
        .logical_drives
        .iter()
        .filter(|d| d.disk_number == Some(disk.disk_number))
        .map(|d| d.free)
        .sum()
}
