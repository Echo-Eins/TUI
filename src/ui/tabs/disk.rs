use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Sparkline, Table},
    Frame,
};

use crate::app::state::{DiskPanelFocus, DiskUIState};
use crate::app::App;
use crate::monitors::{DiskData, DiskIOHistory, DiskIOStats, DriveInfo, PhysicalDiskInfo};
use crate::ui::theme::Theme;
use crate::utils::format::{create_progress_bar, format_bytes};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let disk_data = app.state.disk_data.read();
    let disk_error = app.state.disk_error.read();
    let config = app.state.config.read();
    let theme = Theme::from_config(&config);

    if let Some(message) = disk_error.as_ref() {
        render_message(
            f,
            area,
            "Disk Monitor",
            &format!("Disk monitor unavailable: {message}"),
            theme.warning_color,
        );
    } else if let Some(data) = disk_data.as_ref() {
        if app.state.compact_mode || area.width < 120 || area.height < 24 {
            render_compact(f, area, data, &theme);
        } else {
            render_full(f, area, data, &app.state.disk_state, &theme);
        }
    } else {
        render_message(
            f,
            area,
            "Disk Monitor",
            "Loading disk data...",
            theme.disk_color,
        );
    }
}

fn render_message(f: &mut Frame, area: Rect, title: &str, message: &str, color: Color) {
    let widget = Paragraph::new(message)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(color)),
        )
        .style(Style::default().fg(Color::Gray));
    f.render_widget(widget, area);
}

fn render_full(
    f: &mut Frame,
    area: Rect,
    data: &DiskData,
    disk_state: &DiskUIState,
    theme: &Theme,
) {
    if data.physical_disks.is_empty() {
        render_message(
            f,
            area,
            "Physical Disks",
            "No physical storage devices found",
            theme.disk_color,
        );
        return;
    }

    let summary_height = (data.physical_disks.len() as u16 + 3).clamp(5, 10);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(summary_height),
            Constraint::Length(9),
            Constraint::Min(9),
        ])
        .split(area);

    render_physical_disks(f, sections[0], data, theme);
    render_io_dashboard(f, sections[1], data, theme);

    let lower = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(sections[2]);
    render_volumes(f, lower[0], data, disk_state, theme);
    render_process_table(f, lower[1], data, disk_state, theme);
}

fn render_physical_disks(f: &mut Frame, area: Rect, data: &DiskData, theme: &Theme) {
    let header = Row::new([
        "Device",
        "Model",
        "Type",
        "Capacity",
        "Mounted FS",
        "Used",
        "Usage",
        "Health",
        "Temp",
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );

    let rows = data.physical_disks.iter().map(|disk| {
        let usage = filesystem_usage_percent(disk);
        let mounted = if disk.filesystem_total == 0 {
            "-".to_string()
        } else {
            format_bytes(disk.filesystem_total)
        };
        let used = if disk.filesystem_total == 0 {
            "-".to_string()
        } else {
            format_bytes(disk.filesystem_used)
        };
        let usage_cell = if disk.filesystem_total == 0 {
            Cell::from("-")
        } else {
            Cell::from(format!(
                "{} {:>3.0}%",
                create_progress_bar(usage, 10),
                usage
            ))
            .style(Style::default().fg(get_usage_color(usage)))
        };
        Row::new(vec![
            Cell::from(disk.friendly_name.clone()).style(Style::default().fg(Color::Cyan)),
            Cell::from(disk.model.clone()).style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Cell::from(format!("{} {}", disk.media_type, disk.bus_type)),
            Cell::from(format_bytes(disk.size)).style(Style::default().fg(Color::Yellow)),
            Cell::from(mounted),
            Cell::from(used),
            usage_cell,
            Cell::from(disk.health_status.clone())
                .style(Style::default().fg(get_health_color(&disk.health_status))),
            Cell::from(
                disk.temperature
                    .map(|temp| format!("{temp:.0} C"))
                    .unwrap_or_else(|| "-".to_string()),
            ),
        ])
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Min(20),
            Constraint::Length(14),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(18),
            Constraint::Length(10),
            Constraint::Length(7),
        ],
    )
    .header(header)
    .column_spacing(1)
    .block(
        Block::default()
            .title(format!(" Physical Disks ({}) ", data.physical_disks.len()))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.disk_color)),
    );
    f.render_widget(table, area);
}

