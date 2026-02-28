use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Row, Sparkline, Table, Wrap},
    Frame,
};

use crate::app::state::{
    NetworkCenterView, NetworkDiagnosticTool, NetworkFocusZone, NetworkResultTab, NetworkUIState,
    ToolCategory,
};
use crate::app::App;
use crate::monitors::{NetworkData, NetworkInterface};
use crate::ui::theme::Theme;
use crate::utils::format::format_bytes;

// ═══════════════════════════ ENTRY POINT ═══════════════════════════

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

// ═══════════════════════════ FULL VIEW ═══════════════════════════

fn render_full(
    f: &mut Frame,
    area: Rect,
    data: &NetworkData,
    ui: &NetworkUIState,
    theme: &Theme,
) {
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // header
            Constraint::Min(16),   // body (3-column)
            Constraint::Length(8),  // bottom (params + activity)
            Constraint::Length(1), // help bar
        ])
        .split(area);

    render_header_bar(f, main_chunks[0], data, ui, theme);

    // Body: 3-column
    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(22), // Tools
            Constraint::Min(28),    // Center
            Constraint::Min(34),    // Results
        ])
        .split(main_chunks[1]);

    render_tools_panel(f, body_chunks[0], ui, theme);
    render_center_panel(f, body_chunks[1], data, ui, theme);
    render_results_panel(f, body_chunks[2], ui, theme);

    // Bottom: params + activity
    let bottom_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(38), // parameters
            Constraint::Percentage(62), // activity
        ])
        .split(main_chunks[2]);

    render_parameters_panel(f, bottom_chunks[0], ui, theme);
    render_activity_panel(f, bottom_chunks[1], ui, theme);

    render_help_bar(f, main_chunks[3], ui);
}

// ═══════════════════════════ COMPACT VIEW ═══════════════════════════

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
            Constraint::Length(7),  // tool+params | result
            Constraint::Min(6),    // interface/connections
            Constraint::Length(1), // help
        ])
        .split(area);

    render_header_bar(f, chunks[0], data, ui, theme);

    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(chunks[1]);

    render_compact_tool_params(f, mid[0], ui, theme);
    render_compact_result(f, mid[1], ui, theme);
    render_compact_bottom(f, chunks[2], data, ui, theme);
    render_help_bar(f, chunks[3], ui);
}

// ═══════════════════════════ HEADER BAR ═══════════════════════════

