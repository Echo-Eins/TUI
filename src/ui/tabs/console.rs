use crate::app::{AppState, console_state::{ConsoleMode, CommandBlock}};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect, Alignment},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block as UiBlock, Borders, Clear, Paragraph, Wrap},
    Frame,
};

pub fn render(f: &mut Frame, state: &mut AppState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header bar
            Constraint::Min(5),   // Output / blocks area
            Constraint::Length(1), // Status bar
            Constraint::Length(3), // Input area
        ])
        .split(area);

    let header_area = chunks[0];
    let output_area = chunks[1];
    let status_area = chunks[2];
    let input_area = chunks[3];

    render_header(f, state, header_area);

    if state.console_state.blocks.is_empty() {
        render_session_dashboard(f, state, output_area);
    } else {
        render_blocks(f, state, output_area);
    }

    render_status_bar(f, state, status_area);
    render_input(f, state, input_area);

    // History search overlay (renders on top of output area)
    if state.console_state.mode == ConsoleMode::HistorySearch {
        render_history_search(f, state, output_area);
    }

    // Confirm panel overlay (renders on top of output area)
    if state.console_state.mode == ConsoleMode::Confirm {
        render_confirm_panel(f, state, output_area);
    }
}

// ── Header Bar ─────────────────────────────────────────────────────────────

fn render_header(f: &mut Frame, state: &AppState, area: Rect) {
    let cwd = &state.console_state.cwd;
    let user_host = format!(
        "{}@{}",
        state.console_state.username,
        state.console_state.hostname
    );
    let shell = &state.console_state.shell_name;

    let header = Line::from(vec![
        Span::styled(
            format!(" {} ", cwd),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} ", shell),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} ", user_host),
            Style::default().fg(Color::Green),
        ),
    ]);

    let header_widget = Paragraph::new(header)
        .style(Style::default().bg(Color::Rgb(30, 30, 40)));

    f.render_widget(header_widget, area);
}

// ── Session Dashboard (welcome screen) ──────────────────────────────────────

