use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Table},
    Frame,
};

use crate::app::{state::OllamaView, App};
use crate::ui::theme::Theme;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let ollama_data = app.state.ollama_data.read();
    let ollama_error = app.state.ollama_error.read();

    if let Some(message) = ollama_error.as_ref() {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);
        let block = Block::default()
            .title("Ollama Manager")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.warning_color));

        let text = Paragraph::new(format!("Ollama monitor unavailable: {}", message))
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(text, area);
    } else if let Some(data) = ollama_data.as_ref() {
        if !data.available {
            render_unavailable(f, area);
            return;
        }

        let config = app.state.config.read();
        let theme = Theme::from_config(&config);

        if app.state.compact_mode {
            render_compact(f, area, data, app, &theme);
        } else {
            render_full(f, area, data, app, &theme);
        }
    } else {
        let block = Block::default()
            .title("Ollama Manager")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));

        let text = Paragraph::new("Loading Ollama data...")
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(text, area);
    }
}

fn render_unavailable(f: &mut Frame, area: Rect) {
    let block = Block::default()
        .title("Ollama Manager")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));

    let text = vec![
        Line::from(vec![Span::styled(
            "Ollama Not Available",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Ollama is not installed or not in PATH.",
            Style::default().fg(Color::Gray),
        )]),
        Line::from(vec![
            Span::styled(
                "Please install Ollama from: ",
                Style::default().fg(Color::Gray),
            ),
            Span::styled("https://ollama.ai", Style::default().fg(Color::Cyan)),
        ]),
    ];

    let paragraph = Paragraph::new(text).block(block);
    f.render_widget(paragraph, area);
}

fn render_full(
    f: &mut Frame,
    area: Rect,
    data: &crate::integrations::OllamaData,
    app: &App,
    theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(10),   // Main content (models/running)
            Constraint::Length(8), // Running models / VRAM panel
            Constraint::Length(5), // Activity log
            Constraint::Length(3), // Command input / Help
        ])
        .split(area);

    // Render header
    render_header(f, chunks[0], data, app, theme);

    // Render main content based on current view
    match app.state.ollama_state.current_view {
        OllamaView::Models => render_models_table(f, chunks[1], data, app, theme),
        OllamaView::Running => render_running_models_table(f, chunks[1], data, app, theme),
    }

    // Render VRAM/GPU panel
    render_vram_panel(f, chunks[2], data, app, theme);

    // Render activity log
    render_activity_log(f, chunks[3], data, theme);

    // Render command input or help
    if app.state.ollama_state.show_command_input {
        render_command_input(f, chunks[4], app, theme);
    } else {
        render_help(f, chunks[4], theme);
    }
}

fn render_compact(
    f: &mut Frame,
    area: Rect,
    data: &crate::integrations::OllamaData,
    app: &App,
    theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(10),   // Main content
            Constraint::Length(3), // Help
        ])
        .split(area);

    // Render header
    render_header(f, chunks[0], data, app, theme);

    // Render main content based on current view
    match app.state.ollama_state.current_view {
        OllamaView::Models => render_models_table(f, chunks[1], data, app, theme),
        OllamaView::Running => render_running_models_table(f, chunks[1], data, app, theme),
    }

    // Render help
    render_help(f, chunks[2], theme);
}

