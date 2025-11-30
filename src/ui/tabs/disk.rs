use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph, Row, Sparkline, Table},
    Frame,
};

use crate::app::{state::MonitorStatus, App};
use crate::ui::theme::Theme;
use crate::ui::widgets::render_monitor_status;
use crate::utils::format::{create_progress_bar, format_bytes};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let disk_state = app.state.disk_data.read();

    if let (MonitorStatus::Ready, Some(data)) = (&disk_state.status, disk_state.data.as_ref()) {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);

        if app.state.compact_mode {
            render_compact(f, area, data, &theme);
        } else {
            render_full(f, area, data, &theme);
        }
    } else {
        render_monitor_status(
            f,
            area,
            "Disk Monitor",
            &disk_state.status,
            disk_state.last_updated,
        );
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

    // Calculate constraints for each disk (each disk gets equal space)
    let disk_count = data.physical_disks.len();
    let height_per_disk = 12; // Height for each disk panel
    let mut constraints = Vec::new();

    for _ in 0..disk_count {
        constraints.push(Constraint::Length(height_per_disk));
    }

    if constraints.is_empty() {
        constraints.push(Constraint::Min(0));
    } else {
        // Add remaining space
        constraints.push(Constraint::Min(0));
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    // Render each physical disk
    for (i, disk) in data.physical_disks.iter().enumerate() {
        if i < chunks.len() {
            render_physical_disk(f, chunks[i], disk, data, theme);
        }
    }
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

fn render_physical_disk(
    f: &mut Frame,
    area: Rect,
    disk: &crate::monitors::PhysicalDiskInfo,
    all_data: &crate::monitors::DiskData,
    theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header with model and health
            Constraint::Length(3), // Overall usage
            Constraint::Length(8), // I/O graphs
            Constraint::Min(8),    // Details, partitions, and process table
        ])
        .split(area);

    // Header
    let health_indicator = get_health_indicator(&disk.health_status);
    let temp_str = if let Some(temp) = disk.temperature {
        format!("  {}°C", temp)
    } else {
        String::new()
    };

    let header = format!(
        "{} Disk {}: {} {} | {} | {}{}",
        health_indicator,
        disk.disk_number,
        disk.model,
        disk.media_type,
        disk.bus_type,
        format_bytes(disk.size),
        temp_str
    );

    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(get_health_color(&disk.health_status)));

    let header_text = Paragraph::new(header).block(header_block).style(
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );

    f.render_widget(header_text, chunks[0]);

    // Overall usage gauge
    let free_space = get_disk_free_space(disk, all_data);
    let used_space = disk.size.saturating_sub(free_space);
    let usage_percent = if disk.size > 0 {
        ((used_space as f64 / disk.size as f64) * 100.0) as u16
    } else {
        0
    };

    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title("Total Usage"))
        .gauge_style(
            Style::default()
                .fg(get_usage_color(usage_percent as f32))
                .add_modifier(Modifier::BOLD),
        )
        .percent(usage_percent)
        .label(format!(
            "{}% - {} / {}",
            usage_percent,
            format_bytes(used_space),
            format_bytes(disk.size)
        ));

    f.render_widget(gauge, chunks[1]);

    // I/O Statistics and Graphs
    render_io_stats(f, chunks[2], disk, all_data, theme);

    // Details, partitions, and process table
    render_disk_details(f, chunks[3], disk, all_data, theme);
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

fn render_disk_details(
    f: &mut Frame,
    area: Rect,
    disk: &crate::monitors::PhysicalDiskInfo,
    all_data: &crate::monitors::DiskData,
    theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50), // Details and partitions
            Constraint::Percentage(50), // Process table
        ])
        .split(area);

    // Left side: Details and partitions
    let mut detail_lines = vec![];

    // Health and operational status
    detail_lines.push(Line::from(vec![
        Span::raw("  Health: "),
        Span::styled(
            &disk.health_status,
            Style::default()
                .fg(get_health_color(&disk.health_status))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  |  Status: "),
        Span::styled(&disk.operational_status, Style::default().fg(Color::Cyan)),
    ]));

    // SMART data if available
    if let Some(hours) = disk.power_on_hours {
        detail_lines.push(Line::from(vec![
            Span::raw("  Power-On Hours: "),
            Span::styled(format!("{} hrs", hours), Style::default().fg(Color::Yellow)),
        ]));
    }

    if let Some(tbw) = disk.tbw {
        detail_lines.push(Line::from(vec![
            Span::raw("  Total Bytes Written: "),
            Span::styled(format_bytes(tbw), Style::default().fg(Color::Magenta)),
        ]));
    }

    if let Some(wear) = disk.wear_level {
        detail_lines.push(Line::from(vec![
            Span::raw("  Wear Level: "),
            Span::styled(format!("{:.1}%", wear), Style::default().fg(Color::Green)),
        ]));
    }

    // Partitions
    if !disk.partitions.is_empty() {
        detail_lines.push(Line::from(""));
        detail_lines.push(Line::from(vec![Span::styled(
            "  Partitions:",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]));

        for partition_letter in &disk.partitions {
            if let Some(drive) = all_data
                .logical_drives
                .iter()
                .find(|d| &d.letter == partition_letter)
            {
                let usage_pct = if drive.total > 0 {
                    (drive.used as f64 / drive.total as f64 * 100.0) as f32
                } else {
                    0.0
                };

                detail_lines.push(Line::from(vec![
                    Span::raw(format!("    {} ", drive.letter)),
                    Span::styled(
                        format!("{:15}", drive.name),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw("  "),
                    Span::raw(create_progress_bar(usage_pct, 15)),
                    Span::raw(format!("  {:.0}%", usage_pct)),
                ]));
            }
        }
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Details & Partitions")
        .border_style(Style::default().fg(theme.disk_color));

    let para = Paragraph::new(detail_lines)
        .block(block)
        .style(Style::default().fg(Color::White));

    f.render_widget(para, chunks[0]);

    // Right side: Process table
    render_process_table(f, chunks[1], all_data, theme);
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
        "Healthy" => "●●●●●",
        "Warning" => "●●●●○",
        "Unhealthy" => "●●○○○",
        _ => "●●●○○",
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