fn render_io_dashboard(f: &mut Frame, area: Rect, data: &DiskData, theme: &Theme) {
    let disk = &data.physical_disks[0];
    let stats = data
        .io_stats
        .iter()
        .find(|stats| stats.disk_number == disk.disk_number);
    let history = data
        .io_history
        .iter()
        .find(|history| history.disk_number == disk.disk_number);

    let outer = Block::default()
        .title(format!(" I/O Activity - {} ", disk.friendly_name))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.disk_color));
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(36), Constraint::Min(30)])
        .split(inner);
    render_io_metrics(f, columns[0], stats);
    render_io_graphs(f, columns[1], history);
}

fn render_io_metrics(f: &mut Frame, area: Rect, stats: Option<&DiskIOStats>) {
    let lines = if let Some(stats) = stats {
        vec![
            metric_line(
                "Read",
                format_throughput(stats.read_speed),
                format!("{:.0} IOPS", stats.read_iops),
                Color::Green,
            ),
            metric_line(
                "Write",
                format_throughput(stats.write_speed),
                format!("{:.0} IOPS", stats.write_iops),
                Color::Cyan,
            ),
            Line::from(vec![
                Span::raw(" Queue "),
                Span::styled(
                    format!("{:.2}", stats.queue_depth),
                    Style::default().fg(Color::Magenta),
                ),
                Span::raw("   Latency "),
                Span::styled(
                    format!("{:.2} ms", stats.avg_response_time),
                    Style::default().fg(Color::Yellow),
                ),
            ]),
            Line::from(vec![
                Span::raw(" Active "),
                Span::styled(
                    format!("{:.1}%", stats.active_time),
                    Style::default().fg(get_usage_color(stats.active_time as f32)),
                ),
            ]),
        ]
    } else {
        vec![Line::from(" Collecting I/O counters...")]
    };
    f.render_widget(Paragraph::new(lines), area);
}

fn metric_line(
    label: &'static str,
    throughput: String,
    iops: String,
    color: Color,
) -> Line<'static> {
    Line::from(vec![
        Span::raw(format!(" {label:<5} ")),
        Span::styled(throughput, Style::default().fg(color)),
        Span::raw("   "),
        Span::styled(iops, Style::default().fg(color)),
    ])
}

fn render_io_graphs(f: &mut Frame, area: Rect, history: Option<&DiskIOHistory>) {
    let graphs = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(area);

    let Some(history) = history else {
        render_message(
            f,
            area,
            "I/O History",
            "Building history...",
            Color::DarkGray,
        );
        return;
    };

    render_sparkline(
        f,
        graphs[0],
        &history.read_history,
        "Read",
        Color::Green,
        true,
    );
    render_sparkline(
        f,
        graphs[1],
        &history.write_history,
        "Write",
        Color::Cyan,
        true,
    );
    render_sparkline(
        f,
        graphs[2],
        &history.iops_history,
        "IOPS",
        Color::Yellow,
        false,
    );
}

fn render_sparkline(
    f: &mut Frame,
    area: Rect,
    history: &std::collections::VecDeque<f64>,
    label: &str,
    color: Color,
    throughput: bool,
) {
    let scale = if throughput { 1024.0 } else { 1.0 };
    let data: Vec<u64> = history
        .iter()
        .map(|value| (value.max(0.0) * scale).round() as u64)
        .collect();
    let max = data.iter().copied().max().unwrap_or(1).max(1);
    let current = history.back().copied().unwrap_or(0.0);
    let peak = history.iter().copied().fold(0.0, f64::max);
    let verbose_title = if throughput {
        format!(
            " {label} {} | max {} ",
            format_throughput(current),
            format_throughput(peak)
        )
    } else {
        format!(" {label} {current:.0} | max {peak:.0} ")
    };
    let title = fit_block_title(&verbose_title, area.width);
    let widget = Sparkline::default()
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(color)),
        )
        .data(&data)
        .max(max)
        .style(Style::default().fg(color));
    f.render_widget(widget, area);
}

