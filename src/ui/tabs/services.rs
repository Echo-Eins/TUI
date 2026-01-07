use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Wrap},
    Frame,
};

use crate::app::{
    state::{ServiceSortColumn, ServiceStatusFilter},
    App,
};
use crate::monitors::services::{ServiceEntry, ServiceStatus};
use crate::ui::theme::Theme;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let service_data = app.state.service_data.read();
    let service_error = app.state.service_error.read();

    if let Some(message) = service_error.as_ref() {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);
        let block = Block::default()
            .title("Service Monitor")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.warning_color));

        let text = Paragraph::new(format!("Service monitor unavailable: {}", message))
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(text, area);
    } else if let Some(data) = service_data.as_ref() {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);

        if app.state.compact_mode {
            render_compact(f, area, data, app, &theme);
        } else {
            render_full(f, area, data, app, &theme);
        }
    } else {
        let block = Block::default()
            .title("Service Monitor")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));

        let text = Paragraph::new("Loading service data...")
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(text, area);
    }
}

fn render_full(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::ServiceData,
    app: &App,
    theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header with stats
            Constraint::Min(10),    // Service table
            Constraint::Length(12), // Details panel
        ])
        .split(area);

    // Render header
    render_header(f, chunks[0], data, app, theme);

    // Render service table
    render_service_table(f, chunks[1], data, app, theme);

    // Render details panel
    render_details_panel(f, chunks[2], data, app, theme);
}

fn render_compact(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::ServiceData,
    app: &App,
    theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(8),    // Service table (compact)
        ])
        .split(area);

    // Render header
    render_header(f, chunks[0], data, app, theme);

    // Render service table
    render_service_table(f, chunks[1], data, app, theme);
}