fn render_header_bar(
    f: &mut Frame,
    area: Rect,
    data: &NetworkData,
    ui: &NetworkUIState,
    theme: &Theme,
) {
    let iface = primary_interface(data);
    // Only show job info when a job is running or has run
    let job_span: Option<Span> = if let Some(id) = ui.running_job {
        Some(Span::styled(
            format!("Job #{id} running"),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ))
    } else if let Some(id) = ui.last_job {
        Some(Span::styled(
            format!("Last:#{id}"),
            Style::default().fg(Color::DarkGray),
        ))
    } else {
        None
    };

    let marker_label = if ui.show_marker_traffic {
        " [\u{0394}]"
    } else {
        ""
    };

    let spans: Vec<Span> = if let Some(iface) = iface {
        let (rx_display, tx_display) = traffic_display(iface, ui);
        let mut spans = vec![
            Span::styled(
                format!(" {} ", iface.name),
                Style::default()
                    .fg(Color::Black)
                    .bg(if iface.status.eq_ignore_ascii_case("connected") {
                        Color::Green
                    } else {
                        Color::Red
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(
                format!("\u{2193}{:.2}", iface.download_speed),
                Style::default().fg(Color::Green),
            ),
            Span::styled(" Mbps ", dim()),
            Span::styled(
                format!("\u{2191}{:.2}", iface.upload_speed),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(" Mbps", dim()),
            Span::styled(" \u{2502} ", Style::default().fg(Color::DarkGray)),
            Span::styled("GW ", dim()),
            Span::styled(
                if iface.gateway.is_empty() {
                    "N/A"
                } else {
                    &iface.gateway
                },
                Style::default().fg(Color::White),
            ),
            Span::styled(" \u{2502} ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("Conns:{}", data.connections.len()), dim()),
            Span::styled(" \u{2502} ", Style::default().fg(Color::DarkGray)),
            Span::styled("RX:", dim()),
            Span::styled(rx_display, Style::default().fg(Color::Green)),
            Span::raw(" "),
            Span::styled("TX:", dim()),
            Span::styled(tx_display, Style::default().fg(Color::Cyan)),
            Span::styled(marker_label, Style::default().fg(Color::Yellow)),
        ];
        if let Some(js) = job_span {
            spans.push(Span::styled(" \u{2502} ", Style::default().fg(Color::DarkGray)));
            spans.push(js);
        }
        spans
    } else {
        vec![
            Span::styled(
                " NO IFACE ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" No active network interfaces"),
        ]
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.network_color));
    let paragraph = Paragraph::new(Line::from(spans)).block(block);
    f.render_widget(paragraph, area);
}

// ═══════════════════════════ TOOLS PANEL ═══════════════════════════

fn render_tools_panel(f: &mut Frame, area: Rect, ui: &NetworkUIState, theme: &Theme) {
    let is_focused = ui.focus == NetworkFocusZone::Tools;
    let border_color = zone_border(is_focused, theme);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(zone_title("TOOLS", is_focused))
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 4 || inner.height < 3 {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    let mut last_category: Option<ToolCategory> = None;

    for tool in NetworkDiagnosticTool::ORDERED.iter() {
        let cat = tool.category();
        if last_category != Some(cat) {
            if last_category.is_some() && inner.height > 20 {
                // only add spacing if we have enough room
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(
                cat.label(),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )));
            last_category = Some(cat);
        }

        let is_selected = *tool == ui.selected_tool;
        let style = if is_selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let marker = if is_selected { "\u{25b6} " } else { "  " };
        lines.push(Line::from(Span::styled(
            format!("{}{}", marker, tool.label()),
            style,
        )));
    }

    // Scroll to keep selected tool visible
    let selected_pos = lines
        .iter()
        .position(|l| line_plain_text(l).contains('\u{25b6}'))
        .unwrap_or(0);
    let visible_h = inner.height as usize;
    let scroll = if selected_pos >= visible_h {
        selected_pos.saturating_sub(visible_h / 2)
    } else {
        0
    };

    let p = Paragraph::new(lines).scroll((scroll as u16, 0));
    f.render_widget(p, inner);
}

// ═══════════════════════════ CENTER PANEL ═══════════════════════════

fn render_center_panel(
    f: &mut Frame,
    area: Rect,
    data: &NetworkData,
    ui: &NetworkUIState,
    theme: &Theme,
) {
    match ui.center_view {
        NetworkCenterView::Interface => render_center_interface(f, area, data, ui, theme),
        NetworkCenterView::Connections => render_center_connections(f, area, data, ui, theme),
    }
}

fn render_center_interface(
    f: &mut Frame,
    area: Rect,
    data: &NetworkData,
    ui: &NetworkUIState,
    theme: &Theme,
) {
    let is_focused = ui.focus == NetworkFocusZone::Interface;
    let bc = zone_border(is_focused, theme);

    let iface_count = data.interfaces.len();
    let iface_idx = ui.selected_interface_idx.min(iface_count.saturating_sub(1));
    let iface = data.interfaces.get(iface_idx);

    // Multi-interface: show selector strip + details + graphs + stats
    let has_multi = iface_count > 1;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if has_multi {
            vec![
                Constraint::Length(2),  // interface selector strip
                Constraint::Length(7),  // details
                Constraint::Min(5),    // graphs
                Constraint::Length(3),  // totals + stats
            ]
        } else {
            vec![
                Constraint::Length(0),  // no selector
                Constraint::Length(8),  // details
                Constraint::Min(5),    // graphs
                Constraint::Length(3),  // totals
            ]
        })
        .split(area);

    // ---- Interface selector strip (multi-interface) ----
    if has_multi {
        let mut iface_tabs: Vec<Span> = Vec::new();
        for (i, ifc) in data.interfaces.iter().enumerate() {
            let is_sel = i == iface_idx;
            let iface_type = detect_iface_type(&ifc.name);
            // Build label with separate type tag and name for correct spacing
            let label = format!(" {}{} ", iface_type, ifc.name);
            if is_sel {
                iface_tabs.push(Span::styled(
                    label,
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                let status_fg = if ifc.status.eq_ignore_ascii_case("connected") {
                    Color::Green
                } else {
                    Color::DarkGray
                };
                iface_tabs.push(Span::styled(label, Style::default().fg(status_fg)));
            }
            if i + 1 < iface_count {
                iface_tabs.push(Span::styled(" \u{2502} ", Style::default().fg(Color::DarkGray)));
            }
        }
        let sel_block = Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(bc));
        let sel_inner = sel_block.inner(chunks[0]);
        f.render_widget(sel_block, chunks[0]);
        f.render_widget(
            Paragraph::new(Line::from(iface_tabs)),
            sel_inner,
        );
    }

    // ---- Interface details ----
    let title = if let Some(iface) = iface {
        format!(
            "{} ({}/{}) \u{2502} [\u{2190}\u{2192}]panel [v]view [\u{2191}\u{2193}]iface",
            iface.name,
            iface_idx + 1,
            iface_count
        )
    } else {
        "[v] toggle view".to_string()
    };

    let det_block = Block::default()
        .borders(Borders::ALL)
        .title(zone_title(&title, is_focused))
        .border_style(Style::default().fg(bc));
    let det_inner = det_block.inner(chunks[1]);
    f.render_widget(det_block, chunks[1]);

    if let Some(iface) = iface {
        let status_color = if iface.status.eq_ignore_ascii_case("connected") {
            Color::Green
        } else {
            Color::Red
        };
        let mut lines = vec![
            Line::from(vec![
                Span::styled(
                    format!(" {} ", iface.status),
                    Style::default()
                        .fg(Color::Black)
                        .bg(status_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  Speed: ", dim()),
                Span::styled(&iface.link_speed, Style::default().fg(Color::Cyan)),
                Span::styled("  Duplex: ", dim()),
                Span::styled(&iface.duplex, Style::default().fg(Color::White)),
                Span::styled("  IF-MTU: ", dim()),
                Span::styled(
                    format!("{} B", iface.mtu),
                    Style::default().fg(Color::White),
                ),
            ]),
            Line::from(vec![
                Span::styled("IPv4: ", dim()),
                Span::styled(
                    if iface.ipv4_address.is_empty() {
                        "N/A"
                    } else {
                        &iface.ipv4_address
                    },
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled("  IPv6: ", dim()),
                Span::styled(
                    trunc(&iface.ipv6_address, 30),
                    Style::default().fg(Color::Cyan),
                ),
            ]),
            Line::from(vec![
                Span::styled("Gateway: ", dim()),
                Span::styled(
                    if iface.gateway.is_empty() {
                        "N/A"
                    } else {
                        &iface.gateway
                    },
                    Style::default().fg(Color::White),
                ),
                Span::styled("  MAC: ", dim()),
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
        ];
        if !iface.description.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("Type: ", dim()),
                Span::styled(
                    detect_iface_type_label(&iface.name),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled("  Desc: ", dim()),
                Span::styled(&iface.description, Style::default().fg(Color::DarkGray)),
            ]));
        }
        let p = Paragraph::new(lines);
        f.render_widget(p, det_inner);
    }

    // ---- Traffic graphs ----
    render_traffic_graphs(f, chunks[2], data, theme);

    // ---- Totals + stats row ----
    if let Some(iface) = data.interfaces.get(iface_idx) {
        let (rx_disp, tx_disp) = traffic_display(iface, ui);
        let marker_hint = if ui.show_marker_traffic {
            "[0] global"
        } else if ui.traffic_marker.is_some() {
            "[0] since mark"
        } else {
            "[0] set mark"
        };
        let tot_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(bc));
        let line = Line::from(vec![
            Span::styled(" RX:", dim()),
            Span::styled(format!("{} ", rx_disp), Style::default().fg(Color::Green)),
            Span::styled("TX:", dim()),
            Span::styled(format!("{} ", tx_disp), Style::default().fg(Color::Cyan)),
            Span::styled(" \u{2502} ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(
                    "Peak:\u{2193}{:.1} \u{2191}{:.1} Mbps",
                    iface.peak_download, iface.peak_upload
                ),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(" \u{2502} ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("Pkt:\u{2193}{} \u{2191}{}",
                    format_pkt_count(iface.bytes_received),
                    format_pkt_count(iface.bytes_sent),
                ),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(format!(" {}", marker_hint), Style::default().fg(Color::DarkGray)),
        ]);
        let p = Paragraph::new(line).block(tot_block);
        f.render_widget(p, chunks[3]);
    }
}

fn render_center_connections(
    f: &mut Frame,
    area: Rect,
    data: &NetworkData,
    ui: &NetworkUIState,
    theme: &Theme,
) {
    let is_focused = ui.focus == NetworkFocusZone::Interface;
    let bc = zone_border(is_focused, theme);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(8)])
        .split(area);

    // ---- Connections table ----
    let conn_header = Row::new(vec!["Process", "PID", "Proto", "Local", "Remote", "State"])
        .style(header_style());

    let max_rows = chunks[0].height.saturating_sub(4) as usize;
    let scroll = ui.connections_scroll.min(data.connections.len().saturating_sub(max_rows));

    let conn_rows: Vec<Row> = data
        .connections
        .iter()
        .skip(scroll)
        .take(max_rows)
        .map(|c| {
            let state_color = match c.state.as_str() {
                "ESTABLISHED" | "ESTAB" => Color::Green,
                "LISTEN" => Color::Cyan,
                "TIME_WAIT" | "TIME-WAIT" => Color::DarkGray,
                "CLOSE_WAIT" | "CLOSE-WAIT" => Color::Yellow,
                _ => Color::White,
            };
            Row::new(vec![
                trunc(&c.process_name, 14),
                c.pid.to_string(),
                c.protocol.clone(),
                format!("{}:{}", trunc(&c.local_address, 15), c.local_port),
                format!("{}:{}", trunc(&c.remote_address, 15), c.remote_port),
                c.state.clone(),
            ])
            .style(Style::default().fg(state_color))
        })
        .collect();

    let widths = [
        Constraint::Percentage(16),
        Constraint::Percentage(7),
        Constraint::Percentage(8),
        Constraint::Percentage(25),
        Constraint::Percentage(28),
        Constraint::Percentage(16),
    ];
    let conn_title = format!(
        "Connections ({}) [{}-{}] [v]iface",
        data.connections.len(),
        scroll + 1,
        (scroll + max_rows).min(data.connections.len()),
    );
    let conn_table = Table::new(conn_rows, widths)
        .header(conn_header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(zone_title(&conn_title, is_focused))
                .border_style(Style::default().fg(bc)),
        )
        .column_spacing(1);
    f.render_widget(conn_table, chunks[0]);

    // ---- Bandwidth consumers ----
    let bw_header = Row::new(vec!["Process", "PID", "\u{2193}Mbps", "\u{2191}Mbps", "RX", "TX"])
        .style(header_style());

    let bw_rows: Vec<Row> = data
        .bandwidth_consumers
        .iter()
        .take(5)
        .map(|c| {
            let pf = if c.estimated { "~" } else { "" };
            Row::new(vec![
                format!("{}{}", pf, trunc(&c.process_name, 13)),
                c.pid.to_string(),
                format!("{}{:.1}", pf, c.download_speed),
                format!("{}{:.1}", pf, c.upload_speed),
                format!("{}{}", pf, format_bytes(c.total_bytes_received)),
                format!("{}{}", pf, format_bytes(c.total_bytes_sent)),
            ])
            .style(Style::default().fg(Color::White))
        })
        .collect();

    let bw_widths = [
        Constraint::Percentage(20),
        Constraint::Percentage(10),
        Constraint::Percentage(14),
        Constraint::Percentage(14),
        Constraint::Percentage(21),
        Constraint::Percentage(21),
    ];
    let bw_table = Table::new(bw_rows, bw_widths)
        .header(bw_header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(
                    "Bandwidth (top {})",
                    data.bandwidth_consumers.len().min(5)
                ))
                .border_style(Style::default().fg(bc)),
        )
        .column_spacing(1);
    f.render_widget(bw_table, chunks[1]);
}

// ═══════════════════════════ TRAFFIC GRAPHS ═══════════════════════════

fn render_traffic_graphs(f: &mut Frame, area: Rect, data: &NetworkData, _theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_sparkline_graph(
        f,
        chunks[0],
        &data.traffic_history,
        |s| s.download_mbps,
        "Download",
        Color::Green,
    );
    render_sparkline_graph(
        f,
        chunks[1],
        &data.traffic_history,
        |s| s.upload_mbps,
        "Upload",
        Color::Cyan,
    );
}

fn render_sparkline_graph(
    f: &mut Frame,
    area: Rect,
    history: &std::collections::VecDeque<crate::monitors::TrafficSample>,
    extract: fn(&crate::monitors::TrafficSample) -> f64,
    label: &str,
    color: Color,
) {
    if history.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(label.to_string())
            .border_style(Style::default().fg(color));
        f.render_widget(
            Paragraph::new("Collecting data...").block(block),
            area,
        );
        return;
    }

    let vals: Vec<u64> = history.iter().map(|s| (extract(s) * 100.0) as u64).collect();
    let max_val = vals.iter().max().copied().unwrap_or(1).max(1);
    let max_mbps = max_val as f64 / 100.0;
    let current = vals.last().copied().unwrap_or(0) as f64 / 100.0;
    let avg_mbps = if vals.is_empty() {
        0.0
    } else {
        vals.iter().sum::<u64>() as f64 / (vals.len() as f64 * 100.0)
    };
    let samples = vals.len();

    let sparkline = Sparkline::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(
                    "{} {:.2} Mbps \u{2502} avg:{:.2} peak:{:.2} \u{2502} {}s",
                    label, current, avg_mbps, max_mbps, samples
                ))
                .border_style(Style::default().fg(color)),
        )
        .data(&vals)
        .style(Style::default().fg(color))
        .max(max_val);
    f.render_widget(sparkline, area);
}

