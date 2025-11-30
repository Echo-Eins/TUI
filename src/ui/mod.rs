pub mod theme;
pub mod widgets;
pub mod tabs;

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Tabs as RatatuiTabs},
    Frame,
};

use crate::app::{App, TabType};
use theme::Theme;

pub fn render(f: &mut Frame, app: &App) {
    // Clear the entire frame to avoid stale renders on some terminals
    f.render_widget(Clear, f.size());

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Header
            Constraint::Length(3),  // Tabs
            Constraint::Min(0),     // Content
            Constraint::Length(3),  // Footer/Command input
        ])
        .split(f.size());

    render_header(f, chunks[0], app);
    render_tabs(f, chunks[1], app);
    render_content(f, chunks[2], app);
    render_footer(f, chunks[3], app);

    // Render command history menu if active
    if app.state.command_menu_active {
        render_command_menu(f, f.size(), app);
    }
}

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let config = app.state.config.read();
    let theme = Theme::from_config(&config);
    let title = format!("{} System Monitor v1.0", config.general.app_name);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.foreground));

    let text = Paragraph::new(title)
        .block(block)
        .alignment(Alignment::Center)
        .style(Style::default().fg(theme.foreground).add_modifier(Modifier::BOLD));

    f.render_widget(text, area);
}

fn render_tabs(f: &mut Frame, area: Rect, app: &App) {
    let config = app.state.config.read();
    let theme = Theme::from_config(&config);
    let highlight_config = &config.ui.section_highlight;

    let tab_titles: Vec<Line> = app.state.tab_manager.tabs
        .iter()
        .enumerate()
        .map(|(i, tab)| {
            let is_selected = i == app.state.tab_manager.current_index;
            let tab_name = tab.as_str();

            if is_selected {
                // Variant B: Use round brackets and yellow color
                let bracket_left = match highlight_config.highlighted_bracket.as_str() {
                    "round" => "(",
                    "square" => "[",
                    "curly" => "{",
                    _ => "(",
                };
                let bracket_right = match highlight_config.highlighted_bracket.as_str() {
                    "round" => ")",
                    "square" => "]",
                    "curly" => "}",
                    _ => ")",
                };

                Line::from(vec![
                    Span::raw(bracket_left),
                    Span::styled(
                        tab_name,
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(bracket_right),
                ])
            } else {
                // Normal: Use square brackets and white color
                let bracket_left = match highlight_config.normal_bracket.as_str() {
                    "round" => "(",
                    "square" => "[",
                    "curly" => "{",
                    _ => "[",
                };
                let bracket_right = match highlight_config.normal_bracket.as_str() {
                    "round" => ")",
                    "square" => "]",
                    "curly" => "}",
                    _ => "]",
                };

                Line::from(vec![
                    Span::raw(bracket_left),
                    Span::styled(
                        tab_name,
                        Style::default().fg(Color::White),
                    ),
                    Span::raw(bracket_right),
                ])
            }
        })
        .collect();

    let tabs = RatatuiTabs::new(tab_titles)
        .block(Block::default().borders(Borders::ALL))
        .select(app.state.tab_manager.current_index)
        .style(Style::default().fg(theme.foreground))
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));

    f.render_widget(tabs, area);
}

fn render_content(f: &mut Frame, area: Rect, app: &App) {
    match app.state.tab_manager.current() {
        TabType::Cpu => tabs::cpu::render(f, area, app),
        TabType::Gpu => tabs::gpu::render(f, area, app),
        TabType::Ram => tabs::ram::render(f, area, app),
        TabType::Disk => tabs::disk::render(f, area, app),
        TabType::Network => tabs::network::render(f, area, app),
        TabType::Ollama => tabs::ollama::render(f, area, app),
        TabType::Processes => tabs::processes::render(f, area, app),
        TabType::Services => tabs::services::render(f, area, app),
        TabType::DiskAnalyzer => tabs::disk_analyzer::render(f, area, app),
        TabType::Settings => tabs::settings::render(f, area, app),
    }
}

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    let help_text = if app.state.command_input.is_empty() {
        "[F1] Help │ [F2] Compact │ [Tab] Next │ [Ctrl+F] History │ [Ctrl+C] Exit"
    } else {
        &format!("Command: {} [Enter] Execute [Esc] Cancel", app.state.command_input)
    };

    let block = Block::default().borders(Borders::ALL);
    let paragraph = Paragraph::new(help_text)
        .block(block)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::Gray));

    f.render_widget(paragraph, area);
}

fn render_command_menu(f: &mut Frame, _area: Rect, app: &App) {
    // Create centered popup for command history
    let popup_area = centered_rect(60, 60, f.size());

    // Clear background
    let block = Block::default()
        .title("Command History (Ctrl+F)")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(Color::Black));

    f.render_widget(block, popup_area);

    // Render command list
    let inner = Rect {
        x: popup_area.x + 2,
        y: popup_area.y + 2,
        width: popup_area.width.saturating_sub(4),
        height: popup_area.height.saturating_sub(4),
    };

    let commands: Vec<Line> = app.state.command_history
        .get_all()
        .iter()
        .enumerate()
        .map(|(i, cmd)| {
            let is_selected = i == app.state.command_history.selected_index();
            let style = if is_selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            Line::from(vec![
                Span::raw(if is_selected { "► " } else { "  " }),
                Span::styled(cmd.clone(), style),
            ])
        })
        .collect();

    let paragraph = Paragraph::new(commands)
        .style(Style::default().fg(Color::White));

    f.render_widget(paragraph, inner);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