fn render_header(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::ServiceData,
    app: &App,
    _theme: &Theme,
) {
    let total_services = data.services.len();
    let running_services = data
        .services
        .iter()
        .filter(|s| s.status == ServiceStatus::Running)
        .count();
    let stopped_services = data
        .services
        .iter()
        .filter(|s| s.status == ServiceStatus::Stopped)
        .count();

    let filter_text = match app.state.services_state.status_filter {
        ServiceStatusFilter::All => "All",
        ServiceStatusFilter::Running => "Running",
        ServiceStatusFilter::Stopped => "Stopped",
    };

    let header_text = vec![Line::from(vec![
        Span::styled("Total: ", Style::default().fg(Color::Gray)),
        Span::styled(
            format!("{}", total_services),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("Running: ", Style::default().fg(Color::Gray)),
        Span::styled(
            format!("{}", running_services),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("Stopped: ", Style::default().fg(Color::Gray)),
        Span::styled(
            format!("{}", stopped_services),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("Filter: ", Style::default().fg(Color::Gray)),
        Span::styled(
            filter_text,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ])];

    let block = Block::default()
        .title("Service Monitor")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(header_text).block(block);
    f.render_widget(paragraph, area);
}

fn render_service_table(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::ServiceData,
    app: &App,
    _theme: &Theme,
) {
    // Filter and sort services
    let mut services = data.services.clone();

    // Apply status filter
    match app.state.services_state.status_filter {
        ServiceStatusFilter::Running => {
            services.retain(|s| s.status == ServiceStatus::Running);
        }
        ServiceStatusFilter::Stopped => {
            services.retain(|s| s.status == ServiceStatus::Stopped);
        }
        ServiceStatusFilter::All => {}
    }

    // Apply sorting
    sort_services(
        &mut services,
        app.state.services_state.sort_column,
        app.state.services_state.sort_ascending,
    );

    // Create table header with sort indicators
    let sort_indicator = if app.state.services_state.sort_ascending {
        "↑"
    } else {
        "↓"
    };

    let headers = vec![
        Cell::from(
            if app.state.services_state.sort_column == ServiceSortColumn::Name {
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
            if app.state.services_state.sort_column == ServiceSortColumn::DisplayName {
                format!("Display Name {}", sort_indicator)
            } else {
                "Display Name".to_string()
            },
        )
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from(
            if app.state.services_state.sort_column == ServiceSortColumn::Status {
                format!("Status {}", sort_indicator)
            } else {
                "Status".to_string()
            },
        )
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from(
            if app.state.services_state.sort_column == ServiceSortColumn::StartType {
                format!("Start Type {}", sort_indicator)
            } else {
                "Start Type".to_string()
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
    let rows: Vec<Row> = services
        .iter()
        .enumerate()
        .map(|(i, service)| {
            let is_selected = i == app.state.services_state.selected_index;
            let base_style = if is_selected {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };

            let status_color = match service.status {
                ServiceStatus::Running => Color::Green,
                ServiceStatus::Stopped => Color::Red,
                ServiceStatus::Paused => Color::Yellow,
                ServiceStatus::StartPending | ServiceStatus::ContinuePending => Color::LightBlue,
                ServiceStatus::StopPending | ServiceStatus::PausePending => Color::Magenta,
                ServiceStatus::Unknown => Color::Gray,
            };

            let status_style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD)
            };

            Row::new(vec![
                Cell::from(service.name.clone()).style(base_style),
                Cell::from(service.display_name.clone()).style(base_style),
                Cell::from(service.status.as_str()).style(status_style),
                Cell::from(service.start_type.as_str()).style(base_style),
            ])
        })
        .collect();

    // Hotkeys hint
    let hotkeys = vec![Line::from(vec![
        Span::styled("↑/↓", Style::default().fg(Color::Cyan)),
        Span::raw(": Navigate  "),
        Span::styled("n/d/s/t", Style::default().fg(Color::Cyan)),
        Span::raw(": Sort by Name/Display/Status/Type  "),
        Span::styled("f", Style::default().fg(Color::Cyan)),
        Span::raw(": Filter  "),
        Span::styled("PgUp/PgDn", Style::default().fg(Color::Cyan)),
        Span::raw(": Page"),
    ])];

    let block = Block::default()
        .title("Services")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    // Calculate constraints for table columns
    let widths = [
        Constraint::Length(25), // Name
        Constraint::Min(30),    // Display Name
        Constraint::Length(12), // Status
        Constraint::Length(16), // Start Type
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
    data: &crate::monitors::ServiceData,
    app: &App,
    _theme: &Theme,
) {
    // Filter services (same as in table)
    let mut services = data.services.clone();

    match app.state.services_state.status_filter {
        ServiceStatusFilter::Running => {
            services.retain(|s| s.status == ServiceStatus::Running);
        }
        ServiceStatusFilter::Stopped => {
            services.retain(|s| s.status == ServiceStatus::Stopped);
        }
        ServiceStatusFilter::All => {}
    }

    sort_services(
        &mut services,
        app.state.services_state.sort_column,
        app.state.services_state.sort_ascending,
    );

    // Get selected service
    if let Some(service) = services.get(app.state.services_state.selected_index) {
        let mut details = Vec::new();

        details.push(Line::from(vec![Span::styled(
            "Service Details",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]));

        details.push(Line::from(""));

        details.push(Line::from(vec![
            Span::styled("Name: ", Style::default().fg(Color::Gray)),
            Span::styled(
                &service.name,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        details.push(Line::from(vec![
            Span::styled("Display Name: ", Style::default().fg(Color::Gray)),
            Span::styled(&service.display_name, Style::default().fg(Color::White)),
        ]));

        let status_color = match service.status {
            ServiceStatus::Running => Color::Green,
            ServiceStatus::Stopped => Color::Red,
            ServiceStatus::Paused => Color::Yellow,
            _ => Color::Gray,
        };

        details.push(Line::from(vec![
            Span::styled("Status: ", Style::default().fg(Color::Gray)),
            Span::styled(
                service.status.as_str(),
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled("Start Type: ", Style::default().fg(Color::Gray)),
            Span::styled(
                service.start_type.as_str(),
                Style::default().fg(Color::Cyan),
            ),
        ]));

        details.push(Line::from(vec![
            Span::styled("Can Stop: ", Style::default().fg(Color::Gray)),
            Span::styled(
                if service.can_stop { "Yes" } else { "No" },
                Style::default().fg(if service.can_stop {
                    Color::Green
                } else {
                    Color::Red
                }),
            ),
            Span::raw("  "),
            Span::styled("Can Pause: ", Style::default().fg(Color::Gray)),
            Span::styled(
                if service.can_pause_and_continue {
                    "Yes"
                } else {
                    "No"
                },
                Style::default().fg(if service.can_pause_and_continue {
                    Color::Green
                } else {
                    Color::Red
                }),
            ),
        ]));

        if let Some(service_type) = &service.service_type {
            details.push(Line::from(vec![
                Span::styled("Service Type: ", Style::default().fg(Color::Gray)),
                Span::styled(service_type, Style::default().fg(Color::White)),
            ]));
        }

        if let Some(description) = &service.description {
            details.push(Line::from(""));
            details.push(Line::from(vec![Span::styled(
                "Description:",
                Style::default().fg(Color::Gray),
            )]));
            details.push(Line::from(vec![Span::styled(
                description,
                Style::default().fg(Color::White),
            )]));
        }

        if !service.dependent_services.is_empty() {
            details.push(Line::from(""));
            details.push(Line::from(vec![Span::styled(
                "Dependent Services:",
                Style::default().fg(Color::Gray),
            )]));
            details.push(Line::from(vec![Span::styled(
                service.dependent_services.join(", "),
                Style::default().fg(Color::White),
            )]));
        }

        let block = Block::default()
            .title("Service Details")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let paragraph = Paragraph::new(details)
            .block(block)
            .wrap(Wrap { trim: true });

        f.render_widget(paragraph, area);
    } else {
        let block = Block::default()
            .title("Service Details")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let text = Paragraph::new("No service selected")
            .block(block)
            .style(Style::default().fg(Color::Gray));

        f.render_widget(text, area);
    }
}

fn sort_services(services: &mut Vec<ServiceEntry>, column: ServiceSortColumn, ascending: bool) {
    services.sort_by(|a, b| {
        let cmp = match column {
            ServiceSortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            ServiceSortColumn::DisplayName => a
                .display_name
                .to_lowercase()
                .cmp(&b.display_name.to_lowercase()),
            ServiceSortColumn::Status => {
                // Sort by status priority: Running > Paused > Starting/Stopping > Stopped
                let a_priority = match a.status {
                    ServiceStatus::Running => 0,
                    ServiceStatus::Paused => 1,
                    ServiceStatus::StartPending | ServiceStatus::ContinuePending => 2,
                    ServiceStatus::StopPending | ServiceStatus::PausePending => 3,
                    ServiceStatus::Stopped => 4,
                    ServiceStatus::Unknown => 5,
                };
                let b_priority = match b.status {
                    ServiceStatus::Running => 0,
                    ServiceStatus::Paused => 1,
                    ServiceStatus::StartPending | ServiceStatus::ContinuePending => 2,
                    ServiceStatus::StopPending | ServiceStatus::PausePending => 3,
                    ServiceStatus::Stopped => 4,
                    ServiceStatus::Unknown => 5,
                };
                a_priority.cmp(&b_priority)
            }
            ServiceSortColumn::StartType => {
                // Sort by start type priority: Automatic > Auto (Delayed) > Manual > Disabled
                let a_priority = match a.start_type {
                    crate::monitors::services::ServiceStartType::Automatic => 0,
                    crate::monitors::services::ServiceStartType::AutomaticDelayedStart => 1,
                    crate::monitors::services::ServiceStartType::Manual => 2,
                    crate::monitors::services::ServiceStartType::Disabled => 3,
                    crate::monitors::services::ServiceStartType::Unknown => 4,
                };
                let b_priority = match b.start_type {
                    crate::monitors::services::ServiceStartType::Automatic => 0,
                    crate::monitors::services::ServiceStartType::AutomaticDelayedStart => 1,
                    crate::monitors::services::ServiceStartType::Manual => 2,
                    crate::monitors::services::ServiceStartType::Disabled => 3,
                    crate::monitors::services::ServiceStartType::Unknown => 4,
                };
                a_priority.cmp(&b_priority)
            }
        };

        if ascending {
            cmp
        } else {
            cmp.reverse()
        }
    });
}
