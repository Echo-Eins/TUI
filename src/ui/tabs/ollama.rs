use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Gauge, Paragraph, Row, Table, Wrap},
    Frame,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::app::{
    state::{
        sort_ollama_models, sort_ollama_running, ChatRole, ChatSession, OllamaActivityView,
        OllamaInputMode, OllamaModelSortColumn, OllamaPanelFocus, OllamaRunningSortColumn,
        OllamaView,
    },
    App,
};
use crate::ui::theme::Theme;
use crate::integrations::ollama::{ChatLogEntry, RunningModel};

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
    let prompt_height = if app.state.ollama_state.input_mode == OllamaInputMode::Chat {
        app.state.ollama_state.chat_prompt_height.max(3)
    } else {
        3
    };
    let max_prompt = area
        .height
        .saturating_sub(3 + 8 + 5 + 6)
        .max(3);
    let prompt_height = prompt_height.min(max_prompt);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(10),   // Main content (models/running)
            Constraint::Length(8), // Running models / VRAM panel
            Constraint::Length(5), // Activity log
            Constraint::Length(prompt_height), // Command input / Help
        ])
        .split(area);

    // Render header
    render_header(f, chunks[0], data, app, theme);

    if app.state.ollama_state.chat_active {
        render_chat_view(f, chunks[1], app, theme);
    } else {
        match app.state.ollama_state.current_view {
            OllamaView::Models => render_models_table(f, chunks[1], data, app, theme),
            OllamaView::Running => render_running_models_table(f, chunks[1], data, app, theme),
        }
    }

    // Render VRAM/GPU panel
    render_vram_panel(f, chunks[2], data, app, theme);

    // Render activity log
    render_activity_log(f, chunks[3], data, app, theme);

    // Render command input or help
    if app.state.ollama_state.input_mode == OllamaInputMode::Chat {
        render_action_input(f, chunks[4], app, theme);
    } else {
        render_help(f, chunks[4], app, theme);
    }

    if app.state.ollama_state.show_delete_confirm {
        render_delete_confirm(f, area, app, theme);
    }
    if app.state.ollama_state.input_mode == OllamaInputMode::Pull {
        render_pull_modal(f, area, app, theme);
    }
    if app.state.ollama_state.input_mode == OllamaInputMode::Command {
        render_command_modal(f, area, app, theme);
    }
}