// ═══════════════════════════ RESULTS PANEL ═══════════════════════════

fn render_results_panel(f: &mut Frame, area: Rect, ui: &NetworkUIState, theme: &Theme) {
    let is_focused = ui.focus == NetworkFocusZone::Results;
    let bc = zone_border(is_focused, theme);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(zone_title("RESULTS", is_focused))
        .border_style(Style::default().fg(bc));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 4 || inner.height < 4 {
        return;
    }

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(2)])
        .split(inner);

    // Tab header
    let tab_spans: Vec<Span> = NetworkResultTab::TABS
        .iter()
        .map(|tab| {
            if *tab == ui.result_tab {
                Span::styled(
                    format!(" [{}] ", tab.label()),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(
                    format!("  {}  ", tab.label()),
                    Style::default().fg(Color::DarkGray),
                )
            }
        })
        .collect();
    f.render_widget(Paragraph::new(Line::from(tab_spans)), sections[0]);

    // Tab content
    let lines = match ui.result_tab {
        NetworkResultTab::Summary => result_tab_summary(ui),
        NetworkResultTab::Details => result_tab_details(ui),
        NetworkResultTab::Raw => result_tab_raw(ui),
        NetworkResultTab::Advice => result_tab_advice(ui),
        NetworkResultTab::History => result_tab_history(ui),
    };

    let max_scroll = lines.len().saturating_sub(sections[1].height as usize);
    let scroll = ui.detail_scroll.min(max_scroll).min(u16::MAX as usize) as u16;
    let p = Paragraph::new(lines)
        .scroll((scroll, 0))
        .wrap(Wrap { trim: false });
    f.render_widget(p, sections[1]);
}

