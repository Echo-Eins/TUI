pub mod tabs;
pub mod theme;
pub mod widgets;

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Tabs as RatatuiTabs},
    Frame,
};

use crate::app::{App, TabType};
use theme::Theme;

pub fn render(f: &mut Frame, app: &mut App) {
    // Get the full size of the frame
    let size = f.area();

    // Render a background block to ensure the frame is filled
    // This forces ratatui to update the entire screen
    let background = Block::default().style(Style::default().bg(Color::Reset));
    f.render_widget(background, size);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Length(3), // Tabs
            Constraint::Min(0),    // Content
            Constraint::Length(3), // Footer/Command input
        ])
        .split(size);

    render_header(f, chunks[0], app);
    render_tabs(f, chunks[1], app);
    render_content(f, chunks[2], app);
    render_footer(f, chunks[3], app);
}

fn render_header(f: &mut Frame, area: Rect, app: &mut App) {
    let config = app.state.config.read();
    let theme = Theme::from_config(&config);
    let title = format!("{} System Monitor v1.0", config.general.app_name);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.foreground));

    let text = Paragraph::new(title)
        .block(block)
        .alignment(Alignment::Center)
        .style(
            Style::default()
                .fg(theme.foreground)
                .add_modifier(Modifier::BOLD),
        );

    f.render_widget(text, area);
}

fn render_tabs(f: &mut Frame, area: Rect, app: &mut App) {
    let config = app.state.config.read();
    let theme = Theme::from_config(&config);
    let highlight_config = &config.ui.section_highlight;

    let tab_titles: Vec<Line> = app
        .state
        .tab_manager
        .tabs
        .iter()
        .enumerate()
        .map(|(i, tab)| {
            let is_selected = i == app.state.tab_manager.current_index;
            let tab_name = tab.as_str();

            if is_selected {
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
                    Span::styled(tab_name, Style::default().fg(Color::White)),
                    Span::raw(bracket_right),
                ])
            }
        })
        .collect();

    let tabs = RatatuiTabs::new(tab_titles)
        .block(Block::default().borders(Borders::ALL))
        .select(app.state.tab_manager.current_index)
        .style(Style::default().fg(theme.foreground))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );

    f.render_widget(tabs, area);
}

fn render_content(f: &mut Frame, area: Rect, app: &mut App) {
    match app.state.tab_manager.current() {
        TabType::Cpu => tabs::cpu::render(f, area, app),
        TabType::Gpu => tabs::gpu::render(f, area, app),
        TabType::Ram => tabs::ram::render(f, area, app),
        TabType::Disk => tabs::disk::render(f, area, app),
        TabType::Network => tabs::network::render(f, area, app),
        TabType::Ollama => tabs::ollama::render(f, area, app),
        TabType::Processes => tabs::processes::render(f, area, app),

        TabType::Services => tabs::services::render(f, area, app),
        TabType::Console => tabs::console::render(f, &mut app.state, area),
        TabType::DiskAnalyzer => tabs::disk_analyzer::render(f, area, app),
        TabType::Settings => tabs::settings::render(f, area, app),
    }
}

fn render_footer(f: &mut Frame, area: Rect, app: &mut App) {
    let help_text = match app.state.tab_manager.current() {
        TabType::Cpu => "[Up/Down] Navigate | [p/n/c/t/m] Sort | [PgUp/PgDn] Page | [F2] Compact | [Tab] Next Tab | [Ctrl+C] Exit",
        TabType::Gpu => "[Up/Down] Navigate | [p/n/g/m/t] Sort | [PgUp/PgDn] Page | [F2] Compact | [Tab] Next Tab | [Ctrl+C] Exit",
        TabType::Ram => "[Left/Right] Focus | [Up/Down] Navigate | [p/n/w/b] Sort | [F2] Compact | [Tab] Next Tab | [Ctrl+C] Exit",
        TabType::Disk => "[F2] Compact | [Tab] Next Tab | [1-0] Switch Tab | [Ctrl+C] Exit",
            TabType::Network => "[Up/Down] Select Tool | [I] Edit Target | [Enter] Run | [PgUp/PgDn/Home/End] Scroll Details | [X] Cancel | [K] Clear | [E/D/R/F/C/P/T/M/O/N/A/Y] Quick Run | [F2] Compact",
        _ => "[F2] Compact | [Tab] Next Tab | [1-0] Switch Tab | [Ctrl+C] Exit",
    };

    let block = Block::default().borders(Borders::ALL);
    let paragraph = Paragraph::new(help_text)
        .block(block)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::Gray));

    f.render_widget(paragraph, area);
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