fn render_compact(
    f: &mut Frame,
    area: Rect,
    data: &crate::integrations::OllamaData,
    app: &App,
    theme: &Theme,
) {
    let prompt_height = if app.state.ollama_state.input_mode == OllamaInputMode::Chat {
        app.state.ollama_state.chat_prompt_height.max(3)
    } else {
        3
    };
    let max_prompt = area.height.saturating_sub(3 + 6).max(3);
    let prompt_height = prompt_height.min(max_prompt);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(10),   // Main content
            Constraint::Length(prompt_height), // Help
        ])
        .split(area);

    // Render header
    render_header(f, chunks[0], data, app, theme);

    if app.state.ollama_state.chat_active {
        render_chat_view(f, chunks[1], app, theme);
    } else {
        match app.state.ollama_state.current_view {
            OllamaView::Models => render_models_table(f, chunks[1], data, app, theme),
            OllamaView::Running => render_running_models_table(f, chunks[1], data, app, theme),
        }
    }

    // Render help
    if app.state.ollama_state.input_mode == OllamaInputMode::Chat {
        render_action_input(f, chunks[2], app, theme);
    } else {
        render_help(f, chunks[2], app, theme);
    }

    if app.state.ollama_state.show_delete_confirm {
        render_delete_confirm(f, area, app, theme);
    }
    if app.state.ollama_state.input_mode == OllamaInputMode::Pull {
        render_pull_modal(f, area, app, theme);
    }
    if app.state.ollama_state.input_mode == OllamaInputMode::Command {
        render_command_modal(f, area, app, theme);
    }
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
    theme: &Theme,
) {
    let mut models = data.models.clone();
    sort_ollama_models(
        &mut models,
        app.state.ollama_state.model_sort_column,
        app.state.ollama_state.model_sort_ascending,
    );
    let sort_indicator = if app.state.ollama_state.model_sort_ascending {
        "↑"
    } else {
        "↓"
    };

    let headers = vec![
        Cell::from(
            if app.state.ollama_state.model_sort_column == OllamaModelSortColumn::Name {
                format!("Name {sort_indicator}")
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
            if app.state.ollama_state.model_sort_column == OllamaModelSortColumn::Params {
                format!("Params {sort_indicator}")
            } else {
                "Params".to_string()
            },
        )
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Size").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from(
            if app.state.ollama_state.model_sort_column == OllamaModelSortColumn::Modified {
                format!("Modified {sort_indicator}")
            } else {
                "Modified".to_string()
            },
        )
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    let header = Row::new(headers).height(1);

    let selected_index = if models.is_empty() {
        0
    } else {
        app.state
            .ollama_state
            .selected_model_index
            .min(models.len().saturating_sub(1))
    };

    let content_height = area.height.saturating_sub(2);
    let header_height = 1u16;
    let visible_rows = content_height.saturating_sub(header_height) as usize;
    let scroll_offset = if visible_rows == 0 {
        0
    } else {
        selected_index.saturating_sub(visible_rows.saturating_sub(1))
    };

    let rows: Vec<Row> = models
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
                Cell::from(model.params_display.clone()).style(style),
                Cell::from(model.size_display.clone()).style(style),
                Cell::from(model.modified.clone()).style(style),
            ])
        })
        .collect();

    let main_focused = app.state.ollama_state.focused_panel == OllamaPanelFocus::Main;
    let border_color = if main_focused {
        Color::Magenta
    } else {
        theme.foreground
    };

    let block = Block::default()
        .title("Available Models")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let widths = [
        Constraint::Min(24),    // Name
        Constraint::Length(8),  // Params
        Constraint::Length(12), // Size
        Constraint::Min(15),    // Modified
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1)
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Magenta));

    f.render_widget(table, area);
}