fn render_session_dashboard(f: &mut Frame, state: &AppState, area: Rect) {
    let user_host = format!(
        "{}@{}",
        state.console_state.username,
        state.console_state.hostname
    );

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "╭─ Console Session ──────────────────────────────────────╮",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(vec![
            Span::styled("│  ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("Shell: {}     User: {}", state.console_state.shell_name, user_host),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("│  ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("CWD: {}", state.console_state.cwd),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled("│  ", Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled("│  ", Style::default().fg(Color::Cyan)),
            Span::styled("[i]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(" Insert   "),
            Span::styled("[Esc]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(" Normal   "),
            Span::styled("[Ctrl+R]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(" History   "),
            Span::styled("[Tab/→]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(" Accept"),
        ]),
        Line::from(Span::styled(
            "╰──────────────────────────────────────────────────────╯",
            Style::default().fg(Color::Cyan),
        )),
    ];

    let dashboard = Paragraph::new(lines)
        .alignment(Alignment::Center)
        .style(Style::default());

    f.render_widget(dashboard, area);
}

// ── Block-based Output Rendering ────────────────────────────────────────────

fn render_blocks(f: &mut Frame, state: &mut AppState, area: Rect) {
    let output_block = UiBlock::default()
        .borders(Borders::NONE);

    let inner = output_block.inner(area);
    f.render_widget(output_block, area);

    // Collect all lines from all blocks into a flat list for scrolling
    let mut all_lines: Vec<Line> = Vec::new();

    for block in &state.console_state.blocks {
        // Block header: command + status badge
        let mut header_spans = vec![
            Span::styled("┌─ $ ", Style::default().fg(Color::DarkGray)),
            Span::styled(&block.input, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ];

        // Status badge
        let badge = get_block_badge(block, &state.console_state);
        if let Some((badge_text, badge_color)) = badge {
            header_spans.push(Span::raw("  "));
            header_spans.push(Span::styled(badge_text, Style::default().fg(badge_color)));
        }

        all_lines.push(Line::from(header_spans));

        // Output lines
        for output_line in &block.output_lines {
            all_lines.push(Line::from(vec![
                Span::styled("│ ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    output_line.text.clone(),
                    Style::default().fg(output_line.stream.color()),
                ),
            ]));
        }

        // Sudo & Explain hint lines
        if block.sudo_hint || (block.explain_hint && !block.is_explaining && block.explanation.is_none()) {
            all_lines.push(Line::from(vec![
                Span::styled("│ ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "──────────────────────────────────────",
                    Style::default().fg(Color::DarkGray),
                ),
            ]));

            let mut hint_spans = vec![
                Span::styled("│ ", Style::default().fg(Color::DarkGray)),
            ];

            if block.sudo_hint {
                hint_spans.push(Span::styled(
                    " [Ctrl+S: Re-run with sudo] ",
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ));
            }
            if block.explain_hint && !block.is_explaining && block.explanation.is_none() {
                hint_spans.push(Span::styled(
                    " [Ctrl+E: Explain Error with AI] ",
                    Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
                ));
            }

            all_lines.push(Line::from(hint_spans));
        }

        // Ollama Explanation Rendering
        if block.is_explaining {
            all_lines.push(Line::from(vec![
                Span::styled("│ ", Style::default().fg(Color::DarkGray)),
            ]));
            all_lines.push(Line::from(vec![
                Span::styled("│ ", Style::default().fg(Color::DarkGray)),
                Span::styled(" │ 🤖 Ollama is analyzing the error...", Style::default().fg(Color::Magenta).add_modifier(Modifier::RAPID_BLINK)),
            ]));
        } else if let Some(explanation) = &block.explanation {
            all_lines.push(Line::from(vec![
                Span::styled("│ ", Style::default().fg(Color::DarkGray)),
            ]));
            // Add a small header
            all_lines.push(Line::from(vec![
                Span::styled("│ ", Style::default().fg(Color::DarkGray)),
                Span::styled(" ╭─ 🤖 AI Explanation ───────────────", Style::default().fg(Color::Magenta)),
            ]));
            
            // Handle output text
            for line in explanation.lines() {
                all_lines.push(Line::from(vec![
                    Span::styled("│ ", Style::default().fg(Color::DarkGray)),
                    Span::styled(" │ ", Style::default().fg(Color::Magenta)),
                    Span::raw(line),
                ]));
            }
            all_lines.push(Line::from(vec![
                Span::styled("│ ", Style::default().fg(Color::DarkGray)),
                Span::styled(" ╰───────────────────────────────────", Style::default().fg(Color::Magenta)),
            ]));
        }

        // Block footer
        all_lines.push(Line::from(Span::styled(
            "└──────────────────────────────────────",
            Style::default().fg(Color::DarkGray),
        )));
    }

    // Scrolling logic
    let visible_height = inner.height as usize;
    let content_height = all_lines.len();
    let max_scroll = content_height.saturating_sub(visible_height);
    let view_offset = max_scroll.saturating_sub(state.console_state.scroll_offset as usize);

    let paragraph = Paragraph::new(all_lines)
        .wrap(Wrap { trim: false })
        .scroll((view_offset as u16, 0));

    f.render_widget(paragraph, inner);
}

fn get_block_badge(
    block: &CommandBlock,
    console_state: &crate::app::console_state::ConsoleState,
) -> Option<(String, Color)> {
    if !console_state.should_show_badge(block) {
        return None;
    }

    match &block.state {
        None => {
            // Running — live stopwatch
            let elapsed = block.elapsed_ms();
            let secs = elapsed / 1000;
            let badge = format!("[⟳ {}s]", secs);
            Some((badge, Color::Yellow))
        }
        Some(task_state) => {
            Some((task_state.badge(), task_state.badge_color()))
        }
    }
}

// ── History Search Overlay ──────────────────────────────────────────────────

fn render_history_search(f: &mut Frame, state: &AppState, area: Rect) {
    // Calculate overlay size — take center 70% of the output area
    let overlay_width = (area.width as f32 * 0.75).min(70.0).max(30.0) as u16;
    let overlay_height = (area.height as f32 * 0.7).min(20.0).max(5.0) as u16;

    let overlay = centered_rect(overlay_width, overlay_height, area);

    // Clear the area behind the overlay
    f.render_widget(Clear, overlay);

    let block = UiBlock::default()
        .title(" History Search (Ctrl+R) ")
        .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Rgb(20, 20, 30)));

    let inner = block.inner(overlay);
    f.render_widget(block, overlay);

    if inner.height < 3 {
        return;
    }

    // Split inner into search input + results
    let search_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Search query
            Constraint::Length(1), // Separator
            Constraint::Min(1),   // Results list
        ])
        .split(inner);

    // Search query line
    let query_line = Line::from(vec![
        Span::styled(" > ", Style::default().fg(Color::Yellow)),
        Span::styled(
            &state.console_state.history_search_query,
            Style::default().fg(Color::White),
        ),
        Span::styled("█", Style::default().fg(Color::DarkGray)), // cursor
    ]);

    f.render_widget(Paragraph::new(query_line), search_chunks[0]);

    // Separator
    let sep = "─".repeat(search_chunks[1].width as usize);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(sep, Style::default().fg(Color::DarkGray)))),
        search_chunks[1],
    );

    // Results
    let results_height = search_chunks[2].height as usize;
    let results = &state.console_state.history_search_results;
    let selected = state.console_state.history_search_index;

    let result_lines: Vec<Line> = results
        .iter()
        .take(results_height)
        .enumerate()
        .map(|(i, cmd)| {
            let is_selected = i == selected;
            let indicator = if is_selected { " ▸ " } else { "   " };
            let style = if is_selected {
                Style::default().fg(Color::White).bg(Color::Rgb(50, 50, 70)).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };

            Line::from(vec![
                Span::styled(indicator, style),
                Span::styled(cmd.as_str(), style),
            ])
        })
        .collect();

    if result_lines.is_empty() {
        let empty_msg = if state.console_state.history_search_query.is_empty() {
            "No history yet. Run some commands!"
        } else {
            "No matches found"
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("   {}", empty_msg),
                Style::default().fg(Color::DarkGray),
            ))),
            search_chunks[2],
        );
    } else {
        f.render_widget(Paragraph::new(result_lines), search_chunks[2]);
    }
}