fn render_header(
    f: &mut Frame,
    area: Rect,
    data: &crate::integrations::OllamaData,
    app: &App,
    _theme: &Theme,
) {
    let model_count = data.models.len();
    let running_count = data.running_models.len();

    let view_text = match app.state.ollama_state.current_view {
        OllamaView::Models => "Available Models",
        OllamaView::Running => "Running Models",
    };

    let header_text = vec![Line::from(vec![
        Span::styled("View: ", Style::default().fg(Color::Gray)),
        Span::styled(
            view_text,
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("Total Models: ", Style::default().fg(Color::Gray)),
        Span::styled(
            format!("{}", model_count),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("Running: ", Style::default().fg(Color::Gray)),
        Span::styled(
            format!("{}", running_count),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
    ])];

    let block = Block::default()
        .title("Ollama Manager")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let paragraph = Paragraph::new(header_text).block(block);
    f.render_widget(paragraph, area);
}

fn render_models_table(
    f: &mut Frame,
    area: Rect,
    data: &crate::integrations::OllamaData,
    app: &App,
    _theme: &Theme,
) {
    let headers = vec![
        Cell::from("Name").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Size").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Modified").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    let header = Row::new(headers).height(1);

    let selected_index = if data.models.is_empty() {
        0
    } else {
        app.state
            .ollama_state
            .selected_model_index
            .min(data.models.len().saturating_sub(1))
    };

    let content_height = area.height.saturating_sub(2);
    let footer_height = if area.height > 2 { 1 } else { 0 };
    let header_height = 1u16;
    let visible_rows = content_height
        .saturating_sub(header_height + footer_height) as usize;
    let scroll_offset = if visible_rows == 0 {
        0
    } else {
        selected_index.saturating_sub(visible_rows.saturating_sub(1))
    };

    let rows: Vec<Row> = data
        .models
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_rows.max(0))
        .map(|(i, model)| {
            let is_selected = i == selected_index;
            let style = if is_selected {
                Style::default().fg(Color::Black).bg(Color::Magenta)
            } else {
                Style::default().fg(Color::White)
            };

            Row::new(vec![
                Cell::from(model.name.clone()).style(style),
                Cell::from(model.size_display.clone()).style(style),
                Cell::from(model.modified.clone()).style(style),
            ])
        })
        .collect();

    let hotkeys = vec![Line::from(vec![
        Span::styled("↑/↓", Style::default().fg(Color::Cyan)),
        Span::raw(": Navigate  "),
        Span::styled("v", Style::default().fg(Color::Cyan)),
        Span::raw(": Switch View  "),
        Span::styled("r", Style::default().fg(Color::Cyan)),
        Span::raw(": Run  "),
        Span::styled("d", Style::default().fg(Color::Cyan)),
        Span::raw(": Delete  "),
        Span::styled("p", Style::default().fg(Color::Cyan)),
        Span::raw(": Pull  "),
        Span::styled("c", Style::default().fg(Color::Cyan)),
        Span::raw(": Command"),
    ])];

    let block = Block::default()
        .title("Available Models")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let widths = [
        Constraint::Min(30),    // Name
        Constraint::Length(12), // Size
        Constraint::Min(15),    // Modified
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1)
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Magenta));

    f.render_widget(table, area);

    // Render hotkeys at the bottom
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

fn render_running_models_table(
    f: &mut Frame,
    area: Rect,
    data: &crate::integrations::OllamaData,
    app: &App,
    _theme: &Theme,
) {
    let headers = vec![
        Cell::from("Name").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Size").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Processor").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Until").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    let header = Row::new(headers).height(1);

    let selected_index = if data.running_models.is_empty() {
        0
    } else {
        app.state
            .ollama_state
            .selected_running_index
            .min(data.running_models.len().saturating_sub(1))
    };

    let content_height = area.height.saturating_sub(2);
    let footer_height = if area.height > 2 { 1 } else { 0 };
    let header_height = 1u16;
    let visible_rows = content_height
        .saturating_sub(header_height + footer_height) as usize;
    let scroll_offset = if visible_rows == 0 {
        0
    } else {
        selected_index.saturating_sub(visible_rows.saturating_sub(1))
    };

    let rows: Vec<Row> = data
        .running_models
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_rows.max(0))
        .map(|(i, model)| {
            let is_selected = i == selected_index;
            let style = if is_selected {
                Style::default().fg(Color::Black).bg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            };

            let until_text = model.until.clone().unwrap_or_else(|| "Forever".to_string());

            Row::new(vec![
                Cell::from(model.name.clone()).style(style),
                Cell::from(model.size_display.clone()).style(style),
                Cell::from(model.processor.clone()).style(style.fg(Color::Cyan)),
                Cell::from(until_text).style(style),
            ])
        })
        .collect();

    let hotkeys = vec![Line::from(vec![
        Span::styled("↑/↓", Style::default().fg(Color::Cyan)),
        Span::raw(": Navigate  "),
        Span::styled("v", Style::default().fg(Color::Cyan)),
        Span::raw(": Switch View  "),
        Span::styled("s", Style::default().fg(Color::Cyan)),
        Span::raw(": Stop  "),
        Span::styled("l", Style::default().fg(Color::Cyan)),
        Span::raw(": Refresh"),
    ])];

    let block = Block::default()
        .title("Running Models")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let widths = [
        Constraint::Min(25),    // Name
        Constraint::Length(12), // Size
        Constraint::Length(15), // Processor
        Constraint::Min(20),    // Until
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1)
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Green));

    f.render_widget(table, area);

    // Render hotkeys at the bottom
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