// ── Summary tab ──

fn result_tab_summary(ui: &NetworkUIState) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(Line::from(""));

    // Status badge + tool
    let (status_icon, status_text, status_color) = if ui.running_job.is_some() {
        ("\u{23f3}", "RUNNING", Color::Cyan)
    } else if ui.last_error.is_some() {
        ("\u{2718}", "FAILED", Color::Red)
    } else if ui.last_summary.contains("PARTIAL") || ui.last_summary.contains("WARN") {
        ("\u{26a0}", "PARTIAL", Color::Yellow)
    } else if ui.last_job.is_some() {
        ("\u{2714}", "OK", Color::Green)
    } else {
        ("\u{2500}", "IDLE", Color::DarkGray)
    };

    lines.push(Line::from(vec![
        Span::styled(
            format!(" {} {} ", status_icon, status_text),
            Style::default()
                .fg(Color::Black)
                .bg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            ui.selected_tool.label().to_string(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  [{}]", ui.selected_tool.category().label()),
            Style::default().fg(Color::DarkGray),
        ),
    ]));

    // Job info line
    if let Some(job_id) = ui.last_job {
        lines.push(Line::from(vec![
            Span::styled(" Job #", dim()),
            Span::styled(job_id.to_string(), Style::default().fg(Color::White)),
            if ui.running_job.is_some() {
                Span::styled("  \u{2502} in progress...", Style::default().fg(Color::Cyan))
            } else {
                Span::styled("  \u{2502} completed", Style::default().fg(Color::DarkGray))
            },
        ]));
    }

    lines.push(Line::from(Span::styled(
        format!(" {}", "\u{2500}".repeat(40)),
        Style::default().fg(Color::DarkGray),
    )));

    // Verdict/Summary — tool-specific rich display
    if !ui.last_summary.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(" Verdict: ", dim()),
            Span::styled(
                ui.last_summary.clone(),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    // Tool-specific summary metrics from detail_lines
    if !ui.detail_lines.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " Key Metrics:",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::UNDERLINED),
        )));
        // Show top 6 detail lines as key metrics
        for l in ui.detail_lines.iter().take(6) {
            if l.is_empty() { continue; }
            let (label, value) = if let Some(pos) = l.find('=') {
                (l[..pos].trim().to_string(), l[pos + 1..].trim().to_string())
            } else if let Some(pos) = l.find(':') {
                (l[..pos].trim().to_string(), l[pos + 1..].trim().to_string())
            } else {
                (String::new(), l.clone())
            };
            if !label.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled(format!("   {:<14}", label), dim()),
                    Span::styled(value, Style::default().fg(Color::Cyan)),
                ]));
            } else {
                lines.push(Line::from(Span::styled(
                    format!("   {}", value),
                    Style::default().fg(Color::White),
                )));
            }
        }
    }

    // Error
    if let Some(err) = &ui.last_error {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(" \u{2718} Error: ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::styled(
                err.clone(),
                Style::default().fg(Color::Red),
            ),
        ]));
    }

    // Next action hints
    if ui.last_job.is_some() && ui.running_job.is_none() && ui.last_error.is_none() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " Next Actions:",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::UNDERLINED),
        )));
        let next_hints = summary_next_actions(ui);
        for hint in next_hints {
            lines.push(Line::from(Span::styled(
                format!("   \u{2192} {}", hint),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    lines
}

fn summary_next_actions(ui: &NetworkUIState) -> Vec<&'static str> {
    match ui.selected_tool {
        NetworkDiagnosticTool::Ping => {
            if ui.last_summary.contains("loss") && !ui.last_summary.contains("loss 0.0%") {
                vec!["Run Trace+ to locate the lossy hop", "Run MTU Probe to check PMTU"]
            } else {
                vec!["Stable path — consider Trace+ for topology"]
            }
        }
        NetworkDiagnosticTool::Trace => {
            if ui.last_summary.contains("not reached") || ui.last_summary.contains("PARTIAL") {
                vec!["Try proto=tcp port=443 for firewall bypass", "Run Ping+ to confirm reachability"]
            } else {
                vec!["Path verified — run Port Scan to check services"]
            }
        }
        NetworkDiagnosticTool::Resolve => vec!["Run DNS Explain for full resolver analysis"],
        NetworkDiagnosticTool::DnsExplain => vec!["Use Resolve to test specific domains"],
        NetworkDiagnosticTool::PortScan => vec!["Run Connection Lab for socket details"],
        NetworkDiagnosticTool::MtuProbe => vec!["Check NIC Deep Info for interface MTU settings"],
        NetworkDiagnosticTool::NicDeepInfo => vec!["Run Route Inspect for routing table"],
        NetworkDiagnosticTool::RouteInspect => vec!["Run Trace+ to verify path"],
        NetworkDiagnosticTool::ConnectionLab => vec!["Run Port Scan to check remote services"],
        NetworkDiagnosticTool::NatCapability => vec!["Run NAT Mapping Test if UPnP available"],
        NetworkDiagnosticTool::NatMappingTest => vec!["Verify with Port Scan from external host"],
        NetworkDiagnosticTool::ExportReport => vec!["Report saved — review exported file"],
    }
}

// ── Details tab ──

fn result_tab_details(ui: &NetworkUIState) -> Vec<Line<'static>> {
    if ui.detail_lines.is_empty() {
        return vec![
            Line::from(""),
            Line::from(Span::styled(
                " No results yet. Select a tool and press Enter.",
                Style::default().fg(Color::DarkGray),
            )),
        ];
    }

    let mut lines = Vec::new();
    // Tool-specific header
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            format!(" {} ", ui.selected_tool.label()),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  [{}]", ui.selected_tool.category().label()),
            Style::default().fg(Color::DarkGray),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        format!(" {}", "\u{2500}".repeat(45)),
        Style::default().fg(Color::DarkGray),
    )));

    for l in &ui.detail_lines {
        // Try to parse "key: value" for structured display, but only for
        // top-level lines (not indented) with a clean key (no paths, URLs, etc.)
        let parsed = if !l.starts_with(' ') && !l.starts_with('\t') {
            // Try "key: value" first (but avoid splitting paths like /etc/resolv.conf)
            if let Some(pos) = l.find(": ") {
                let key = l[..pos].trim();
                let val = l[pos + 2..].trim();
                // Key must be short alphanumeric label (no slashes, dots, colons)
                if pos <= 24
                    && !key.is_empty()
                    && !key.contains('/')
                    && !key.contains('.')
                    && key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == ' ' || c == '-')
                {
                    Some((key.to_string(), val.to_string()))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        if let Some((key, val)) = parsed {
            // Color the value based on content
            let val_color = if val.contains("fail") || val.contains("error") || val == "false" {
                Color::Red
            } else if val.contains("warn") || val.contains("PARTIAL") || val.contains("could not") {
                Color::Yellow
            } else if val.contains("true") || val.contains("OK") || val == "0" {
                Color::Green
            } else {
                Color::Cyan
            };
            lines.push(Line::from(vec![
                Span::styled(format!(" {:<18} ", key), dim()),
                Span::styled(val, Style::default().fg(val_color)),
            ]));
        } else {
            // Fallback: color-code known prefixes
            let style = if l.starts_with("warning") || l.starts_with("WARN") {
                Style::default().fg(Color::Yellow)
            } else if l.starts_with("error") || l.starts_with("Error") {
                Style::default().fg(Color::Red)
            } else if l.starts_with("hop ") || l.starts_with("attempt ") {
                Style::default().fg(Color::Cyan)
            } else if l.starts_with("  ") {
                // Indented sub-items
                Style::default().fg(Color::DarkGray)
            } else if l.starts_with("blocked") || l.starts_with("conflicts") {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push(Line::from(Span::styled(format!(" {}", l), style)));
        }
    }
    lines
}

// ── Raw tab ──

fn result_tab_raw(ui: &NetworkUIState) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    lines.push(Line::from(""));

    let stdout_count = ui.raw_stdout.len();
    let stderr_count = ui.raw_stderr.len();
    lines.push(Line::from(vec![
        Span::styled(
            format!(" stdout ({} lines)", stdout_count),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ),
    ]));

    if ui.raw_stdout.is_empty() {
        lines.push(Line::from(Span::styled(
            "   <empty>",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (i, l) in ui.raw_stdout.iter().enumerate() {
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {:>3}\u{2502}", i + 1),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(format!(" {}", l), Style::default().fg(Color::White)),
            ]));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            format!(" stderr ({} lines)", stderr_count),
            Style::default()
                .fg(Color::Red)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ),
    ]));

    if ui.raw_stderr.is_empty() {
        lines.push(Line::from(Span::styled(
            "   <empty>",
            Style::default().fg(Color::DarkGray),
        )));
    } else {
        for (i, l) in ui.raw_stderr.iter().enumerate() {
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {:>3}\u{2502}", i + 1),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(format!(" {}", l), Style::default().fg(Color::Red)),
            ]));
        }
    }

    lines
}

