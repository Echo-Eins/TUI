use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Row, Sparkline, Table, Wrap},
    Frame,
};

use crate::app::state::{
    NetworkCenterView, NetworkDiagHistoryEntry, NetworkDiagnosticTool, NetworkFocusZone,
    NetworkResultTab, NetworkUIState, ToolCategory, TrafficMarker,
};
use crate::app::App;
use crate::monitors::NetworkData;
use crate::ui::theme::Theme;
use crate::utils::format::format_bytes;

// ─────────────────────────── entry point ───────────────────────────

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let network_data = app.state.network_data.read();
    let network_error = app.state.network_error.read();
    let network_ui = &app.state.network_ui_state;

    if let Some(message) = network_error.as_ref() {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);
        let block = Block::default()
            .title("Network Monitor")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.warning_color));
        let text = Paragraph::new(format!("Network monitor unavailable: {}", message))
            .block(block)
            .style(Style::default().fg(Color::White));
        f.render_widget(text, area);
    } else if let Some(data) = network_data.as_ref() {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);
        if app.state.compact_mode {
            render_compact(f, area, data, network_ui, &theme);
        } else {
            render_full(f, area, data, network_ui, &theme);
        }
    } else {
        let block = Block::default()
            .title("Network Monitor")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));
        let text = Paragraph::new("Loading network data...")
            .block(block)
            .style(Style::default().fg(Color::White));
        f.render_widget(text, area);
    }
}

// ─────────────────────────── full view (3-column) ──────────────────

fn render_full(
    f: &mut Frame,
    area: Rect,
    data: &NetworkData,
    ui: &NetworkUIState,
    theme: &Theme,
) {
    // Top-level: header + body + footer
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // header bar
            Constraint::Min(16),   // body (3-column)
            Constraint::Length(7), // bottom (params + activity)
            Constraint::Length(1), // status/help bar
        ])
        .split(area);

    render_header_bar(f, main_chunks[0], data, ui, theme);

    // Body: 3-column layout
    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(22), // Tools navigator
            Constraint::Min(30),    // Center (interface/connections)
            Constraint::Min(36),    // Results workspace
        ])
        .split(main_chunks[1]);

    render_tools_panel(f, body_chunks[0], ui, theme);
    render_center_panel(f, body_chunks[1], data, ui, theme);
    render_results_panel(f, body_chunks[2], ui, theme);

    // Bottom: params + activity
    let bottom_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(40), // parameters
            Constraint::Percentage(60), // activity log
        ])
        .split(main_chunks[2]);

    render_parameters_panel(f, bottom_chunks[0], ui, theme);
    render_activity_panel(f, bottom_chunks[1], ui, theme);

    // Help bar
    render_help_bar(f, main_chunks[3], ui, theme);
}

// ─────────────────────────── compact view ──────────────────────────

fn render_compact(
    f: &mut Frame,
    area: Rect,
    data: &NetworkData,
    ui: &NetworkUIState,
    theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // header
            Constraint::Length(7),  // tool + params (left) / result summary (right)
            Constraint::Min(8),    // interface info / connections
            Constraint::Length(1), // help bar
        ])
        .split(area);

    render_header_bar(f, chunks[0], data, ui, theme);

    // Mid section: tool+params | result
    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(chunks[1]);

    render_compact_tool_params(f, mid[0], ui, theme);
    render_compact_result(f, mid[1], ui, theme);

    // Bottom: interface info or connections
    render_compact_bottom(f, chunks[2], data, ui, theme);

    render_help_bar(f, chunks[3], ui, theme);
}

// ─────────────────────────── header bar ────────────────────────────

