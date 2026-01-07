use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Row, Sparkline, Table},
    Frame,
};

use crate::app::App;
use crate::ui::theme::Theme;
use crate::utils::format::format_bytes;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let network_data = app.state.network_data.read();
    let network_error = app.state.network_error.read();

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
            render_compact(f, area, data, &theme);
        } else {
            render_full(f, area, data, &theme);
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

fn render_full(f: &mut Frame, area: Rect, data: &crate::monitors::NetworkData, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Length(8), // Interface details (per interface)
            Constraint::Length(8), // Traffic graphs (Download/Upload)
            Constraint::Min(10),   // Active connections and bandwidth consumers
        ])
        .split(area);

    // Header - show primary interface summary
    render_header(f, chunks[0], data, theme);

    // Interface details
    render_interface_details(f, chunks[1], data, theme);

    // Traffic graphs
    render_traffic_graphs(f, chunks[2], data, theme);

    // Split bottom section for connections and bandwidth consumers
    let bottom_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50), // Active connections
            Constraint::Percentage(50), // Bandwidth consumers
        ])
        .split(chunks[3]);

    // Active connections
    render_connections_table(f, bottom_chunks[0], data, theme);

    // Bandwidth consumers
    render_bandwidth_consumers(f, bottom_chunks[1], data, theme);
}

fn render_compact(f: &mut Frame, area: Rect, data: &crate::monitors::NetworkData, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Length(6), // Quick stats
            Constraint::Min(8),    // Connections (compact)
        ])
        .split(area);

    // Header
    render_header(f, chunks[0], data, theme);

    // Quick stats
    let mut lines = Vec::new();

    if let Some(iface) = data.interfaces.first() {
        lines.push(Line::from(vec![
            Span::styled("Status: ", Style::default().fg(Color::Gray)),
            Span::styled(
                &iface.status,
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled("Speed: ", Style::default().fg(Color::Gray)),
            Span::styled(&iface.link_speed, Style::default().fg(Color::Cyan)),
        ]));

        lines.push(Line::from(vec![
            Span::styled("IPv4: ", Style::default().fg(Color::Gray)),
            Span::styled(&iface.ipv4_address, Style::default().fg(Color::White)),
            Span::raw("  "),
            Span::styled("Gateway: ", Style::default().fg(Color::Gray)),
            Span::styled(&iface.gateway, Style::default().fg(Color::White)),
        ]));

        lines.push(Line::from(vec![
            Span::styled("Download: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{:.2} Mbps", iface.download_speed),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled("Upload: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format!("{:.2} Mbps", iface.upload_speed),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        lines.push(Line::from(vec![
            Span::styled("Total RX: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format_bytes(iface.bytes_received),
                Style::default().fg(Color::White),
            ),
            Span::raw("  "),
            Span::styled("TX: ", Style::default().fg(Color::Gray)),
            Span::styled(
                format_bytes(iface.bytes_sent),
                Style::default().fg(Color::White),
            ),
        ]));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Network Stats")
        .border_style(Style::default().fg(theme.network_color));

    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, chunks[1]);

    // Compact connections (top 5)
    render_connections_compact(f, chunks[2], data, theme);
}

fn render_header(f: &mut Frame, area: Rect, data: &crate::monitors::NetworkData, theme: &Theme) {
    let header_text = if let Some(iface) = data.interfaces.first() {
        format!(
            "{} | {} | ↓ {:.2} Mbps ↑ {:.2} Mbps | Connections: {}",
            iface.name,
            iface.status,
            iface.download_speed,
            iface.upload_speed,
            data.connections.len()
        )
    } else {
        "No active network interfaces".to_string()
    };

    let header_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.network_color));

    let header_paragraph = Paragraph::new(header_text).block(header_block).style(
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    );

    f.render_widget(header_paragraph, area);
}