// ── Advice tab ──

fn result_tab_advice(ui: &NetworkUIState) -> Vec<Line<'static>> {
    if ui.advice_lines.is_empty() {
        return vec![
            Line::from(""),
            Line::from(Span::styled(
                " Run a diagnostic to get recommendations.",
                Style::default().fg(Color::DarkGray),
            )),
        ];
    }

    let mut lines = Vec::new();
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            " Diagnosis & Recommendations",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ),
        Span::styled(
            format!("  ({} items)", ui.advice_lines.len()),
            Style::default().fg(Color::DarkGray),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        format!(" {}", "\u{2500}".repeat(40)),
        Style::default().fg(Color::DarkGray),
    )));

    for (i, a) in ui.advice_lines.iter().enumerate() {
        let (icon, color) = if a.contains("!") || a.starts_with("Significant") || a.starts_with("High") || a.contains("fail") {
            ("\u{2718}", Color::Red)   // cross mark for critical
        } else if a.contains("Consider") || a.contains("Try") || a.contains("Run") || a.contains("Check") {
            ("\u{2192}", Color::Cyan)  // arrow for actionable
        } else if a.contains("stable") || a.contains("reached") || a.contains("no packet loss") || a.contains("No") && a.contains("loss") {
            ("\u{2714}", Color::Green) // checkmark for good
        } else if a.contains("warn") || a.contains("low") || a.contains("Minor") || a.contains("filtered") {
            ("\u{26a0}", Color::Yellow) // warning
        } else {
            ("\u{2022}", Color::White) // bullet for info
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {} ", icon),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{}. ", i + 1),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(a.clone(), Style::default().fg(color)),
        ]));
    }

    lines
}