fn render_header_bar(
    f: &mut Frame,
    area: Rect,
    data: &NetworkData,
    ui: &NetworkUIState,
    theme: &Theme,
) {
    let iface = primary_interface(data);
    let job_status = match ui.running_job {
        Some(id) => format!("Job #{id} running"),
        None => "idle".to_string(),
    };

    let marker_label = if ui.show_marker_traffic { " [M]" } else { "" };

    let header_text = if let Some(iface) = iface {
        let (rx_display, tx_display) = traffic_display(iface, ui);
        format!(
            "Network: {} {} | \u{2193} {:.2} Mbps  \u{2191} {:.2} Mbps | GW {} | Conns: {} | RX {} TX {}{} | Jobs: {}",
            iface.name,
            if iface.status.eq_ignore_ascii_case("connected") { "UP" } else { &iface.status },
            iface.download_speed,
            iface.upload_speed,
            if iface.gateway.is_empty() { "N/A" } else { &iface.gateway },
            data.connections.len(),
            rx_display,
            tx_display,
            marker_label,
            job_status,
        )
    } else {
        format!("Network: No active interfaces | Jobs: {}", job_status)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.network_color));

    let paragraph = Paragraph::new(header_text).block(block).style(
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );
    f.render_widget(paragraph, area);
}

// ─────────────────────────── tools panel (left) ────────────────────