fn render_running_models_table(
    f: &mut Frame,
    area: Rect,
    data: &crate::integrations::OllamaData,
    app: &App,
    theme: &Theme,
) {
    let mut running_models = data.running_models.clone();
    let mut known_models = std::collections::HashSet::new();
    for model in &running_models {
        known_models.insert(model.name.to_ascii_lowercase());
    }
    for session in &app.state.ollama_state.paused_chats {
        let key = session.model.to_ascii_lowercase();
        if !known_models.contains(&key) {
            running_models.push(build_paused_running_model(session));
            known_models.insert(key);
        }
    }
    sort_ollama_running(
        &mut running_models,
        app.state.ollama_state.running_sort_column,
        app.state.ollama_state.running_sort_ascending,
        &app.state.ollama_state.paused_chats,
        app.state.ollama_state.active_chat_model.as_deref(),
        &app.state.ollama_state.chat_messages,
    );
    let sort_indicator = if app.state.ollama_state.running_sort_ascending {
        "↑"
    } else {
        "↓"
    };

    let mut message_count_map = std::collections::HashMap::new();
    for session in &app.state.ollama_state.paused_chats {
        let count = session
            .messages
            .iter()
            .filter(|message| message.role == ChatRole::Assistant)
            .count();
        message_count_map.insert(session.model.clone(), count);
    }
    if let Some(model) = app.state.ollama_state.active_chat_model.as_deref() {
        let count = app
            .state
            .ollama_state
            .chat_messages
            .iter()
            .filter(|message| message.role == ChatRole::Assistant)
            .count();
        message_count_map.insert(model.to_string(), count);
    }

    let headers = vec![
        Cell::from(
            if app.state.ollama_state.running_sort_column == OllamaRunningSortColumn::Name {
                format!("Name {sort_indicator}")
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
            if app.state.ollama_state.running_sort_column == OllamaRunningSortColumn::Params {
                format!("Params {sort_indicator}")
            } else {
                "Params".to_string()
            },
        )
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("VRAM").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Processor").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from(
            if app.state.ollama_state.running_sort_column == OllamaRunningSortColumn::PausedAt {
                format!("Status {sort_indicator}")
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
            if app.state.ollama_state.running_sort_column
                == OllamaRunningSortColumn::MessageCount
            {
                format!("Msgs {sort_indicator}")
            } else {
                "Msgs".to_string()
            },
        )
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Unload").style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    let header = Row::new(headers).height(1);

    let selected_index = if running_models.is_empty() {
        0
    } else {
        app.state
            .ollama_state
            .selected_running_index
            .min(running_models.len().saturating_sub(1))
    };

    let content_height = area.height.saturating_sub(2);
    let header_height = 1u16;
    let visible_rows = content_height.saturating_sub(header_height) as usize;
    let scroll_offset = if visible_rows == 0 {
        0
    } else {
        selected_index.saturating_sub(visible_rows.saturating_sub(1))
    };

    let rows: Vec<Row> = running_models
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

            let paused = app
                .state
                .ollama_state
                .paused_chats
                .iter()
                .find(|session| session.model == model.name);
            let status_text = paused
                .map(|session| {
                    format!(
                        "Paused {} ({})",
                        session.paused_at_display,
                        format_elapsed_short(session.paused_at)
                    )
                })
                .unwrap_or_else(|| "Running".to_string());
            let status_style = if paused.is_some() {
                style.fg(Color::Yellow)
            } else {
                style.fg(Color::Green)
            };
            let unload_style = if is_selected {
                Style::default().fg(Color::Black).bg(Color::Red)
            } else {
                Style::default().fg(Color::Red)
            };

            Row::new(vec![
                Cell::from(model.name.clone()).style(style),
                Cell::from(model.params_display.clone()).style(style),
                Cell::from(model.gpu_memory_display.clone()).style(style),
                Cell::from(model.processor.clone()).style(style.fg(Color::Cyan)),
                Cell::from(status_text).style(status_style),
                Cell::from(
                    message_count_map
                        .get(&model.name)
                        .copied()
                        .unwrap_or(0)
                        .to_string(),
                )
                .style(style),
                Cell::from("Unload").style(unload_style),
            ])
        })
        .collect();

    let main_focused = app.state.ollama_state.focused_panel == OllamaPanelFocus::Main;
    let border_color = if main_focused {
        Color::Green
    } else {
        theme.foreground
    };

    let block = Block::default()
        .title("Running Models")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let widths = [
        Constraint::Min(20),    // Name
        Constraint::Length(8),  // Params
        Constraint::Length(10), // VRAM
        Constraint::Length(15), // Processor
        Constraint::Min(16),    // Status
        Constraint::Length(6),  // Msgs
        Constraint::Length(8),  // Unload
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(block)
        .column_spacing(1)
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Green));

    f.render_widget(table, area);
}

fn render_chat_view(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let model_name = app
        .state
        .ollama_state
        .active_chat_model
        .as_deref()
        .unwrap_or("Unknown");
    let title = format!("Chat - {}", model_name);

    let mut lines = format_chat_lines(&app.state.ollama_state.chat_messages);
    if lines.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "No chat history yet. Type a prompt to start.",
            Style::default().fg(Color::Gray),
        )]));
    }

    let content_height = area.height.saturating_sub(2) as usize;
    let max_scroll = lines.len().saturating_sub(content_height);
    let scroll = app.state.ollama_state.chat_scroll.min(max_scroll);

    let main_focused = app.state.ollama_state.focused_panel == OllamaPanelFocus::Main;
    let border_color = if main_focused {
        Color::Magenta
    } else {
        theme.foreground
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((scroll as u16, 0));

    f.render_widget(paragraph, area);
}

fn format_chat_lines(messages: &[crate::app::state::ChatMessage]) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    for message in messages {
        let (label, color) = match message.role {
            ChatRole::User => ("You:", Color::Cyan),
            ChatRole::Assistant => ("Assistant:", Color::Magenta),
        };
        let mut message_lines = message.text.lines();
        if let Some(first) = message_lines.next() {
            lines.push(Line::from(vec![
                Span::styled(label, Style::default().fg(color).add_modifier(Modifier::BOLD)),
                Span::raw(" "),
                Span::styled(first, Style::default().fg(Color::White)),
            ]));
        }
        for line in message_lines {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(line, Style::default().fg(Color::White)),
            ]));
        }
        lines.push(Line::from(""));
    }
    lines
}