fn render_volumes(
    f: &mut Frame,
    area: Rect,
    data: &DiskData,
    disk_state: &DiskUIState,
    theme: &Theme,
) {
    let header = Row::new(["Volume / mounts", "FS", "Used", "Available", "Usage"]).style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    let mut rows = Vec::new();
    for (index, drive) in data.logical_drives.iter().enumerate() {
        let usage = drive_usage_percent(drive);
        let expanded = disk_state.expanded_volumes.contains(&drive.stable_key());
        let selected = disk_state.focused_panel == DiskPanelFocus::Filesystems
            && disk_state.selected_volume == index;
        let row_style = if selected {
            Style::default().fg(Color::Black).bg(theme.disk_color)
        } else {
            Style::default().fg(Color::White)
        };
        rows.push(
            Row::new(vec![
                Cell::from(volume_mount_label(drive, expanded)).style(if selected {
                    row_style
                } else {
                    Style::default().fg(Color::Cyan)
                }),
                Cell::from(drive.file_system.clone()),
                Cell::from(format_bytes(drive.used)),
                Cell::from(format_bytes(drive.free)),
                Cell::from(format!(
                    "{} {:>3.0}%",
                    create_progress_bar(usage, 10),
                    usage
                ))
                .style(Style::default().fg(get_usage_color(usage))),
            ])
            .style(row_style),
        );

        if expanded {
            for (mount_index, mount_point) in drive.mount_points.iter().enumerate() {
                let details = drive
                    .mount_details
                    .iter()
                    .find(|details| details.path == *mount_point);
                let mount_used = details.map_or(drive.used, |details| details.used);
                let mount_free = details.map_or(drive.free, |details| details.free);
                let mount_total = details.map_or(drive.total, |details| details.total);
                let mount_usage = percent(mount_used, mount_total);
                let branch = if mount_index + 1 == drive.mount_points.len() {
                    "└─"
                } else {
                    "├─"
                };
                rows.push(
                    Row::new(vec![
                        Cell::from(format!("  {branch} {mount_point}"))
                            .style(Style::default().fg(Color::Gray)),
                        Cell::from(drive.file_system.clone())
                            .style(Style::default().fg(Color::DarkGray)),
                        Cell::from(format_bytes(mount_used)),
                        Cell::from(format_bytes(mount_free)),
                        Cell::from(format!(
                            "{} {:>3.0}%",
                            create_progress_bar(mount_usage, 10),
                            mount_usage
                        ))
                        .style(Style::default().fg(get_usage_color(mount_usage))),
                    ])
                    .style(Style::default().fg(Color::Gray)),
                );
            }
        }
    }
    let focused = disk_state.focused_panel == DiskPanelFocus::Filesystems;
    let table = Table::new(
        rows,
        [
            Constraint::Min(22),
            Constraint::Length(10),
            Constraint::Length(13),
            Constraint::Length(13),
            Constraint::Length(18),
        ],
    )
    .header(header)
    .column_spacing(1)
    .block(
        Block::default()
            .title(format!(
                " {}Filesystems ({}) - Enter: mounts ",
                if focused { ">" } else { "" },
                data.logical_drives.len()
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if focused {
                theme.disk_color
            } else {
                Color::DarkGray
            })),
    );
    f.render_widget(table, area);
}

fn render_process_table(
    f: &mut Frame,
    area: Rect,
    data: &DiskData,
    disk_state: &DiskUIState,
    theme: &Theme,
) {
    let focused = disk_state.focused_panel == DiskPanelFocus::Processes;
    if data.process_activity.is_empty() {
        render_message(
            f,
            area,
            if focused {
                "> Top Disk I/O"
            } else {
                "Top Disk I/O"
            },
            "No process disk activity in the last sample",
            if focused {
                theme.disk_color
            } else {
                Color::DarkGray
            },
        );
        return;
    }

    let header = Row::new(["Process", "PID", "Read", "Write", "Total"]).style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    );
    let rows = data.process_activity.iter().map(|process| {
        Row::new([
            process.process_name.clone(),
            process.pid.to_string(),
            format_rate_bytes(process.read_bytes_per_sec),
            format_rate_bytes(process.write_bytes_per_sec),
            format_rate_bytes(process.io_bytes_per_sec),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Min(15),
            Constraint::Length(8),
            Constraint::Length(11),
            Constraint::Length(11),
            Constraint::Length(11),
        ],
    )
    .header(header)
    .column_spacing(1)
    .block(
        Block::default()
            .title(if focused {
                " > Top Disk I/O "
            } else {
                " Top Disk I/O "
            })
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if focused {
                theme.disk_color
            } else {
                Color::DarkGray
            })),
    );
    f.render_widget(table, area);
}