fn render_interface_details(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::NetworkData,
    theme: &Theme,
) {
    if let Some(iface) = data.interfaces.first() {
        let lines = vec![
            Line::from(vec![
                Span::styled("Interface: ", Style::default().fg(Color::Gray)),
                Span::styled(&iface.description, Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("IPv4: ", Style::default().fg(Color::Gray)),
                Span::styled(&iface.ipv4_address, Style::default().fg(Color::Cyan)),
                Span::raw("  "),
                Span::styled("IPv6: ", Style::default().fg(Color::Gray)),
                Span::styled(&iface.ipv6_address, Style::default().fg(Color::Cyan)),
            ]),
            Line::from(vec![
                Span::styled("Gateway: ", Style::default().fg(Color::Gray)),
                Span::styled(&iface.gateway, Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled("MAC: ", Style::default().fg(Color::Gray)),
                Span::styled(&iface.mac_address, Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("DNS: ", Style::default().fg(Color::Gray)),
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
                Span::styled("Link Speed: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    &iface.link_speed,
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled("Duplex: ", Style::default().fg(Color::Gray)),
                Span::styled(&iface.duplex, Style::default().fg(Color::White)),
                Span::raw("  "),
                Span::styled("MTU: ", Style::default().fg(Color::Gray)),
                Span::styled(format!("{}", iface.mtu), Style::default().fg(Color::White)),
            ]),
            Line::from(vec![
                Span::styled("Total Received: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format_bytes(iface.bytes_received),
                    Style::default().fg(Color::Green),
                ),
                Span::raw("  "),
                Span::styled("Total Sent: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    format_bytes(iface.bytes_sent),
                    Style::default().fg(Color::Cyan),
                ),
            ]),
        ];

        let block = Block::default()
            .borders(Borders::ALL)
            .title("Interface Details")
            .border_style(Style::default().fg(theme.network_color));

        let paragraph = Paragraph::new(lines).block(block);
        f.render_widget(paragraph, area);
    }
}

fn render_traffic_graphs(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::NetworkData,
    _theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    // Download graph
    if !data.traffic_history.is_empty() {
        let download_data: Vec<u64> = data
            .traffic_history
            .iter()
            .map(|s| (s.download_mbps * 100.0) as u64)
            .collect();

        let max_download = download_data.iter().max().copied().unwrap_or(1).max(1);
        let max_download_mbps = max_download as f64 / 100.0;

        let sparkline = Sparkline::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Download (peak {:.2} Mbps)", max_download_mbps))
                    .border_style(Style::default().fg(Color::Green)),
            )
            .data(&download_data)
            .style(Style::default().fg(Color::Green))
            .max(max_download);

        f.render_widget(sparkline, chunks[0]);
    } else {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Download")
            .border_style(Style::default().fg(Color::Green));

        let text = Paragraph::new("Collecting data...").block(block);
        f.render_widget(text, chunks[0]);
    }

    // Upload graph
    if !data.traffic_history.is_empty() {
        let upload_data: Vec<u64> = data
            .traffic_history
            .iter()
            .map(|s| (s.upload_mbps * 100.0) as u64)
            .collect();

        let max_upload = upload_data.iter().max().copied().unwrap_or(1).max(1);
        let max_upload_mbps = max_upload as f64 / 100.0;

        let sparkline = Sparkline::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("Upload (peak {:.2} Mbps)", max_upload_mbps))
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .data(&upload_data)
            .style(Style::default().fg(Color::Cyan))
            .max(max_upload);

        f.render_widget(sparkline, chunks[1]);
    } else {
        let block = Block::default()
            .borders(Borders::ALL)
            .title("Upload")
            .border_style(Style::default().fg(Color::Cyan));

        let text = Paragraph::new("Collecting data...").block(block);
        f.render_widget(text, chunks[1]);
    }
}

fn render_connections_table(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::NetworkData,
    theme: &Theme,
) {
    let header = Row::new(vec![
        "Process", "PID", "Protocol", "Local", "Remote", "State",
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
    .bottom_margin(0);

    let rows: Vec<Row> = data
        .connections
        .iter()
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
        Constraint::Percentage(20), // Process
        Constraint::Percentage(8),  // PID
        Constraint::Percentage(10), // Protocol
        Constraint::Percentage(25), // Local
        Constraint::Percentage(25), // Remote
        Constraint::Percentage(12), // State
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("Active Connections ({})", data.connections.len()))
                .border_style(Style::default().fg(theme.network_color)),
        )
        .column_spacing(1);

    f.render_widget(table, area);
}

fn render_connections_compact(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::NetworkData,
    theme: &Theme,
) {
    let header = Row::new(vec!["Process", "Remote", "State"])
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .bottom_margin(0);

    let rows: Vec<Row> = data
        .connections
        .iter()
        .take(5)
        .map(|conn| {
            Row::new(vec![
                format!("{} ({})", conn.process_name, conn.pid),
                format!("{}:{}", conn.remote_address, conn.remote_port),
                conn.state.clone(),
            ])
            .style(Style::default().fg(Color::White))
        })
        .collect();

    let widths = [
        Constraint::Percentage(40), // Process
        Constraint::Percentage(40), // Remote
        Constraint::Percentage(20), // State
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("Active Connections ({})", data.connections.len()))
                .border_style(Style::default().fg(theme.network_color)),
        )
        .column_spacing(1);

    f.render_widget(table, area);
}

fn render_bandwidth_consumers(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::NetworkData,
    theme: &Theme,
) {
    let has_estimated = data.bandwidth_consumers.iter().any(|c| c.estimated);
    let header = Row::new(vec![
        "Process", "PID", "Download", "Upload", "Total RX", "Total TX",
    ])
    .style(
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
    .bottom_margin(0);

    let rows: Vec<Row> = data
        .bandwidth_consumers
        .iter()
        .take(10)
        .map(|consumer| {
            let name = if consumer.estimated {
                format!("~{}", consumer.process_name)
            } else {
                consumer.process_name.clone()
            };
            let download = if consumer.estimated {
                format!("~{:.2} Mbps", consumer.download_speed)
            } else {
                format!("{:.2} Mbps", consumer.download_speed)
            };
            let upload = if consumer.estimated {
                format!("~{:.2} Mbps", consumer.upload_speed)
            } else {
                format!("{:.2} Mbps", consumer.upload_speed)
            };
            let total_rx = if consumer.estimated {
                format!("~{}", format_bytes(consumer.total_bytes_received))
            } else {
                format_bytes(consumer.total_bytes_received)
            };
            let total_tx = if consumer.estimated {
                format!("~{}", format_bytes(consumer.total_bytes_sent))
            } else {
                format_bytes(consumer.total_bytes_sent)
            };

            Row::new(vec![
                name,
                format!("{}", consumer.pid),
                download,
                upload,
                total_rx,
                total_tx,
            ])
            .style(Style::default().fg(Color::White))
        })
        .collect();

    let widths = [
        Constraint::Percentage(20), // Process
        Constraint::Percentage(10), // PID
        Constraint::Percentage(15), // Download
        Constraint::Percentage(15), // Upload
        Constraint::Percentage(20), // Total RX
        Constraint::Percentage(20), // Total TX
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(
                    "Bandwidth Consumers{} (Top {})",
                    if has_estimated { " ~est." } else { "" },
                    data.bandwidth_consumers.len().min(10)
                ))
                .border_style(Style::default().fg(theme.network_color)),
        )
        .column_spacing(1);

    f.render_widget(table, area);
}