fn render_tools_panel(f: &mut Frame, area: Rect, ui: &NetworkUIState, theme: &Theme) {
    let is_focused = ui.focus == NetworkFocusZone::Tools;
    let border_color = if is_focused {
        Color::Yellow
    } else {
        theme.network_color
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(if is_focused { "TOOLS [*]" } else { "TOOLS" })
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 3 || inner.height < 3 {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    let mut last_category: Option<ToolCategory> = None;

    for tool in NetworkDiagnosticTool::ORDERED.iter() {
        let cat = tool.category();
        if last_category != Some(cat) {
            if last_category.is_some() {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(
                cat.label(),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )));
            last_category = Some(cat);
        }

        let is_selected = *tool == ui.selected_tool;
        let prefix = if is_selected { "> " } else { "  " };
        let style = if is_selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        lines.push(Line::from(Span::styled(
            format!("{}{}", prefix, tool.label()),
            style,
        )));
    }

    let visible_height = inner.height as usize;
    let total_lines = lines.len();
    let scroll = if total_lines > visible_height {
        // Find the line of the selected tool and center it
        let selected_line = lines
            .iter()
            .position(|l| {
                let text = line_text(l);
                text.starts_with("> ")
            })
            .unwrap_or(0);
        selected_line.saturating_sub(visible_height / 2)
    } else {
        0
    };

    let paragraph = Paragraph::new(lines).scroll((scroll as u16, 0));
    f.render_widget(paragraph, inner);
}

// ─────────────────────────── center panel (interface/connections) ───

fn render_center_panel(
    f: &mut Frame,
    area: Rect,
    data: &NetworkData,
    ui: &NetworkUIState,
    theme: &Theme,
) {
    let is_focused = ui.focus == NetworkFocusZone::Interface;
    let border_color = if is_focused {
        Color::Yellow
    } else {
        theme.network_color
    };

    match ui.center_view {
        NetworkCenterView::Interface => {
            render_center_interface(f, area, data, ui, theme, border_color);
        }
        NetworkCenterView::Connections => {
            render_center_connections(f, area, data, ui, theme, border_color);
        }
    }
}

fn render_center_interface(
    f: &mut Frame,
    area: Rect,
    data: &NetworkData,
    ui: &NetworkUIState,
    theme: &Theme,
    border_color: Color,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8), // interface details
            Constraint::Min(6),   // traffic graphs
            Constraint::Length(3), // totals + view toggle
        ])
        .split(area);

    // Interface details
    let iface_idx = ui.selected_interface_idx.min(data.interfaces.len().saturating_sub(1));
    let iface = data.interfaces.get(iface_idx);
    let iface_count = data.interfaces.len();
    let iface_title = if let Some(iface) = iface {
        format!(
            "{} ({}/{}) [V] toggle view",
            iface.name,
            iface_idx + 1,
            iface_count
        )
    } else {
        "No interfaces [V] toggle view".to_string()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(iface_title)
        .border_style(Style::default().fg(border_color));
    let inner_iface = block.inner(chunks[0]);
    f.render_widget(block, chunks[0]);

    if let Some(iface) = iface {
        let lines = vec![
            Line::from(vec![
                Span::styled("Status: ", dim()),
                Span::styled(
                    &iface.status,
                    Style::default()
                        .fg(if iface.status.eq_ignore_ascii_case("connected") {
                            Color::Green
                        } else {
                            Color::Red
                        })
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled("Speed: ", dim()),
                Span::styled(&iface.link_speed, Style::default().fg(Color::Cyan)),
                Span::raw("  "),
                Span::styled("Duplex: ", dim()),
                Span::styled(&iface.duplex, Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled("MTU: ", dim()),
                Span::styled(format!("{}", iface.mtu), Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("IPv4: ", dim()),
                Span::styled(&iface.ipv4_address, Style::default().fg(Color::Cyan)),
                Span::raw("  "),
                Span::styled("IPv6: ", dim()),
                Span::styled(
                    truncate_str(&iface.ipv6_address, 28),
                    Style::default().fg(Color::Cyan),
                ),
            ]),
            Line::from(vec![
                Span::styled("Gateway: ", dim()),
                Span::styled(&iface.gateway, Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled("MAC: ", dim()),
                Span::styled(&iface.mac_address, Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("DNS: ", dim()),
                Span::styled(
                    if iface.dns_servers.is_empty() {
                        "N/A".to_string()
                    } else {
                        iface.dns_servers.join(", ")
                    },
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(vec![
                Span::styled("Interface: ", dim()),
                Span::styled(&iface.description, Style::default().fg(Color::DarkGray)),
            ]),
        ];
        let paragraph = Paragraph::new(lines);
        f.render_widget(paragraph, inner_iface);
    }

    // Traffic graphs
    render_traffic_graphs(f, chunks[1], data, theme);

    // Totals row
    if let Some(iface) = data.interfaces.get(iface_idx) {
        let (rx_display, tx_display) = traffic_display(iface, ui);
        let marker_hint = if ui.traffic_marker.is_some() {
            if ui.show_marker_traffic {
                " [0: show global]"
            } else {
                " [0: show since mark]"
            }
        } else {
            " [0: set mark]"
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.network_color));
        let line = Line::from(vec![
            Span::styled("RX: ", dim()),
            Span::styled(&rx_display, Style::default().fg(Color::Green)),
            Span::raw("  "),
            Span::styled("TX: ", dim()),
            Span::styled(&tx_display, Style::default().fg(Color::Cyan)),
            Span::raw("  "),
            Span::styled(
                format!(
                    "Peak: \u{2193}{:.1} \u{2191}{:.1} Mbps",
                    iface.peak_download, iface.peak_upload
                ),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(marker_hint, Style::default().fg(Color::DarkGray)),
        ]);
        let p = Paragraph::new(line).block(block);
        f.render_widget(p, chunks[2]);
    }
}

fn render_center_connections(
    f: &mut Frame,
    area: Rect,
    data: &NetworkData,
    ui: &NetworkUIState,
    _theme: &Theme,
    border_color: Color,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(10),  // connections table
            Constraint::Length(8), // bandwidth consumers
        ])
        .split(area);

    // Connections table
    let header = Row::new(vec!["Process", "PID", "Proto", "Local", "Remote", "State"])
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    let visible_conns: Vec<Row> = data
        .connections
        .iter()
        .skip(ui.connections_scroll)
        .take(chunks[0].height.saturating_sub(3) as usize)
        .map(|conn| {
            Row::new(vec![
                conn.process_name.clone(),
                format!("{}", conn.pid),
                conn.protocol.clone(),
                format!("{}:{}", conn.local_address, conn.local_port),
                format!("{}:{}", conn.remote_address, conn.remote_port),
                conn.state.clone(),
            ])
            .style(Style::default().fg(Color::White))
        })
        .collect();

    let widths = [
        Constraint::Percentage(18),
        Constraint::Percentage(7),
        Constraint::Percentage(8),
        Constraint::Percentage(25),
        Constraint::Percentage(28),
        Constraint::Percentage(14),
    ];

    let table = Table::new(visible_conns, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(
                    "Connections ({}) [V] toggle view",
                    data.connections.len()
                ))
                .border_style(Style::default().fg(border_color)),
        )
        .column_spacing(1);
    f.render_widget(table, chunks[0]);

    // Bandwidth consumers
    let bw_header = Row::new(vec!["Process", "PID", "DL", "UL", "Total RX", "Total TX"])
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    let bw_rows: Vec<Row> = data
        .bandwidth_consumers
        .iter()
        .take(5)
        .map(|c| {
            let pfx = if c.estimated { "~" } else { "" };
            Row::new(vec![
                format!("{}{}", pfx, c.process_name),
                format!("{}", c.pid),
                format!("{}{:.1}", pfx, c.download_speed),
                format!("{}{:.1}", pfx, c.upload_speed),
                format!("{}{}", pfx, format_bytes(c.total_bytes_received)),
                format!("{}{}", pfx, format_bytes(c.total_bytes_sent)),
            ])
            .style(Style::default().fg(Color::White))
        })
        .collect();

    let bw_widths = [
        Constraint::Percentage(22),
        Constraint::Percentage(10),
        Constraint::Percentage(14),
        Constraint::Percentage(14),
        Constraint::Percentage(20),
        Constraint::Percentage(20),
    ];

    let bw_table = Table::new(bw_rows, bw_widths)
        .header(bw_header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(
                    "Bandwidth Consumers (Top {})",
                    data.bandwidth_consumers.len().min(5)
                ))
                .border_style(Style::default().fg(border_color)),
        )
        .column_spacing(1);
    f.render_widget(bw_table, chunks[1]);
}

// ─────────────────────────── traffic graphs ────────────────────────

fn render_traffic_graphs(f: &mut Frame, area: Rect, data: &NetworkData, _theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Download
    if !data.traffic_history.is_empty() {
        let dl_data: Vec<u64> = data
            .traffic_history
            .iter()
            .map(|s| (s.download_mbps * 100.0) as u64)
            .collect();
        let max_dl = dl_data.iter().max().copied().unwrap_or(1).max(1);

        let sparkline = Sparkline::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Download (peak {:.2} Mbps)", max_dl as f64 / 100.0))
                    .border_style(Style::default().fg(Color::Green)),
            )
            .data(&dl_data)
            .style(Style::default().fg(Color::Green))
            .max(max_dl);
        f.render_widget(sparkline, chunks[0]);
    } else {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Download")
            .border_style(Style::default().fg(Color::Green));
        f.render_widget(Paragraph::new("Collecting...").block(block), chunks[0]);
    }

    // Upload
    if !data.traffic_history.is_empty() {
        let ul_data: Vec<u64> = data
            .traffic_history
            .iter()
            .map(|s| (s.upload_mbps * 100.0) as u64)
            .collect();
        let max_ul = ul_data.iter().max().copied().unwrap_or(1).max(1);

        let sparkline = Sparkline::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Upload (peak {:.2} Mbps)", max_ul as f64 / 100.0))
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .data(&ul_data)
            .style(Style::default().fg(Color::Cyan))
            .max(max_ul);
        f.render_widget(sparkline, chunks[1]);
    } else {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Upload")
            .border_style(Style::default().fg(Color::Cyan));
        f.render_widget(Paragraph::new("Collecting...").block(block), chunks[1]);
    }
}

// ─────────────────────────── results panel (right) ─────────────────

fn render_results_panel(f: &mut Frame, area: Rect, ui: &NetworkUIState, theme: &Theme) {
    let is_focused = ui.focus == NetworkFocusZone::Results;
    let border_color = if is_focused {
        Color::Yellow
    } else {
        theme.network_color
    };

    // Build tab header
    let tab_labels: Vec<Span> = NetworkResultTab::TABS
        .iter()
        .map(|tab| {
            let label = tab.label();
            if *tab == ui.result_tab {
                Span::styled(
                    format!("[{}]", label),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(format!(" {} ", label), Style::default().fg(Color::DarkGray))
            }
        })
        .collect();

    let mut tab_line_spans = vec![Span::raw("RESULTS > ")];
    tab_line_spans.extend(tab_labels);
    let title_line = Line::from(tab_line_spans);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(if is_focused { "RESULTS [*]" } else { "RESULTS" })
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 3 || inner.height < 3 {
        return;
    }

    // Split: tab header line + content
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(2)])
        .split(inner);

    f.render_widget(Paragraph::new(title_line), sections[0]);

    let content_lines: Vec<Line> = match ui.result_tab {
        NetworkResultTab::Summary => render_result_summary_lines(ui),
        NetworkResultTab::Details => render_result_details_lines(ui),
        NetworkResultTab::Raw => render_result_raw_lines(ui),
        NetworkResultTab::Advice => render_result_advice_lines(ui),
        NetworkResultTab::History => render_result_history_lines(ui),
    };

    let max_scroll = content_lines
        .len()
        .saturating_sub(sections[1].height as usize);
    let scroll = ui.detail_scroll.min(max_scroll).min(u16::MAX as usize) as u16;

    let paragraph = Paragraph::new(content_lines)
        .scroll((scroll, 0))
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, sections[1]);
}

fn render_result_summary_lines(ui: &NetworkUIState) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    lines.push(Line::from(vec![
        Span::styled("Tool: ", dim()),
        Span::styled(
            ui.selected_tool.label().to_string(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    if let Some(job_id) = ui.last_job {
        lines.push(Line::from(vec![
            Span::styled("Job: ", dim()),
            Span::styled(format!("#{job_id}"), Style::default().fg(Color::White)),
        ]));
    }

    let verdict_color = if ui.last_error.is_some() {
        Color::Red
    } else if ui.last_summary.contains("PARTIAL") || ui.last_summary.contains("WARN") {
        Color::Yellow
    } else {
        Color::Green
    };

    lines.push(Line::from(vec![
        Span::styled("Verdict: ", dim()),
        Span::styled(
            ui.last_summary.clone(),
            Style::default()
                .fg(verdict_color)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    if let Some(err) = &ui.last_error {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("Error: ", Style::default().fg(Color::Red)),
            Span::styled(err.clone(), Style::default().fg(Color::Red)),
        ]));
    }

    lines
}

fn render_result_details_lines(ui: &NetworkUIState) -> Vec<Line<'static>> {
    if ui.detail_lines.is_empty() {
        return vec![Line::from(Span::styled(
            "No detailed result yet. Run a diagnostic.",
            Style::default().fg(Color::DarkGray),
        ))];
    }
    ui.detail_lines
        .iter()
        .map(|line| Line::from(Span::styled(line.clone(), Style::default().fg(Color::White))))
        .collect()
}

fn render_result_raw_lines(ui: &NetworkUIState) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    lines.push(Line::from(Span::styled(
        "stdout:",
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD),
    )));

    if ui.raw_stdout.is_empty() {
        lines.push(Line::from(Span::styled(
            "<empty>",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for l in &ui.raw_stdout {
            lines.push(Line::from(Span::styled(
                l.clone(),
                Style::default().fg(Color::White),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "stderr:",
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD),
    )));

    if ui.raw_stderr.is_empty() {
        lines.push(Line::from(Span::styled(
            "<empty>",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for l in &ui.raw_stderr {
            lines.push(Line::from(Span::styled(
                l.clone(),
                Style::default().fg(Color::Red),
            )));
        }
    }

    lines
}

fn render_result_advice_lines(ui: &NetworkUIState) -> Vec<Line<'static>> {
    if ui.advice_lines.is_empty() {
        return vec![Line::from(Span::styled(
            "Run a diagnostic to get advice.",
            Style::default().fg(Color::DarkGray),
        ))];
    }

    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "Diagnosis & Recommendations:",
        Style::default()
            .fg(Color::Gray)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    for (i, advice) in ui.advice_lines.iter().enumerate() {
        lines.push(Line::from(Span::styled(
            format!("{}. {}", i + 1, advice),
            Style::default().fg(Color::White),
        )));
    }

    lines
}

fn render_result_history_lines(ui: &NetworkUIState) -> Vec<Line<'static>> {
    if ui.result_history.is_empty() {
        return vec![Line::from(Span::styled(
            "No history yet.",
            Style::default().fg(Color::DarkGray),
        ))];
    }

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            format!("{:<10}{:<14}{:<18}{}",
                "Time", "Tool", "Target", "Verdict"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    for entry in ui.result_history.iter().rev() {
        let target_display = truncate_str(&entry.target, 16);
        let summary_display = truncate_str(&entry.summary, 40);
        lines.push(Line::from(Span::styled(
            format!(
                "{:<10}{:<14}{:<18}{}",
                entry.timestamp, entry.tool_label, target_display, summary_display
            ),
            Style::default().fg(Color::White),
        )));
    }

    lines
}

// ─────────────────────────── parameters panel (bottom-left) ────────

fn render_parameters_panel(f: &mut Frame, area: Rect, ui: &NetworkUIState, theme: &Theme) {
    let is_focused = ui.focus == NetworkFocusZone::Parameters;
    let border_color = if is_focused {
        Color::Yellow
    } else {
        theme.network_color
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(if is_focused {
            "PARAMETERS [*]"
        } else {
            "PARAMETERS"
        })
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 3 || inner.height < 2 {
        return;
    }

    let mut lines = Vec::new();

    // Tool name
    lines.push(Line::from(vec![
        Span::styled("Tool: ", dim()),
        Span::styled(
            ui.selected_tool.label().to_string(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Target input field
    let input_indicator = if ui.input_mode { " [EDITING]" } else { "" };
    let cursor = if ui.input_mode { "_" } else { "" };
    lines.push(Line::from(vec![
        Span::styled("Target: ", dim()),
        Span::styled("[", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{}{}", ui.target_input, cursor),
            if ui.input_mode {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            },
        ),
        Span::styled("]", Style::default().fg(Color::DarkGray)),
        Span::styled(
            input_indicator.to_string(),
            Style::default().fg(Color::Magenta),
        ),
    ]));

    // Hint for the selected tool
    lines.push(Line::from(vec![
        Span::styled("Hint: ", dim()),
        Span::styled(
            diagnostic_input_hint(ui.selected_tool).to_string(),
            Style::default().fg(Color::DarkGray),
        ),
    ]));

    // NAT mapping confirmation
    if let Some(until) = ui.nat_mapping_confirm_until {
        if until > std::time::Instant::now() {
            lines.push(Line::from(Span::styled(
                "Confirm: Press Enter again to run NAT Mapping Test",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
        }
    }

    // Action buttons hint
    lines.push(Line::from(vec![
        Span::styled("[Enter] Run  ", Style::default().fg(Color::Green)),
        Span::styled("[x] Cancel  ", Style::default().fg(Color::Red)),
        Span::styled("[i] Edit target", Style::default().fg(Color::Cyan)),
    ]));

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}

// ─────────────────────────── activity panel (bottom-right) ─────────

fn render_activity_panel(f: &mut Frame, area: Rect, ui: &NetworkUIState, theme: &Theme) {
    let is_focused = ui.focus == NetworkFocusZone::Activity;
    let border_color = if is_focused {
        Color::Yellow
    } else {
        theme.network_color
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(if is_focused {
            "ACTIVITY [*]"
        } else {
            "ACTIVITY"
        })
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 3 || inner.height < 2 {
        return;
    }

    let visible_height = inner.height as usize;
    let lines: Vec<Line> = ui
        .event_log
        .iter()
        .rev()
        .take(visible_height)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|event| {
            let color = if event.contains("failed") || event.contains("Error") {
                Color::Red
            } else if event.contains("completed") || event.contains("OK") {
                Color::Green
            } else if event.contains("started") || event.contains("Queued") {
                Color::Cyan
            } else {
                Color::DarkGray
            };
            Line::from(Span::styled((*event).clone(), Style::default().fg(color)))
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}

// ─────────────────────────── help bar ──────────────────────────────

fn render_help_bar(f: &mut Frame, area: Rect, ui: &NetworkUIState, _theme: &Theme) {
    let focus_label = match ui.focus {
        NetworkFocusZone::Tools => "Tools",
        NetworkFocusZone::Interface => "Interface",
        NetworkFocusZone::Results => "Results",
        NetworkFocusZone::Parameters => "Params",
        NetworkFocusZone::Activity => "Activity",
    };

    let view_label = match ui.center_view {
        NetworkCenterView::Interface => "Iface",
        NetworkCenterView::Connections => "Conns",
    };

    let line = Line::from(vec![
        Span::styled(
            format!(" Focus: [{}] ", focus_label),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("Tab/S-Tab: switch", Style::default().fg(Color::DarkGray)),
        Span::raw(" | "),
        Span::styled("\u{2191}\u{2193}: navigate", Style::default().fg(Color::DarkGray)),
        Span::raw(" | "),
        Span::styled("\u{2190}\u{2192}: result tabs", Style::default().fg(Color::DarkGray)),
        Span::raw(" | "),
        Span::styled("Enter: run", Style::default().fg(Color::DarkGray)),
        Span::raw(" | "),
        Span::styled(
            format!("v: {}", view_label),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw(" | "),
        Span::styled("0: RX/TX mark", Style::default().fg(Color::DarkGray)),
        Span::raw(" | "),
        Span::styled("k: clear log", Style::default().fg(Color::DarkGray)),
    ]);

    let paragraph = Paragraph::new(line);
    f.render_widget(paragraph, area);
}

// ─────────────────────────── compact helpers ───────────────────────

fn render_compact_tool_params(f: &mut Frame, area: Rect, ui: &NetworkUIState, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Tool + Params")
        .border_style(Style::default().fg(theme.network_color));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 3 || inner.height < 2 {
        return;
    }

    let cursor = if ui.input_mode { "_" } else { "" };
    let mut lines = vec![
        Line::from(vec![
            Span::styled("Tool: ", dim()),
            Span::styled(
                ui.selected_tool.label().to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" [\u{2191}\u{2193}]"),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled("Target: [", dim()),
            Span::styled(
                format!("{}{}", ui.target_input, cursor),
                Style::default().fg(Color::White),
            ),
            Span::styled("]", dim()),
        ]),
        Line::from(vec![
            Span::styled("[Enter] Run  ", Style::default().fg(Color::Green)),
            Span::styled("[x] Cancel", Style::default().fg(Color::Red)),
        ]),
    ];

    if let Some(err) = &ui.last_error {
        lines.push(Line::from(Span::styled(
            truncate_str(err, inner.width as usize),
            Style::default().fg(Color::Red),
        )));
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}

fn render_compact_result(f: &mut Frame, area: Rect, ui: &NetworkUIState, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Result")
        .border_style(Style::default().fg(theme.network_color));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 3 || inner.height < 2 {
        return;
    }

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("Summary: ", dim()),
        Span::styled(
            truncate_str(&ui.last_summary, inner.width.saturating_sub(10) as usize),
            Style::default().fg(Color::White),
        ),
    ]));

    // Show first few detail lines
    for line in ui.detail_lines.iter().take(inner.height.saturating_sub(2) as usize) {
        lines.push(Line::from(Span::styled(
            truncate_str(line, inner.width as usize),
            Style::default().fg(Color::White),
        )));
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}

fn render_compact_bottom(
    f: &mut Frame,
    area: Rect,
    data: &NetworkData,
    ui: &NetworkUIState,
    theme: &Theme,
) {
    let iface = primary_interface(data);
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Network Info [v] toggle")
        .border_style(Style::default().fg(theme.network_color));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 3 || inner.height < 2 {
        return;
    }

    match ui.center_view {
        NetworkCenterView::Interface => {
            if let Some(iface) = iface {
                let (rx_display, tx_display) = traffic_display(iface, ui);
                let lines = vec![
                    Line::from(vec![
                        Span::styled(&iface.name, Style::default().fg(Color::Cyan)),
                        Span::raw(": "),
                        Span::styled(&iface.ipv4_address, Style::default().fg(Color::White)),
                        Span::raw("  GW: "),
                        Span::styled(&iface.gateway, Style::default().fg(Color::White)),
                    ]),
                    Line::from(vec![
                        Span::styled(
                            format!("\u{2193}{:.1} \u{2191}{:.1} Mbps", iface.download_speed, iface.upload_speed),
                            Style::default().fg(Color::Green),
                        ),
                        Span::raw("  "),
                        Span::styled(format!("RX:{} TX:{}", rx_display, tx_display), Style::default().fg(Color::DarkGray)),
                    ]),
                ];
                let paragraph = Paragraph::new(lines);
                f.render_widget(paragraph, inner);
            }
        }
        NetworkCenterView::Connections => {
            let lines: Vec<Line> = data
                .connections
                .iter()
                .take(inner.height as usize)
                .map(|c| {
                    Line::from(vec![
                        Span::styled(
                            format!("{:<12}", truncate_str(&c.process_name, 11)),
                            Style::default().fg(Color::White),
                        ),
                        Span::styled(
                            format!("{:<6}", c.protocol),
                            Style::default().fg(Color::Cyan),
                        ),
                        Span::styled(
                            format!("{}:{}", c.remote_address, c.remote_port),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ])
                })
                .collect();
            let paragraph = Paragraph::new(lines);
            f.render_widget(paragraph, inner);
        }
    }
}

// ─────────────────────────── utility functions ─────────────────────

fn primary_interface(data: &NetworkData) -> Option<&crate::monitors::NetworkInterface> {
    data.interfaces
        .iter()
        .max_by(|a, b| {
            let a_score = interface_score(a);
            let b_score = interface_score(b);
            a_score
                .partial_cmp(&b_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .or_else(|| data.interfaces.first())
}

fn interface_score(iface: &crate::monitors::NetworkInterface) -> f64 {
    let mut score = 0.0;
    if iface.name != "lo" {
        score += 1000.0;
    }
    if iface.status.eq_ignore_ascii_case("connected") {
        score += 400.0;
    }
    if !iface.gateway.is_empty() {
        score += 300.0;
    }
    if !iface.ipv4_address.is_empty() {
        score += 200.0;
    }
    if !iface.ipv6_address.is_empty() {
        score += 100.0;
    }
    score + iface.download_speed + iface.upload_speed
}

/// Compute RX/TX display values respecting the traffic marker
fn traffic_display(
    iface: &crate::monitors::NetworkInterface,
    ui: &NetworkUIState,
) -> (String, String) {
    if ui.show_marker_traffic {
        if let Some(marker) = &ui.traffic_marker {
            let rx = iface.bytes_received.saturating_sub(marker.bytes_received_at_mark);
            let tx = iface.bytes_sent.saturating_sub(marker.bytes_sent_at_mark);
            return (format!("\u{0394}{}", format_bytes(rx)), format!("\u{0394}{}", format_bytes(tx)));
        }
    }
    (format_bytes(iface.bytes_received), format_bytes(iface.bytes_sent))
}

fn dim() -> Style {
    Style::default().fg(Color::Gray)
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else if max_len > 3 {
        format!("{}...", &s[..max_len - 3])
    } else {
        s[..max_len].to_string()
    }
}

fn line_text(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

fn diagnostic_input_hint(tool: NetworkDiagnosticTool) -> &'static str {
    match tool {
        NetworkDiagnosticTool::Resolve => "host/IP (example.org, 1.1.1.1)",
        NetworkDiagnosticTool::DnsExplain => "input ignored; inspects system DNS stack",
        NetworkDiagnosticTool::RouteInspect => {
            "optional target for `ip route get`"
        }
        NetworkDiagnosticTool::NicDeepInfo => {
            "optional interface (eth0, wlan0); empty = all"
        }
        NetworkDiagnosticTool::ConnectionLab => {
            "proto=tcp state=estab limit=200"
        }
        NetworkDiagnosticTool::Ping => {
            "target [profile=quick|latency|loss] [count=N]"
        }
        NetworkDiagnosticTool::Trace => {
            "target [proto=icmp|udp|tcp] [hops=N]"
        }
        NetworkDiagnosticTool::MtuProbe => "target host/IP for PMTU probing",
        NetworkDiagnosticTool::PortScan => "host[:ports] e.g. example.org:22,80,443",
        NetworkDiagnosticTool::NatCapability => "input ignored; probes UPnP/NAT-PMP/PCP",
        NetworkDiagnosticTool::NatMappingTest => {
            "tcp 8080 8080 120 (proto in out ttl)"
        }
        NetworkDiagnosticTool::ExportReport => "input ignored; exports diagnostics report",
    }
}