fn render_compact(f: &mut Frame, area: Rect, data: &DiskData, theme: &Theme) {
    let mut lines = Vec::new();
    for disk in &data.physical_disks {
        let usage = filesystem_usage_percent(disk);
        let mounted_usage = if disk.filesystem_total > 0 {
            format!(
                "{} / {} ({usage:.0}%)",
                format_bytes(disk.filesystem_used),
                format_bytes(disk.filesystem_total)
            )
        } else {
            "no mounted filesystem".to_string()
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} ", disk.friendly_name),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "{} | {} | {} | ",
                disk.model,
                disk.media_type,
                format_bytes(disk.size)
            )),
            Span::styled(mounted_usage, Style::default().fg(get_usage_color(usage))),
        ]));
    }
    if let Some(stats) = data.io_stats.first() {
        lines.push(Line::from(format!(
            "I/O  R {}  W {}  Q {:.2}  Active {:.1}%",
            format_throughput(stats.read_speed),
            format_throughput(stats.write_speed),
            stats.queue_depth,
            stats.active_time
        )));
    }
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(" Disk Monitor ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.disk_color)),
        ),
        area,
    );
}

fn filesystem_usage_percent(disk: &PhysicalDiskInfo) -> f32 {
    percent(disk.filesystem_used, disk.filesystem_total)
}

fn drive_usage_percent(drive: &DriveInfo) -> f32 {
    percent(drive.used, drive.total)
}

fn percent(used: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64 * 100.0).clamp(0.0, 100.0) as f32
    }
}

fn volume_mount_label(drive: &DriveInfo, expanded: bool) -> String {
    let mount_count = drive.mount_points.len();
    if mount_count <= 1 {
        drive.letter.clone()
    } else {
        format!(
            "{} {} ({} mounts)",
            if expanded { "▾" } else { "▸" },
            drive.letter,
            mount_count
        )
    }
}

fn format_throughput(megabytes_per_second: f64) -> String {
    let bytes = megabytes_per_second.max(0.0) * 1_048_576.0;
    format_rate_bytes(bytes)
}

fn format_rate_bytes(bytes_per_second: f64) -> String {
    if bytes_per_second >= 1_073_741_824.0 {
        format!("{:.1} GiB/s", bytes_per_second / 1_073_741_824.0)
    } else if bytes_per_second >= 1_048_576.0 {
        format!("{:.1} MiB/s", bytes_per_second / 1_048_576.0)
    } else if bytes_per_second >= 1024.0 {
        format!("{:.1} KiB/s", bytes_per_second / 1024.0)
    } else {
        format!("{bytes_per_second:.0} B/s")
    }
}

fn fit_block_title(title: &str, block_width: u16) -> String {
    let max_width = block_width.saturating_sub(2) as usize;
    if title.chars().count() <= max_width {
        return title.to_string();
    }
    if max_width <= 3 {
        return title.chars().take(max_width).collect();
    }
    let mut fitted: String = title.chars().take(max_width - 3).collect();
    fitted.push_str("...");
    fitted
}

fn get_health_color(health_status: &str) -> Color {
    match health_status {
        "Healthy" => Color::Green,
        "Warning" => Color::Yellow,
        "Unhealthy" => Color::Red,
        _ => Color::Gray,
    }
}