// ── History tab ──

fn result_tab_history(ui: &NetworkUIState) -> Vec<Line<'static>> {
    if ui.result_history.is_empty() {
        return vec![
            Line::from(""),
            Line::from(Span::styled(
                " No history yet. Results will appear here after diagnostics complete.",
                Style::default().fg(Color::DarkGray),
            )),
        ];
    }

    let mut lines = Vec::new();
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            format!(" History ({} entries)", ui.result_history.len()),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    // Header
    lines.push(Line::from(vec![
        Span::styled(
            format!(
                " {:<6}{:<10}{:<14}{:<16}{}",
                "Job", "Time", "Tool", "Target", "Result"
            ),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        format!(" {}", "\u{2500}".repeat(65)),
        Style::default().fg(Color::DarkGray),
    )));

    for entry in ui.result_history.iter().rev() {
        let (verdict_icon, verdict_color) = if entry.summary.contains("loss 0.0%")
            || entry.summary.contains("OK")
            || entry.summary.contains("open")
            || entry.summary.contains("reached")
        {
            ("\u{2714}", Color::Green)
        } else if entry.summary.contains("PARTIAL")
            || entry.summary.contains("WARN")
            || entry.summary.contains("could not")
        {
            ("\u{26a0}", Color::Yellow)
        } else if entry.summary.contains("fail") || entry.summary.contains("error") {
            ("\u{2718}", Color::Red)
        } else {
            ("\u{2022}", Color::White)
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!(" #{:<5}", entry.job_id),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("{:<10}", entry.timestamp),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("{:<14}", trunc(&entry.tool_label, 12)),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                format!("{:<16}", trunc(&entry.target, 14)),
                Style::default().fg(Color::White),
            ),
            Span::styled(
                format!("{} ", verdict_icon),
                Style::default().fg(verdict_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(trunc(&entry.summary, 36), Style::default().fg(verdict_color)),
        ]));
    }

    lines
}

// ═══════════════════════════ PARAMETERS PANEL ═══════════════════════════