/// Create a centered rect with fixed dimensions.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

// ── Confirm Panel ───────────────────────────────────────────────────────────

fn render_confirm_panel(f: &mut Frame, state: &AppState, area: Rect) {
    let cmd = state.console_state.confirm_command.as_deref().unwrap_or("???");
    let action = state.console_state.confirm_action.as_deref().unwrap_or("Confirm action");

    let panel_width = (area.width.saturating_sub(10)).min(60);
    let panel_height = 7;
    let panel_area = centered_rect(panel_width, panel_height, area);

    // Clear background
    f.render_widget(Clear, panel_area);

    let panel_block = UiBlock::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(format!(" 🔐 {} ", action))
        .title_alignment(Alignment::Center)
        .style(Style::default().bg(Color::Rgb(25, 25, 35)));

    let inner = panel_block.inner(panel_area);
    f.render_widget(panel_block, panel_area);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Command: ", Style::default().fg(Color::DarkGray)),
            Span::styled(cmd, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("[Enter]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(" Execute   ", Style::default().fg(Color::DarkGray)),
            Span::styled("[Esc]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::styled(" Cancel", Style::default().fg(Color::DarkGray)),
        ]),
    ];

    f.render_widget(Paragraph::new(lines), inner);
}

// ── Status Bar ──────────────────────────────────────────────────────────────

