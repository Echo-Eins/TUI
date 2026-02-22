use crate::app::{AppState, console_state::ConsoleMode};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

pub fn render(f: &mut Frame, state: &mut AppState, area: ratatui::layout::Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),    // Output history area
            Constraint::Length(1), // Status bar
            Constraint::Length(3), // Input area
        ])
        .split(area);

    let output_history_area = chunks[0];
    let status_area = chunks[1];
    let input_area = chunks[2];

    // --- Render Output History ---
    let history_lines: Vec<Line> = state
        .console_state
        .output_history
        .iter()
        .map(|msg| {
            Line::from(Span::styled(
                msg.text.clone(),
                Style::default().fg(msg.color),
            ))
        })
        .collect();

    let output_block = Block::default()
        .borders(Borders::ALL)
        .title(" Console Output ")
        .style(Style::default().fg(Color::Cyan));

    // Calculate maximum scroll based on content height and block height
    let inner_height = output_block.inner(output_history_area).height as usize;
    let content_height = history_lines.len();
    
    // Auto-scroll logic: if we have more lines than the view, and scroll_offset is 0,
    // show the bottom-most lines. scroll_offset represents how many lines UP from the bottom we scrolled.
    let max_scroll = content_height.saturating_sub(inner_height);
    let view_offset = if max_scroll > 0 {
        max_scroll.saturating_sub(state.console_state.scroll_offset as usize)
    } else {
        0
    };

    let paragraph = Paragraph::new(history_lines)
        .block(output_block)
        .wrap(Wrap { trim: false })
        .scroll((view_offset as u16, 0));

    f.render_widget(paragraph, output_history_area);

    // --- Render Status Bar ---
    let mode_str = match state.console_state.mode {
        ConsoleMode::Normal => " NORMAL ",
        ConsoleMode::Insert => " INSERT ",
    };
    
    let mode_color = match state.console_state.mode {
        ConsoleMode::Normal => Color::LightBlue,
        ConsoleMode::Insert => Color::LightGreen,
    };

    let running_status = if state.console_state.is_running {
        " [RUNNING] "
    } else {
        ""
    };

    let scroll_status = if state.console_state.scroll_offset > 0 {
        format!(" [SCROLL UP {}] ", state.console_state.scroll_offset)
    } else {
        String::new()
    };

    let status_line = Line::from(vec![
        Span::styled(mode_str, Style::default().bg(mode_color).fg(Color::Black)),
        Span::styled(
            format!("{}{}", running_status, scroll_status), 
            Style::default().fg(Color::Yellow)
        ),
        Span::raw(" | Press 'i' to insert, 'Esc' to return to normal mode"),
    ]);

    let status_paragraph = Paragraph::new(status_line)
        .style(Style::default().bg(Color::DarkGray));
    
    f.render_widget(status_paragraph, status_area);

    // --- Render Input Area ---
    let input_block_color = if state.console_state.mode == ConsoleMode::Insert {
        Color::LightGreen
    } else {
        Color::DarkGray
    };

    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(input_block_color));

    let input_text = format!("> {}", state.console_state.input_buffer);
    let input_paragraph = Paragraph::new(input_text).block(input_block);

    f.render_widget(input_paragraph, input_area);

    // Render cursor if in insert mode
    if state.console_state.mode == ConsoleMode::Insert {
        let cursor_x = input_area.x + 2 + state.console_state.cursor_position as u16;
        let cursor_y = input_area.y + 1;
        
        // Ensure cursor doesn't render completely outside the box
        if cursor_x < input_area.x + input_area.width - 1 {
            f.set_cursor(cursor_x, cursor_y);
        }
    }
}