fn render_vram_panel(
    f: &mut Frame,
    area: Rect,
    data: &crate::integrations::OllamaData,
    app: &App,
    _theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50), // VRAM gauge
            Constraint::Percentage(50), // Running models summary
        ])
        .split(area);

    // VRAM usage gauge (from GPU data if available)
    let gpu_data = app.state.gpu_data.read();
    let (vram_used, vram_total, vram_percent) = if let Some(gpu) = gpu_data.as_ref() {
        let percent = if gpu.memory_total > 0 {
            (gpu.memory_used as f64 / gpu.memory_total as f64) * 100.0
        } else {
            0.0
        };
        (gpu.memory_used, gpu.memory_total, percent as f32)
    } else {
        (0, 0, 0.0)
    };

    let vram_text = format!(
        "{:.2}% ({} MB / {} MB)",
        vram_percent, vram_used, vram_total
    );
    let gauge = Gauge::default()
        .block(
            Block::default()
                .title("VRAM Usage")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::Black))
        .percent(vram_percent as u16)
        .label(vram_text);

    f.render_widget(gauge, chunks[0]);

    // Running models summary
    let mut summary = Vec::new();
    summary.push(Line::from(vec![
        Span::styled("Active Models: ", Style::default().fg(Color::Gray)),
        Span::styled(
            format!("{}", data.running_models.len()),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    let total_size: u64 = data.running_models.iter().map(|m| m.size_bytes).sum();
    let size_gb = total_size as f64 / (1024.0 * 1024.0 * 1024.0);
    summary.push(Line::from(vec![
        Span::styled("Total Size: ", Style::default().fg(Color::Gray)),
        Span::styled(
            format!("{:.2} GB", size_gb),
            Style::default().fg(Color::Cyan),
        ),
    ]));

    if !data.running_models.is_empty() {
        summary.push(Line::from(""));
        for (i, model) in data.running_models.iter().take(3).enumerate() {
            summary.push(Line::from(vec![
                Span::raw(format!("{}. ", i + 1)),
                Span::styled(&model.name, Style::default().fg(Color::White)),
                Span::raw(" - "),
                Span::styled(&model.processor, Style::default().fg(Color::Green)),
            ]));
        }
    }

    let block = Block::default()
        .title("Running Summary")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let paragraph = Paragraph::new(summary).block(block);
    f.render_widget(paragraph, chunks[1]);
}

fn render_activity_log(
    f: &mut Frame,
    area: Rect,
    data: &crate::integrations::OllamaData,
    _theme: &Theme,
) {
    let log_entries: Vec<Line> = data
        .activity_log
        .iter()
        .rev()
        .take(3)
        .map(|entry| {
            let color = if entry.success {
                Color::Green
            } else {
                Color::Red
            };
            let icon = if entry.success { "✓" } else { "✗" };

            Line::from(vec![
                Span::styled(icon, Style::default().fg(color)),
                Span::raw(" "),
                Span::styled(&entry.action, Style::default().fg(Color::Cyan)),
                Span::raw(": "),
                Span::styled(&entry.details, Style::default().fg(Color::White)),
            ])
        })
        .collect();

    let text = if log_entries.is_empty() {
        vec![Line::from(vec![Span::styled(
            "No recent activity",
            Style::default().fg(Color::Gray),
        )])]
    } else {
        log_entries
    };

    let block = Block::default()
        .title("Recent Activity")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let paragraph = Paragraph::new(text).block(block);
    f.render_widget(paragraph, area);
}

fn render_command_input(f: &mut Frame, area: Rect, app: &App, _theme: &Theme) {
    let text = vec![Line::from(vec![
        Span::styled(
            "ollama ",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            &app.state.ollama_state.command_input,
            Style::default().fg(Color::White),
        ),
        Span::styled(
            "_",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::SLOW_BLINK),
        ),
    ])];

    let block = Block::default()
        .title("Command Input (Enter: Execute, Esc: Cancel)")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let paragraph = Paragraph::new(text).block(block);
    f.render_widget(paragraph, area);
}

fn render_help(f: &mut Frame, area: Rect, _theme: &Theme) {
    let help_text = vec![Line::from(vec![
        Span::styled("Quick Actions: ", Style::default().fg(Color::Gray)),
        Span::styled(
            "R",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(":Run  "),
        Span::styled(
            "S",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(":Stop  "),
        Span::styled(
            "D",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(":Delete  "),
        Span::styled(
            "P",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(":Pull  "),
        Span::styled(
            "L",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(":Refresh  "),
        Span::styled(
            "C",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(":Command  "),
        Span::styled(
            "V",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(":View"),
    ])];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Gray));

    let paragraph = Paragraph::new(help_text).block(block);
    f.render_widget(paragraph, area);
}