fn render_status_bar(f: &mut Frame, state: &AppState, area: Rect) {
    let mode_str = match state.console_state.mode {
        ConsoleMode::Normal => " NORMAL ",
        ConsoleMode::Insert => " INSERT ",
        ConsoleMode::HistorySearch => " SEARCH ",
        ConsoleMode::Confirm => " CONFIRM ",
    };

    let mode_color = match state.console_state.mode {
        ConsoleMode::Normal => Color::LightBlue,
        ConsoleMode::Insert => Color::LightGreen,
        ConsoleMode::HistorySearch => Color::LightMagenta,
        ConsoleMode::Confirm => Color::Yellow,
    };

    let mut status_spans = vec![
        Span::styled(mode_str, Style::default().bg(mode_color).fg(Color::Black)),
    ];

    // Show running indicator
    if state.console_state.is_running() {
        if let Some(block_id) = state.console_state.active_block_id {
            if let Some(block) = state.console_state.get_block(block_id) {
                let elapsed = block.elapsed_ms();
                status_spans.push(Span::styled(
                    format!(" [RUNNING {:.1}s] ", elapsed as f64 / 1000.0),
                    Style::default().fg(Color::Yellow),
                ));
            }
        }
    }

    // Show scroll indicator
    if state.console_state.scroll_offset > 0 {
        status_spans.push(Span::styled(
            format!(" [SCROLL ↑{}] ", state.console_state.scroll_offset),
            Style::default().fg(Color::Cyan),
        ));
    }

    // Help text
    let help = match state.console_state.mode {
        ConsoleMode::Normal => " │ 'i' insert  '↑↓' scroll  Ctrl+S sudo",
        ConsoleMode::Insert => " │ Esc normal  Ctrl+R history  Tab/→ accept  ↑↓ prev/next",
        ConsoleMode::HistorySearch => " │ Esc cancel  Enter accept  ↑↓ navigate",
        ConsoleMode::Confirm => " │ Enter confirm  Esc cancel",
    };

    status_spans.push(Span::styled(help, Style::default().fg(Color::DarkGray)));

    let status_line = Line::from(status_spans);
    let status_paragraph = Paragraph::new(status_line)
        .style(Style::default().bg(Color::Rgb(30, 30, 40)));

    f.render_widget(status_paragraph, area);
}

// ── Input Area with Ghost Text ──────────────────────────────────────────────

fn render_input(f: &mut Frame, state: &mut AppState, area: Rect) {
    let border_color = match state.console_state.mode {
        ConsoleMode::Insert | ConsoleMode::HistorySearch => Color::LightGreen,
        ConsoleMode::Confirm => Color::Yellow,
        ConsoleMode::Normal => Color::DarkGray,
    };

    let input_block = UiBlock::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = input_block.inner(area);

    // Build input line with syntax highlighting + ghost text
    let prompt = "> ";
    let input = &state.console_state.input_buffer;

    let mut spans = vec![
        Span::styled(prompt, Style::default().fg(Color::Cyan)),
    ];

    // Use syntax-highlighted tokens if available, otherwise plain white
    if !state.console_state.highlighted_input.is_empty() {
        for (text, color) in &state.console_state.highlighted_input {
            spans.push(Span::styled(text.as_str(), Style::default().fg(*color)));
        }
    } else {
        spans.push(Span::styled(input.as_str(), Style::default().fg(Color::White)));
    }

    // Ghost text: show the remainder of the suggestion in dark gray
    if let Some(ghost) = &state.console_state.ghost_text {
        if ghost.len() > input.len() && ghost.starts_with(input.as_str()) {
            let remainder = &ghost[input.len()..];
            spans.push(Span::styled(
                remainder,
                Style::default().fg(Color::Rgb(80, 80, 100)),
            ));
        }
    }

    let input_paragraph = Paragraph::new(Line::from(spans)).block(input_block);
    f.render_widget(input_paragraph, area);

    // Render cursor if in insert mode
    if state.console_state.mode == ConsoleMode::Insert {
        let cursor_x = inner.x + prompt.len() as u16 + state.console_state.cursor_position as u16;
        let cursor_y = inner.y;

        if cursor_x < inner.x + inner.width {
            f.set_cursor_position((cursor_x, cursor_y));
        }
    }
}