fn get_usage_color(usage: f32) -> Color {
    if usage < 70.0 {
        Color::Green
    } else if usage < 85.0 {
        Color::Yellow
    } else {
        Color::Red
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitors::{DiskIOHistory, DiskProcessActivity, MountPointInfo};
    use ratatui::{backend::TestBackend, Terminal};
    use std::collections::VecDeque;

    fn theme() -> Theme {
        Theme {
            background: Color::Black,
            foreground: Color::White,
            cpu_color: Color::Blue,
            gpu_color: Color::Green,
            ram_color: Color::Magenta,
            disk_color: Color::Yellow,
            network_color: Color::Cyan,
            warning_color: Color::Yellow,
            error_color: Color::Red,
            success_color: Color::Green,
        }
    }

    fn sample_data() -> DiskData {
        DiskData {
            physical_disks: vec![PhysicalDiskInfo {
                disk_number: 0,
                friendly_name: "nvme0n1".to_string(),
                device_path: "/dev/nvme0n1".to_string(),
                model: "Example NVMe".to_string(),
                media_type: "NVMe".to_string(),
                bus_type: "NVME".to_string(),
                size: 1_024_000_000_000,
                filesystem_total: 1_000_000_000_000,
                filesystem_used: 530_000_000_000,
                filesystem_available: 470_000_000_000,
                health_status: "Healthy".to_string(),
                operational_status: "OK".to_string(),
                temperature: Some(42.0),
                write_cache_enabled: false,
                power_on_hours: None,
                tbw: None,
                wear_level: None,
                partitions: vec!["/".to_string(), "/home".to_string()],
            }],
            logical_drives: vec![DriveInfo {
                letter: "/".to_string(),
                name: "Root".to_string(),
                source: "/dev/nvme0n1p3".to_string(),
                uuid: Some("uuid".to_string()),
                mount_points: vec!["/".to_string(), "/home".to_string()],
                mount_details: vec![
                    MountPointInfo {
                        path: "/".to_string(),
                        total: 1_000_000_000_000,
                        used: 530_000_000_000,
                        free: 470_000_000_000,
                    },
                    MountPointInfo {
                        path: "/home".to_string(),
                        total: 200_000_000_000,
                        used: 50_000_000_000,
                        free: 150_000_000_000,
                    },
                ],
                drive_type: "Local filesystem".to_string(),
                file_system: "btrfs".to_string(),
                total: 1_000_000_000_000,
                used: 530_000_000_000,
                free: 470_000_000_000,
                disk_number: Some(0),
            }],
            io_stats: vec![DiskIOStats {
                disk_number: 0,
                read_speed: 0.04,
                write_speed: 12.5,
                read_iops: 10.0,
                write_iops: 40.0,
                queue_depth: 0.2,
                avg_response_time: 0.4,
                active_time: 4.0,
            }],
            process_activity: vec![DiskProcessActivity {
                process_name: "example".to_string(),
                pid: 42,
                io_bytes_per_sec: 4096.0,
                read_bytes_per_sec: 1024.0,
                write_bytes_per_sec: 3072.0,
            }],
            io_history: vec![DiskIOHistory {
                disk_number: 0,
                read_history: VecDeque::from([0.01, 0.04]),
                write_history: VecDeque::from([2.0, 12.5]),
                iops_history: VecDeque::from([10.0, 50.0]),
            }],
        }
    }

    #[test]
    fn volume_label_collapses_duplicate_subvolume_mounts() {
        assert_eq!(
            volume_mount_label(&sample_data().logical_drives[0], false),
            "▸ / (2 mounts)"
        );
    }

    #[test]
    fn full_disk_view_keeps_outer_borders_and_aligned_sections() {
        let backend = TestBackend::new(160, 32);
        let mut terminal = Terminal::new(backend).expect("backend");
        let data = sample_data();
        terminal
            .draw(|frame| {
                render_full(
                    frame,
                    frame.area(),
                    &data,
                    &DiskUIState {
                        focused_panel: DiskPanelFocus::Filesystems,
                        selected_volume: 0,
                        expanded_volumes: std::collections::HashSet::new(),
                    },
                    &theme(),
                )
            })
            .expect("render");

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].symbol(), "┌");
        assert_eq!(buffer[(159, 0)].symbol(), "┐");
        assert_eq!(buffer[(0, 31)].symbol(), "└");
        assert_eq!(buffer[(159, 31)].symbol(), "┘");
    }

    #[test]
    fn low_throughput_is_not_rounded_to_zero_in_titles() {
        assert_eq!(format_throughput(0.04), "41.0 KiB/s");
    }

    #[test]
    fn graph_title_never_exceeds_its_block() {
        let title = fit_block_title(" Read 999.9 GiB/s | max 999.9 GiB/s ", 20);
        assert!(title.chars().count() <= 18);
    }

    #[test]
    fn expanded_volume_renders_each_mount_point() {
        let backend = TestBackend::new(100, 12);
        let mut terminal = Terminal::new(backend).expect("backend");
        let data = sample_data();
        let mut expanded = std::collections::HashSet::new();
        expanded.insert(data.logical_drives[0].stable_key());
        let state = DiskUIState {
            focused_panel: DiskPanelFocus::Filesystems,
            selected_volume: 0,
            expanded_volumes: expanded,
        };

        terminal
            .draw(|frame| {
                render_volumes(frame, frame.area(), &data, &state, &theme());
            })
            .expect("render");

        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("└─ /"));
        assert!(rendered.contains("└─ /home"));
        assert!(rendered.contains("46.57 GB"));
        assert!(rendered.contains("25%"));
    }
}