fn wrap_text_lines(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut line_len = 0usize;
    for ch in text.chars() {
        if ch == '\n' {
            lines.push(current);
            current = String::new();
            line_len = 0;
            continue;
        }
        line_len += 1;
        if line_len > width {
            lines.push(current);
            current = String::new();
            line_len = 1;
        }
        current.push(ch);
    }
    lines.push(current);
    lines
}

fn build_activity_row(
    entry: &ChatLogEntry,
    model_width: usize,
    date_width: usize,
    prompt_width: usize,
) -> String {
    let model = pad_to_width(&trim_with_ellipsis(&entry.model, model_width), model_width);
    let date = pad_to_width(
        &trim_with_ellipsis(&entry.ended_at_display, date_width),
        date_width,
    );
    let prompt = if prompt_width == 0 {
        String::new()
    } else {
        let flat_prompt = entry.last_prompt.replace('\n', " ");
        let trimmed_prompt = flat_prompt.trim();
        trim_with_ellipsis(
            if trimmed_prompt.is_empty() {
                "-"
            } else {
                trimmed_prompt
            },
            prompt_width,
        )
    };

    if prompt_width == 0 {
        format!("{model}  {date}")
    } else {
        format!("{model}  {date}  {prompt}")
    }
}

fn trim_with_ellipsis(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let len = text.chars().count();
    if len <= width {
        return text.to_string();
    }
    if width <= 3 {
        return text.chars().take(width).collect();
    }
    let trimmed: String = text.chars().take(width - 3).collect();
    format!("{trimmed}...")
}

fn pad_to_width(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut count = 0usize;
    for ch in text.chars() {
        if count >= width {
            break;
        }
        out.push(ch);
        count += 1;
    }
    if count < width {
        out.push_str(&" ".repeat(width - count));
    }
    out
}

fn format_elapsed_short(epoch_seconds: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let elapsed = now.saturating_sub(epoch_seconds);
    if elapsed < 60 {
        format!("{elapsed}s")
    } else if elapsed < 3600 {
        format!("{}m", elapsed / 60)
    } else if elapsed < 86_400 {
        format!("{}h", elapsed / 3600)
    } else {
        format!("{}d", elapsed / 86_400)
    }
}

fn build_paused_running_model(session: &ChatSession) -> RunningModel {
    let (params_value, params_unit, params_display) = parse_params_from_name(&session.model);
    let is_cloud = session.model.to_ascii_lowercase().contains("cloud");
    RunningModel {
        name: session.model.clone(),
        size_bytes: 0,
        size_display: "-".to_string(),
        gpu_memory_mb: None,
        gpu_memory_display: if is_cloud { "cloud".to_string() } else { "-".to_string() },
        params_value,
        params_unit,
        params_display,
        processor: "Paused".to_string(),
        until: None,
    }
}

fn parse_params_from_name(name: &str) -> (Option<f64>, Option<char>, String) {
    let chars: Vec<char> = name.chars().collect();
    for (idx, ch) in chars.iter().enumerate() {
        let unit = ch.to_ascii_uppercase();
        if !matches!(unit, 'M' | 'B' | 'T') {
            continue;
        }
        if idx == 0 {
            continue;
        }
        let mut start = idx;
        while start > 0 {
            let prev = chars[start - 1];
            if prev.is_ascii_digit() || prev == '.' {
                start -= 1;
            } else {
                break;
            }
        }
        if start == idx {
            continue;
        }
        let num_str: String = chars[start..idx].iter().collect();
        if let Ok(value) = num_str.parse::<f64>() {
            let display = format_param_display(value, unit);
            return (Some(value), Some(unit), display);
        }
    }
    (None, None, "-".to_string())
}

fn format_param_display(value: f64, unit: char) -> String {
    if (value.fract() - 0.0).abs() < f64::EPSILON {
        format!("{:.0}{}", value, unit)
    } else {
        let mut text = format!("{:.2}", value);
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
        format!("{text}{unit}")
    }
}