fn render_parameters_panel(f: &mut Frame, area: Rect, ui: &NetworkUIState, theme: &Theme) {
    let is_focused = ui.focus == NetworkFocusZone::Parameters;
    let bc = zone_border(is_focused, theme);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(zone_title("PARAMETERS", is_focused))
        .border_style(Style::default().fg(bc));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 4 || inner.height < 2 {
        return;
    }

    let cursor = if ui.input_mode { "\u{2588}" } else { "" };
    let input_style = if ui.input_mode {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled("Tool:   ", dim()),
            Span::styled(
                ui.selected_tool.label().to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  [{}]", ui.selected_tool.category().label()),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled("Target: ", dim()),
            Span::styled(
                if ui.input_mode { "\u{2502}" } else { "[" },
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(format!("{}{}", ui.target_input, cursor), input_style),
            Span::styled(
                if ui.input_mode { "\u{2502}" } else { "]" },
                Style::default().fg(Color::DarkGray),
            ),
            if ui.input_mode {
                Span::styled(
                    " EDITING",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::raw("")
            },
        ]),
        Line::from(vec![
            Span::styled("Hint:   ", dim()),
            Span::styled(
                diagnostic_hint(ui.selected_tool).to_string(),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];

    // NAT confirmation
    if let Some(until) = ui.nat_mapping_confirm_until {
        if until > std::time::Instant::now() {
            lines.push(Line::from(Span::styled(
                "\u{26a0} Press Enter again to confirm NAT Mapping Test",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
        }
    }

    // Action buttons
    lines.push(Line::from(vec![
        Span::styled("[Enter]", Style::default().fg(Color::Green)),
        Span::styled(" Run  ", dim()),
        Span::styled("[x]", Style::default().fg(Color::Red)),
        Span::styled(" Cancel  ", dim()),
        Span::styled("[i]", Style::default().fg(Color::Cyan)),
        Span::styled(" Edit  ", dim()),
        Span::styled("[k]", Style::default().fg(Color::DarkGray)),
        Span::styled(" Clear log", dim()),
    ]));

    let p = Paragraph::new(lines);
    f.render_widget(p, inner);
}

// ═══════════════════════════ ACTIVITY PANEL ═══════════════════════════

fn render_activity_panel(f: &mut Frame, area: Rect, ui: &NetworkUIState, theme: &Theme) {
    let is_focused = ui.focus == NetworkFocusZone::Activity;
    let bc = zone_border(is_focused, theme);

    // Spinner animation based on event_log length (changes with each new event)
    let spinner_frames = ["\u{25dc}", "\u{25dd}", "\u{25de}", "\u{25df}"];
    let spinner_idx = ui.event_log.len() % spinner_frames.len();

    let running_label = if let Some(id) = ui.running_job {
        format!(" {} #{} running", spinner_frames[spinner_idx], id)
    } else {
        format!(" ({} events)", ui.event_log.len())
    };

    let title_style = if ui.running_job.is_some() {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(bc)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            "{}{}",
            zone_title("ACTIVITY", is_focused),
            running_label,
        ))
        .border_style(title_style);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width < 4 || inner.height < 2 {
        return;
    }

    let visible = inner.height as usize;
    let total = ui.event_log.len();
    // Clamp scroll: 0 means show latest (bottom), larger values scroll up
    let max_scroll = total.saturating_sub(visible);
    let scroll = ui.activity_scroll.min(max_scroll);
    // Take a window of events: skip the newest `scroll` entries, then take `visible`
    let lines: Vec<Line> = ui
        .event_log
        .iter()
        .rev()
        .skip(scroll)
        .take(visible)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|ev| {
            let (icon, color) = if ev.contains("failed") || ev.contains("Error") || ev.contains("Cannot") {
                ("\u{2718}", Color::Red)
            } else if ev.contains("completed") {
                ("\u{2714}", Color::Green)
            } else if ev.contains("started") {
                ("\u{25b6}", Color::Cyan)
            } else if ev.contains("Queued") {
                ("\u{23f3}", Color::Cyan)
            } else if ev.contains("cancelled") || ev.contains("Cancel") {
                ("\u{23f9}", Color::Yellow)
            } else if ev.contains("progress") || ev.contains("Job #") {
                ("\u{2022}", Color::White)
            } else {
                (" ", Color::DarkGray)
            };

            // Split timestamp from message for better formatting
            if ev.starts_with('[') {
                if let Some(close) = ev.find(']') {
                    let ts = &ev[..close + 1];
                    let msg = ev[close + 1..].trim_start();
                    return Line::from(vec![
                        Span::styled(
                            format!(" {}", ts),
                            Style::default().fg(Color::DarkGray),
                        ),
                        Span::styled(
                            format!(" {} ", icon),
                            Style::default().fg(color).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(msg.to_string(), Style::default().fg(color)),
                    ]);
                }
            }
            Line::from(vec![
                Span::styled(format!(" {} ", icon), Style::default().fg(color)),
                Span::styled(ev.to_string(), Style::default().fg(color)),
            ])
        })
        .collect();

    if lines.is_empty() {
        let p = Paragraph::new(vec![
            Line::from(Span::styled(
                " No activity yet.",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(
                " Select a tool and press Enter to run.",
                Style::default().fg(Color::DarkGray),
            )),
        ]);
        f.render_widget(p, inner);
    } else {
        f.render_widget(Paragraph::new(lines), inner);
    }
}

// ═══════════════════════════ HELP BAR ═══════════════════════════

fn render_help_bar(f: &mut Frame, area: Rect, ui: &NetworkUIState) {
    let focus_name = match ui.focus {
        NetworkFocusZone::Tools => "Tools",
        NetworkFocusZone::Interface => "Interface",
        NetworkFocusZone::Results => "Results",
        NetworkFocusZone::Parameters => "Params",
        NetworkFocusZone::Activity => "Activity",
    };

    let view = match ui.center_view {
        NetworkCenterView::Interface => "Iface",
        NetworkCenterView::Connections => "Conns",
    };

    let line = Line::from(vec![
        Span::styled(
            format!(" \u{25c6}{}\u{25c6} ", focus_name),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" \u{2190}\u{2192}", Style::default().fg(Color::White)),
        Span::styled(":panel ", dim()),
        Span::styled("\u{2191}\u{2193}", Style::default().fg(Color::White)),
        Span::styled(":nav ", dim()),
        Span::styled("Tab", Style::default().fg(Color::White)),
        Span::styled(":tabs ", dim()),
        Span::styled("Enter", Style::default().fg(Color::White)),
        Span::styled(":run ", dim()),
        Span::styled("i", Style::default().fg(Color::White)),
        Span::styled(":edit ", dim()),
        Span::styled("v", Style::default().fg(Color::White)),
        Span::styled(format!(":{} ", view), dim()),
        Span::styled("0", Style::default().fg(Color::White)),
        Span::styled(":mark ", dim()),
        Span::styled("x", Style::default().fg(Color::White)),
        Span::styled(":cancel ", dim()),
        Span::styled("k", Style::default().fg(Color::White)),
        Span::styled(":clear", dim()),
    ]);

    f.render_widget(Paragraph::new(line), area);
}

// ═══════════════════════════ COMPACT HELPERS ═══════════════════════════

fn render_compact_tool_params(f: &mut Frame, area: Rect, ui: &NetworkUIState, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Tool + Params")
        .border_style(Style::default().fg(theme.network_color));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width < 4 || inner.height < 2 {
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
        ]),
        Line::from(vec![
            Span::styled("Tgt:  [", dim()),
            Span::styled(
                format!("{}{}", ui.target_input, cursor),
                Style::default().fg(Color::White),
            ),
            Span::styled("]", dim()),
        ]),
        Line::from(vec![
            Span::styled("[Enter] Run ", Style::default().fg(Color::Green)),
            Span::styled("[x] Cancel", Style::default().fg(Color::Red)),
        ]),
    ];

    if let Some(err) = &ui.last_error {
        lines.push(Line::from(Span::styled(
            trunc(err, inner.width as usize),
            Style::default().fg(Color::Red),
        )));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn render_compact_result(f: &mut Frame, area: Rect, ui: &NetworkUIState, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Result")
        .border_style(Style::default().fg(theme.network_color));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.width < 4 || inner.height < 2 {
        return;
    }

    let mut lines = vec![Line::from(vec![
        Span::styled("Sum: ", dim()),
        Span::styled(
            trunc(&ui.last_summary, inner.width.saturating_sub(6) as usize),
            Style::default().fg(Color::White),
        ),
    ])];

    let max = inner.height.saturating_sub(1) as usize;
    for l in ui.detail_lines.iter().take(max) {
        lines.push(Line::from(Span::styled(
            trunc(l, inner.width as usize),
            Style::default().fg(Color::White),
        )));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn render_compact_bottom(
    f: &mut Frame,
    area: Rect,
    data: &NetworkData,
    ui: &NetworkUIState,
    theme: &Theme,
) {
    let bc = theme.network_color;
    match ui.center_view {
        NetworkCenterView::Interface => {
            let iface = primary_interface(data);
            let block = Block::default()
                .borders(Borders::ALL)
                .title("Network [v]toggle")
                .border_style(Style::default().fg(bc));
            let inner = block.inner(area);
            f.render_widget(block, area);
            if inner.width < 4 || inner.height < 2 {
                return;
            }

            if let Some(iface) = iface {
                let (rx, tx) = traffic_display(iface, ui);
                let lines = vec![
                    Line::from(vec![
                        Span::styled(&iface.name, Style::default().fg(Color::Cyan)),
                        Span::raw(" "),
                        Span::styled(&iface.ipv4_address, Style::default().fg(Color::White)),
                        Span::raw("  GW:"),
                        Span::styled(&iface.gateway, Style::default().fg(Color::White)),
                    ]),
                    Line::from(vec![
                        Span::styled(
                            format!("\u{2193}{:.1} \u{2191}{:.1} Mbps", iface.download_speed, iface.upload_speed),
                            Style::default().fg(Color::Green),
                        ),
                        Span::raw("  "),
                        Span::styled(format!("RX:{} TX:{}", rx, tx), Style::default().fg(Color::DarkGray)),
                    ]),
                ];
                f.render_widget(Paragraph::new(lines), inner);
            }
        }
        NetworkCenterView::Connections => {
            let block = Block::default()
                .borders(Borders::ALL)
                .title(format!("Conns ({}) [v]toggle", data.connections.len()))
                .border_style(Style::default().fg(bc));
            let inner = block.inner(area);
            f.render_widget(block, area);
            if inner.width < 4 || inner.height < 2 {
                return;
            }

            let lines: Vec<Line> = data
                .connections
                .iter()
                .take(inner.height as usize)
                .map(|c| {
                    Line::from(vec![
                        Span::styled(
                            format!("{:<12}", trunc(&c.process_name, 11)),
                            Style::default().fg(Color::White),
                        ),
                        Span::styled(format!("{:<5}", c.protocol), Style::default().fg(Color::Cyan)),
                        Span::styled(
                            format!("{}:{}", trunc(&c.remote_address, 15), c.remote_port),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ])
                })
                .collect();
            f.render_widget(Paragraph::new(lines), inner);
        }
    }
}

// ═══════════════════════════ UTILITIES ═══════════════════════════

fn primary_interface(data: &NetworkData) -> Option<&NetworkInterface> {
    data.interfaces
        .iter()
        .max_by(|a, b| {
            iface_score(a)
                .partial_cmp(&iface_score(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .or_else(|| data.interfaces.first())
}

fn iface_score(iface: &NetworkInterface) -> f64 {
    let mut s = 0.0;
    if iface.name != "lo" {
        s += 1000.0;
    }
    if iface.status.eq_ignore_ascii_case("connected") {
        s += 400.0;
    }
    if !iface.gateway.is_empty() {
        s += 300.0;
    }
    if !iface.ipv4_address.is_empty() {
        s += 200.0;
    }
    if !iface.ipv6_address.is_empty() {
        s += 100.0;
    }
    s + iface.download_speed + iface.upload_speed
}

fn traffic_display(iface: &NetworkInterface, ui: &NetworkUIState) -> (String, String) {
    if ui.show_marker_traffic {
        if let Some(marker) = &ui.traffic_marker {
            let rx = iface
                .bytes_received
                .saturating_sub(marker.bytes_received_at_mark);
            let tx = iface
                .bytes_sent
                .saturating_sub(marker.bytes_sent_at_mark);
            return (
                format!("\u{0394}{}", format_bytes(rx)),
                format!("\u{0394}{}", format_bytes(tx)),
            );
        }
    }
    (
        format_bytes(iface.bytes_received),
        format_bytes(iface.bytes_sent),
    )
}

fn zone_border(focused: bool, theme: &Theme) -> Color {
    if focused {
        Color::Yellow
    } else {
        theme.network_color
    }
}

fn zone_title(base: &str, focused: bool) -> String {
    if focused {
        format!("{} \u{25c6}", base)
    } else {
        base.to_string()
    }
}

fn header_style() -> Style {
    Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

fn dim() -> Style {
    Style::default().fg(Color::Gray)
}

fn trunc(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else if max > 3 {
        format!("{}...", &s[..max - 3])
    } else {
        s[..max].to_string()
    }
}

fn line_plain_text(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

fn diagnostic_hint(tool: NetworkDiagnosticTool) -> &'static str {
    match tool {
        NetworkDiagnosticTool::Resolve => "host/IP (example.org, 1.1.1.1)",
        NetworkDiagnosticTool::DnsExplain => "no input needed; inspects system DNS",
        NetworkDiagnosticTool::RouteInspect => "optional target for ip route get",
        NetworkDiagnosticTool::NicDeepInfo => "optional iface (eth0); empty=all",
        NetworkDiagnosticTool::ConnectionLab => "proto=tcp state=estab limit=200",
        NetworkDiagnosticTool::Ping => "target [profile=quick|latency|loss] [count=N]",
        NetworkDiagnosticTool::Trace => "target [proto=icmp|udp|tcp] [hops=N]",
        NetworkDiagnosticTool::MtuProbe => "target host/IP for PMTU probe",
        NetworkDiagnosticTool::PortScan => "host[:ports] e.g. example.org:22,80,443",
        NetworkDiagnosticTool::NatCapability => "no input; probes UPnP/NAT-PMP/PCP",
        NetworkDiagnosticTool::NatMappingTest => "tcp 8080 8080 120 (proto in out ttl)",
        NetworkDiagnosticTool::ExportReport => "no input; exports diagnostics report",
    }
}

/// Detect interface type short tag from name prefix (fixed-width ASCII, no overlapping emojis)
fn detect_iface_type(name: &str) -> &'static str {
    if name.starts_with("wl") || name.starts_with("wlan") {
        "[W] "
    } else if name.starts_with("eth") || name.starts_with("en") {
        "[E] "
    } else if name.starts_with("lo") {
        "[L] "
    } else if name.starts_with("docker") || name.starts_with("br-") || name.starts_with("veth") {
        "[D] "
    } else if name.starts_with("tun") || name.starts_with("tap") || name.starts_with("wg") {
        "[V] "
    } else if name.starts_with("virbr") || name.starts_with("vnet") {
        "[B] "
    } else {
        ""
    }
}

/// Detect interface type label from name prefix
fn detect_iface_type_label(name: &str) -> &'static str {
    if name.starts_with("wl") || name.starts_with("wlan") {
        "Wireless"
    } else if name.starts_with("eth") || name.starts_with("en") {
        "Ethernet"
    } else if name.starts_with("lo") {
        "Loopback"
    } else if name.starts_with("docker") || name.starts_with("br-") || name.starts_with("veth") {
        "Docker/Container"
    } else if name.starts_with("tun") || name.starts_with("tap") || name.starts_with("wg") {
        "VPN/Tunnel"
    } else if name.starts_with("virbr") || name.starts_with("vnet") {
        "Virtual Bridge"
    } else {
        "Unknown"
    }
}

/// Format large packet/byte counts in human-readable abbreviated form
fn format_pkt_count(count: u64) -> String {
    if count >= 1_000_000_000 {
        format!("{:.1}G", count as f64 / 1_000_000_000.0)
    } else if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.1}K", count as f64 / 1_000.0)
    } else {
        count.to_string()
    }
}