fn format_bytes_gb(value: u64) -> String {
    let gb = value as f64 / 1_073_741_824.0;
    format!("{:.2} GB", gb)
}

fn render_vram_panel(
    f: &mut Frame,
    area: Rect,
    data: &crate::integrations::OllamaData,
    app: &App,
    theme: &Theme,
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

    let vram_text = if vram_total > 0 {
        format!(
            "{:.2}% ({} / {})",
            vram_percent,
            format_bytes_gb(vram_used),
            format_bytes_gb(vram_total)
        )
    } else {
        "0.00% (0.00 GB / 0.00 GB)".to_string()
    };
    let vram_focused = app.state.ollama_state.focused_panel == OllamaPanelFocus::Vram;
    let vram_border = if vram_focused {
        Color::Cyan
    } else {
        theme.foreground
    };

    let gauge = Gauge::default()
        .block(
            Block::default()
                .title("VRAM Usage")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(vram_border)),
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

    let total_vram_mb: u64 = data
        .running_models
        .iter()
        .filter_map(|model| model.gpu_memory_mb)
        .sum();
    summary.push(Line::from(vec![
        Span::styled("Total VRAM: ", Style::default().fg(Color::Gray)),
        Span::styled(
            if total_vram_mb > 0 {
                let total_bytes = total_vram_mb.saturating_mul(1_048_576);
                format_bytes_gb(total_bytes)
            } else {
                "-".to_string()
            },
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
        .border_style(Style::default().fg(vram_border));

    let paragraph = Paragraph::new(summary).block(block);
    f.render_widget(paragraph, chunks[1]);
}

fn render_activity_log(
    f: &mut Frame,
    area: Rect,
    data: &crate::integrations::OllamaData,
    app: &App,
    theme: &Theme,
) {
    let activity_focused = app.state.ollama_state.focused_panel == OllamaPanelFocus::Activity;
    let additions_focused = app.state.ollama_state.focused_panel == OllamaPanelFocus::Additions;
    let show_additions = app.state.ollama_state.activity_additions_open
        && app.state.ollama_state.activity_view == OllamaActivityView::List;

    if show_additions {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
            .split(area);
        render_activity_list_panel(f, chunks[0], data, app, theme, activity_focused);
        render_activity_additions_panel(f, chunks[1], data, app, theme, additions_focused);
        return;
    }

    let border_color = if activity_focused {
        Color::Yellow
    } else {
        theme.foreground
    };

    let title = match app.state.ollama_state.activity_view {
        OllamaActivityView::List => "Recent Activity".to_string(),
        OllamaActivityView::Log => {
            if app.state.ollama_state.activity_log_title.is_empty() {
                "Log".to_string()
            } else {
                app.state.ollama_state.activity_log_title.clone()
            }
        }
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let content_height = area.height.saturating_sub(2) as usize;
    let paragraph = match app.state.ollama_state.activity_view {
        OllamaActivityView::List => {
            build_activity_list_paragraph(area, data, app, activity_focused, block)
        }
        OllamaActivityView::Log => {
            let lines: Vec<Line> = if app.state.ollama_state.activity_log_lines.is_empty() {
                vec![Line::from(vec![Span::styled(
                    "No log loaded",
                    Style::default().fg(Color::Gray),
                )])]
            } else {
                app.state
                    .ollama_state
                    .activity_log_lines
                    .iter()
                    .map(|line| Line::from(line.clone()))
                    .collect()
            };

            let max_scroll = lines.len().saturating_sub(content_height);
            let scroll = app.state.ollama_state.activity_log_scroll.min(max_scroll);

            Paragraph::new(lines)
                .block(block)
                .wrap(Wrap { trim: false })
                .scroll((scroll as u16, 0))
        }
    };

    f.render_widget(paragraph, area);
}

fn render_activity_list_panel(
    f: &mut Frame,
    area: Rect,
    data: &crate::integrations::OllamaData,
    app: &App,
    theme: &Theme,
    focused: bool,
) {
    let border_color = if focused {
        Color::Yellow
    } else {
        theme.foreground
    };

    let block = Block::default()
        .title("Recent Activity")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));
    let paragraph = build_activity_list_paragraph(area, data, app, focused, block);
    f.render_widget(paragraph, area);
}

fn render_activity_additions_panel(
    f: &mut Frame,
    area: Rect,
    data: &crate::integrations::OllamaData,
    app: &App,
    theme: &Theme,
    focused: bool,
) {
    let border_color = if focused {
        Color::Yellow
    } else {
        theme.foreground
    };
    let block = Block::default()
        .title("Additions")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let entries = ["Chat restart"];
    let has_logs = !data.chat_logs.is_empty();
    let mut lines: Vec<Line> = Vec::new();
    for (idx, label) in entries.iter().enumerate() {
        let is_selected = focused && app.state.ollama_state.activity_additions_selected == idx;
        let fg = if has_logs {
            Color::White
        } else {
            Color::DarkGray
        };
        let style = if is_selected {
            Style::default().fg(Color::Black).bg(Color::Yellow)
        } else {
            Style::default().fg(fg)
        };
        lines.push(Line::from(Span::styled(*label, style)));
    }

    let paragraph = Paragraph::new(lines).block(block);
    f.render_widget(paragraph, area);
}

fn build_activity_list_paragraph(
    area: Rect,
    data: &crate::integrations::OllamaData,
    app: &App,
    focused: bool,
    block: Block<'static>,
) -> Paragraph<'static> {
    let logs = &data.chat_logs;
    if logs.is_empty() {
        return Paragraph::new(vec![Line::from(vec![Span::styled(
            "No recent chats",
            Style::default().fg(Color::Gray),
        )])])
        .block(block);
    }

    let content_height = area.height.saturating_sub(2) as usize;
    let selected = app
        .state
        .ollama_state
        .activity_selected
        .min(logs.len().saturating_sub(1));
    let expanded = focused
        && !app.state.ollama_state.activity_expand_suppressed
        && app.state.ollama_state.activity_view == OllamaActivityView::List
        && app.state.ollama_state.activity_expand_row == Some(selected)
        && app
            .state
            .ollama_state
            .activity_expand_started_at
            .map(|started| started.elapsed() >= Duration::from_secs(2))
            .unwrap_or(false);

    struct ActivityLine {
        line: Line<'static>,
    }

    let max_width = area.width.saturating_sub(2) as usize;
    let date_width = 16usize.min(max_width);
    let model_width = (max_width / 3).max(12).min(30);
    let prompt_width = max_width
        .saturating_sub(model_width)
        .saturating_sub(date_width)
        .saturating_sub(4);

    let mut lines: Vec<ActivityLine> = Vec::new();
    let mut selected_line_index = 0usize;
    let mut expanded_block: Option<(usize, usize)> = None;

    for (idx, entry) in logs.iter().enumerate() {
        let row_text = build_activity_row(entry, model_width, date_width, prompt_width);
        let is_selected = idx == selected && focused;
        let row_style = if is_selected {
            Style::default().fg(Color::Black).bg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(ActivityLine {
            line: Line::from(Span::styled(row_text, row_style)),
        });
        if idx == selected {
            selected_line_index = lines.len().saturating_sub(1);
        }
        if idx == selected && expanded {
            let prompt = entry.last_prompt.trim();
            let prompt = if prompt.is_empty() {
                "No last prompt"
            } else {
                prompt
            };
            let wrap_width = max_width.saturating_sub(2);
            let wrapped = wrap_text_lines(prompt, wrap_width);
            if !wrapped.is_empty() {
                expanded_block = Some((lines.len(), wrapped.len()));
            }
            for line in wrapped {
                lines.push(ActivityLine {
                    line: Line::from(Span::styled(
                        format!("  {line}"),
                        Style::default().fg(Color::Gray),
                    )),
                });
            }
        }
    }

    let visible = content_height.max(1);
    let max_scroll = lines.len().saturating_sub(visible);
    let mut scroll = if selected_line_index >= visible {
        selected_line_index - (visible - 1)
    } else {
        0
    };
    if let Some((start, count)) = expanded_block {
        let end = start.saturating_add(count.saturating_sub(1));
        let last_visible = scroll.saturating_add(visible.saturating_sub(1));
        if end > last_visible {
            scroll = end.saturating_sub(visible.saturating_sub(1));
        }
        if start < scroll {
            scroll = start;
        }
    }
    if scroll > max_scroll {
        scroll = max_scroll;
    }

    let visible_lines: Vec<Line> = lines
        .into_iter()
        .skip(scroll)
        .take(visible)
        .map(|item| item.line)
        .collect();

    Paragraph::new(visible_lines).block(block)
}

fn render_action_input(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mode = app.state.ollama_state.input_mode;
    let title = match mode {
        OllamaInputMode::Pull => "Pull Model (Enter: Start, Esc: Cancel)".to_string(),
        OllamaInputMode::Command => "Command (Enter: Run, Esc: Cancel)".to_string(),
        OllamaInputMode::Chat => {
            if let Some(model) = app.state.ollama_state.active_chat_model.as_ref() {
                format!("Chat Prompt [{}] (Enter: Send, Esc: End Chat)", model)
            } else {
                "Chat Prompt (Enter: Send, Esc: End Chat)".to_string()
            }
        }
        OllamaInputMode::None => "Action Input".to_string(),
    };
    let prefix = match mode {
        OllamaInputMode::Pull => "pull ",
        OllamaInputMode::Command => "ollama ",
        OllamaInputMode::Chat => "chat ",
        OllamaInputMode::None => "",
    };

    let input_text = format!(
        "{}{}_",
        prefix,
        app.state.ollama_state.input_buffer
    );
    let inner_width = area.width.saturating_sub(2) as usize;
    let wrapped = wrap_text_lines(&input_text, inner_width);
    let mut text_lines: Vec<Line> = Vec::new();
    if wrapped.is_empty() {
        text_lines.push(Line::from(Span::raw("")));
    } else {
        let mut iter = wrapped.into_iter();
        if let Some(first) = iter.next() {
            let trimmed = first
                .strip_prefix(prefix)
                .unwrap_or(&first)
                .to_string();
            text_lines.push(Line::from(vec![
                Span::styled(
                    prefix,
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(trimmed, Style::default().fg(Color::White)),
            ]));
        }
        for line in iter {
            text_lines.push(Line::from(Span::styled(line, Style::default().fg(Color::White))));
        }
    }

    let input_focused = app.state.ollama_state.focused_panel == OllamaPanelFocus::Input;
    let border_color = if input_focused {
        Color::Yellow
    } else {
        theme.foreground
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let scroll = if mode == OllamaInputMode::Chat {
        let max_scroll = text_lines
            .len()
            .saturating_sub(area.height.saturating_sub(2) as usize);
        app.state.ollama_state.chat_prompt_scroll.min(max_scroll)
    } else {
        0
    };
    let paragraph = Paragraph::new(text_lines)
        .block(block)
        .scroll((scroll as u16, 0));
    f.render_widget(paragraph, area);
}

fn render_help(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let narrow = area.width < 120;
    let help_text = if narrow {
        vec![
            Line::from(vec![
                Span::styled("Quick Actions: ", Style::default().fg(Color::Gray)),
                Span::styled(
                    "R",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(":Chat  "),
                Span::styled(
                    "S",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(":Stop  "),
                Span::styled(
                    "U",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(":Unload  "),
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
                    "C",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(":Command"),
            ]),
            Line::from(vec![
                Span::styled("  ", Style::default().fg(Color::Gray)),
                Span::styled(
                    "L",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(":Refresh  "),
                Span::styled(
                    "V",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(":View  "),
                Span::styled(
                    "N/M/T",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(":Sort  "),
                Span::styled(
                    "G",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(":Msgs  "),
                Span::styled(
                    "A",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(":Additions  "),
                Span::styled(
                    "Esc",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(":Back  "),
                Span::styled(
                    "Left/Right",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(":Focus"),
            ]),
        ]
    } else {
        vec![Line::from(vec![
            Span::styled("Quick Actions: ", Style::default().fg(Color::Gray)),
            Span::styled(
                "R",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(":Chat  "),
            Span::styled(
                "S",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(":Stop  "),
            Span::styled(
                "U",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(":Unload  "),
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
                "C",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(":Command  "),
            Span::styled(
                "L",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(":Refresh  "),
            Span::styled(
                "V",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(":View  "),
            Span::styled(
                "N/M/T",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(":Sort  "),
            Span::styled(
                "G",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(":Msgs  "),
            Span::styled(
                "A",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(":Additions  "),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(":Back  "),
            Span::styled(
                "Left/Right",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(":Focus"),
        ])]
    };

    let help_focused = app.state.ollama_state.focused_panel == OllamaPanelFocus::Help;
    let border_color = if help_focused {
        Color::Yellow
    } else {
        theme.foreground
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let paragraph = Paragraph::new(help_text).block(block);
    f.render_widget(paragraph, area);
}

fn render_pull_modal(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let rect = centered_rect(70, 8, area);
    f.render_widget(Clear, rect);

    let input_text = format!("pull {}_", app.state.ollama_state.input_buffer);
    let lines = vec![
        Line::from(vec![
            Span::styled(
                "Enter model name to pull",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(":"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            input_text,
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(": Start  "),
            Span::styled("Esc", Style::default().fg(Color::Cyan)),
            Span::raw(": Cancel  "),
            Span::styled("Left/Right", Style::default().fg(Color::Cyan)),
            Span::raw(": Focus"),
        ]),
    ];

    let input_focused = app.state.ollama_state.focused_panel == OllamaPanelFocus::Input;
    let border_color = if input_focused {
        Color::Yellow
    } else {
        theme.foreground
    };

    let block = Block::default()
        .title("Pull Model")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, rect);
}

fn render_command_modal(f: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let rect = centered_rect(70, 8, area);
    f.render_widget(Clear, rect);

    let input_text = format!("ollama {}_", app.state.ollama_state.input_buffer);
    let lines = vec![
        Line::from(vec![
            Span::styled(
                "Run ollama command",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(":"),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            input_text,
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(": Run  "),
            Span::styled("Esc", Style::default().fg(Color::Cyan)),
            Span::raw(": Cancel  "),
            Span::styled("Left/Right", Style::default().fg(Color::Cyan)),
            Span::raw(": Focus"),
        ]),
    ];

    let input_focused = app.state.ollama_state.focused_panel == OllamaPanelFocus::Input;
    let border_color = if input_focused {
        Color::Yellow
    } else {
        theme.foreground
    };

    let block = Block::default()
        .title("Command")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, rect);
}

fn render_delete_confirm(f: &mut Frame, area: Rect, app: &App, _theme: &Theme) {
    let model_name = app
        .state
        .ollama_state
        .pending_delete
        .clone()
        .unwrap_or_else(|| "selected model".to_string());
    let rect = centered_rect(60, 7, area);

    f.render_widget(Clear, rect);

    let text = vec![
        Line::from(vec![
            Span::styled(
                "Delete model",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw(": "),
            Span::styled(model_name, Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from("This action cannot be undone."),
        Line::from(""),
        Line::from(vec![
            Span::styled("Enter/Y", Style::default().fg(Color::Cyan)),
            Span::raw(": Confirm  "),
            Span::styled("Esc/N", Style::default().fg(Color::Cyan)),
            Span::raw(": Cancel"),
        ]),
    ];

    let block = Block::default()
        .title("Confirm Delete")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));

    let paragraph = Paragraph::new(text).block(block);
    f.render_widget(paragraph, rect);
}

fn centered_rect(percent_width: u16, percent_height: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_height) / 2),
            Constraint::Percentage(percent_height),
            Constraint::Percentage((100 - percent_height) / 2),
        ])
        .split(area);

    let vertical = popup_layout[1];
    let horizontal_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_width) / 2),
            Constraint::Percentage(percent_width),
            Constraint::Percentage((100 - percent_width) / 2),
        ])
        .split(vertical);

    horizontal_layout[1]
}





