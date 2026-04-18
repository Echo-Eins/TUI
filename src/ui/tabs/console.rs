use crate::app::{
    console_state::{
        CommandBlock, CommandOutput, ConsoleMode, ConsolePlotBlock, ConsolePlotMode,
        ConsoleTrigUnitCircleBlock, ConsoleVisualBlock, ConsoleVisualKind,
    },
    AppState,
};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        canvas::{Canvas, Circle, Line as CanvasLine, Points},
        Axis, Block as UiBlock, Borders, Chart, Clear, Dataset, GraphType, Paragraph, Sparkline,
        Wrap,
    },
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn render(f: &mut Frame, state: &mut AppState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header bar
            Constraint::Min(5),    // Output / blocks area
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
        state.console_state.username, state.console_state.hostname
    );
    let shell = &state.console_state.shell_name;

    let header = Line::from(vec![
        Span::styled(format!(" {} ", cwd), Style::default().fg(Color::Cyan)),
        Span::styled(" | ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{} ", shell), Style::default().fg(Color::Yellow)),
        Span::styled(" | ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{} ", user_host), Style::default().fg(Color::Green)),
    ]);

    let header_widget = Paragraph::new(header).style(Style::default().bg(Color::Rgb(30, 30, 40)));

    f.render_widget(header_widget, area);
}

// ── Session Dashboard (welcome screen) ──────────────────────────────────────

fn render_session_dashboard(f: &mut Frame, state: &AppState, area: Rect) {
    let user_host = format!(
        "{}@{}",
        state.console_state.username, state.console_state.hostname
    );

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Console Session",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!(
                "Shell: {} | User: {}",
                state.console_state.shell_name, user_host
            ),
            Style::default().fg(Color::White),
        )),
        Line::from(Span::styled(
            format!("CWD: {}", state.console_state.cwd),
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "[i]",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Insert   "),
            Span::styled(
                "[Esc]",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Normal   "),
            Span::styled(
                "[Ctrl+R]",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" History   "),
            Span::styled(
                "[Tab/Right]",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Accept"),
        ]),
    ];

    let dashboard = Paragraph::new(lines)
        .alignment(Alignment::Center)
        .style(Style::default());

    f.render_widget(dashboard, area);
}

// ── Block-based Output Rendering ────────────────────────────────────────────

fn render_blocks(f: &mut Frame, state: &mut AppState, area: Rect) {
    let output_block = UiBlock::default().borders(Borders::NONE);

    let inner = output_block.inner(area);
    f.render_widget(output_block, area);

    if state.console_state.blocks.iter().any(|block| {
        block.session.is_some()
            || block
                .output_items
                .iter()
                .any(|item| matches!(item, CommandOutput::Plot(_) | CommandOutput::Visual(_)))
    }) {
        render_blocks_typed(f, state, inner);
        return;
    }

    // Collect all lines from all blocks into a flat list for scrolling
    let mut all_lines: Vec<Line> = Vec::new();

    for block in &state.console_state.blocks {
        // Block header: command + status badge
        let mut header_spans = vec![
            Span::styled("$ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                &block.input,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
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
            all_lines.push(output_line_line(output_line));
        }

        // Sudo & Explain hint lines
        if block.sudo_hint
            || (block.explain_hint && !block.is_explaining && block.explanation.is_none())
        {
            all_lines.push(Line::from(vec![
                Span::styled("| ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "--------------------------------------",
                    Style::default().fg(Color::DarkGray),
                ),
            ]));

            let mut hint_spans = vec![Span::styled("| ", Style::default().fg(Color::DarkGray))];

            if block.sudo_hint {
                hint_spans.push(Span::styled(
                    " [Ctrl+S: Re-run with sudo] ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            if block.explain_hint && !block.is_explaining && block.explanation.is_none() {
                hint_spans.push(Span::styled(
                    " [Ctrl+E: Explain Error with AI] ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ));
            }

            all_lines.push(Line::from(hint_spans));
        }

        // Ollama Explanation Rendering
        if block.is_explaining {
            all_lines.push(Line::from(vec![Span::styled(
                "| ",
                Style::default().fg(Color::DarkGray),
            )]));
            all_lines.push(Line::from(vec![
                Span::styled("| ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "AI is analyzing the error...",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::RAPID_BLINK),
                ),
            ]));
        } else if let Some(explanation) = &block.explanation {
            all_lines.push(Line::from(vec![Span::styled(
                "| ",
                Style::default().fg(Color::DarkGray),
            )]));
            // Add a small header
            all_lines.push(Line::from(vec![
                Span::styled("| ", Style::default().fg(Color::DarkGray)),
                Span::styled("AI Explanation", Style::default().fg(Color::Magenta)),
            ]));

            // Handle output text
            for line in explanation.lines() {
                all_lines.push(Line::from(vec![
                    Span::styled("| ", Style::default().fg(Color::DarkGray)),
                    Span::styled("> ", Style::default().fg(Color::Magenta)),
                    Span::raw(line),
                ]));
            }
            all_lines.push(Line::from(vec![
                Span::styled("| ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    "-----------------------------------",
                    Style::default().fg(Color::Magenta),
                ),
            ]));
        }

        // Block footer
        all_lines.push(Line::from(Span::styled(
            "",
            Style::default().fg(Color::DarkGray),
        )));
    }

    // Scrolling logic — clamp scroll_offset to prevent over-scrolling
    let visible_height = inner.height as usize;
    let content_height = all_lines.len();
    let max_scroll = content_height.saturating_sub(visible_height);
    // Clamp scroll_offset to the valid range
    if state.console_state.scroll_offset as usize > max_scroll {
        state.console_state.scroll_offset = max_scroll as u16;
    }
    let view_offset = max_scroll.saturating_sub(state.console_state.scroll_offset as usize);

    let paragraph = Paragraph::new(all_lines)
        .wrap(Wrap { trim: false })
        .scroll((view_offset as u16, 0));

    f.render_widget(paragraph, inner);
}

fn render_blocks_typed(f: &mut Frame, state: &mut AppState, area: Rect) {
    let items = build_console_render_items(state);
    let visible_height = area.height;
    let content_height = items
        .iter()
        .map(|item| item.height(area.width))
        .sum::<u16>();
    let max_scroll = content_height.saturating_sub(visible_height);
    let scroll_offset = state.console_state.scroll_offset.min(max_scroll);
    let view_offset = max_scroll.saturating_sub(scroll_offset);

    let mut consumed = 0u16;
    let mut y = area.y;
    let bottom = area.y.saturating_add(area.height);

    for item in items {
        let height = item.height(area.width);
        if consumed.saturating_add(height) <= view_offset {
            consumed = consumed.saturating_add(height);
            continue;
        }
        if y >= bottom {
            break;
        }

        let clipped_top = view_offset.saturating_sub(consumed);
        let available_height = bottom.saturating_sub(y);
        let render_height = height.saturating_sub(clipped_top).min(available_height);
        if render_height == 0 {
            consumed = consumed.saturating_add(height);
            continue;
        }

        match item {
            ConsoleRenderItem::Line(line) => {
                if clipped_top == 0 {
                    f.render_widget(Paragraph::new(line), Rect::new(area.x, y, area.width, 1));
                    y = y.saturating_add(1);
                }
            }
            ConsoleRenderItem::Plot(plot) => {
                let plot_x = area.x.saturating_add(1);
                let plot_width = area.width.saturating_sub(1);
                render_plot_output(f, plot, Rect::new(plot_x, y, plot_width, render_height));
                y = y.saturating_add(render_height);
            }
            ConsoleRenderItem::Visual(visual) => {
                let visual_x = area.x.saturating_add(1);
                let visual_width = area.width.saturating_sub(1);
                render_visual_output(
                    f,
                    visual,
                    Rect::new(visual_x, y, visual_width, render_height),
                );
                y = y.saturating_add(render_height);
            }
            ConsoleRenderItem::Session(session) => {
                let session_x = area.x.saturating_add(1);
                let session_width = area.width.saturating_sub(1);
                session.render(f, Rect::new(session_x, y, session_width, render_height));
                y = y.saturating_add(render_height);
            }
        }

        consumed = consumed.saturating_add(height);
    }

    state.console_state.scroll_offset = scroll_offset;
}

enum ConsoleRenderItem<'a> {
    Line(Line<'static>),
    Plot(&'a ConsolePlotBlock),
    Visual(&'a ConsoleVisualBlock),
    Session(&'a dyn crate::app::extensions::ConsoleSession),
}

impl ConsoleRenderItem<'_> {
    fn height(&self, width: u16) -> u16 {
        match self {
            Self::Line(_) => 1,
            Self::Plot(plot) => plot_item_height(plot, width),
            Self::Visual(visual) => visual_item_height(visual, width),
            Self::Session(_) => session_item_height(width),
        }
    }
}

fn build_console_render_items(state: &AppState) -> Vec<ConsoleRenderItem<'_>> {
    let mut items = Vec::new();
    for block in &state.console_state.blocks {
        items.push(ConsoleRenderItem::Line(block_header_line(
            block,
            &state.console_state,
        )));

        if block.output_items.is_empty() {
            for output_line in &block.output_lines {
                items.push(ConsoleRenderItem::Line(output_line_line(output_line)));
            }
        } else {
            for output in &block.output_items {
                match output {
                    CommandOutput::Line(output_line) => {
                        items.push(ConsoleRenderItem::Line(output_line_line(output_line)));
                    }
                    CommandOutput::Plot(plot) => items.push(ConsoleRenderItem::Plot(plot)),
                    CommandOutput::Visual(visual) => items.push(ConsoleRenderItem::Visual(visual)),
                }
            }
        }
        if let Some(session) = block.session.as_deref() {
            items.push(ConsoleRenderItem::Session(session));
        }

        if block.sudo_hint
            || (block.explain_hint && !block.is_explaining && block.explanation.is_none())
        {
            items.push(ConsoleRenderItem::Line(gutter_line(
                "--------------------------------------",
                Style::default().fg(Color::DarkGray),
            )));

            let mut hint_spans = vec![Span::styled("| ", Style::default().fg(Color::DarkGray))];
            if block.sudo_hint {
                hint_spans.push(Span::styled(
                    " [Ctrl+S: Re-run with sudo] ",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            if block.explain_hint && !block.is_explaining && block.explanation.is_none() {
                hint_spans.push(Span::styled(
                    " [Ctrl+E: Explain Error with AI] ",
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            items.push(ConsoleRenderItem::Line(Line::from(hint_spans)));
        }

        if block.is_explaining {
            items.push(ConsoleRenderItem::Line(gutter_line("", Style::default())));
            items.push(ConsoleRenderItem::Line(gutter_line(
                "AI is analyzing the error...",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::RAPID_BLINK),
            )));
        } else if let Some(explanation) = &block.explanation {
            items.push(ConsoleRenderItem::Line(gutter_line("", Style::default())));
            items.push(ConsoleRenderItem::Line(gutter_line(
                "AI Explanation",
                Style::default().fg(Color::Magenta),
            )));
            for line in explanation.lines() {
                items.push(ConsoleRenderItem::Line(Line::from(vec![
                    Span::styled("| ", Style::default().fg(Color::DarkGray)),
                    Span::styled("> ", Style::default().fg(Color::Magenta)),
                    Span::raw(line.to_string()),
                ])));
            }
            items.push(ConsoleRenderItem::Line(gutter_line(
                "-----------------------------------",
                Style::default().fg(Color::Magenta),
            )));
        }

        items.push(ConsoleRenderItem::Line(Line::from("")));
    }
    items
}

fn block_header_line(
    block: &CommandBlock,
    console_state: &crate::app::console_state::ConsoleState,
) -> Line<'static> {
    let mut header_spans = vec![
        Span::styled("$ ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            block.input.clone(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    if let Some((badge_text, badge_color)) = get_block_badge(block, console_state) {
        header_spans.push(Span::raw("  "));
        header_spans.push(Span::styled(badge_text, Style::default().fg(badge_color)));
    }

    Line::from(header_spans)
}

fn output_line_line(output_line: &crate::app::console_state::OutputLine) -> Line<'static> {
    let mut spans = vec![Span::styled("| ", Style::default().fg(Color::DarkGray))];
    if let Some(output_spans) = &output_line.spans {
        spans.extend(
            output_spans
                .iter()
                .map(|span| Span::styled(span.text.clone(), Style::default().fg(span.color))),
        );
    } else {
        spans.push(Span::styled(
            output_line.text.clone(),
            Style::default().fg(output_line.stream.color()),
        ));
    }
    Line::from(spans)
}

fn gutter_line(text: impl Into<String>, style: Style) -> Line<'static> {
    Line::from(vec![
        Span::styled("| ", Style::default().fg(Color::DarkGray)),
        Span::styled(text.into(), style),
    ])
}

fn plot_item_height(plot: &ConsolePlotBlock, width: u16) -> u16 {
    if width < 46 || plot.series.is_empty() {
        return plot.fallback_lines.len().clamp(4, 14) as u16;
    }
    let chart_height = (plot.requested_height as u16).clamp(6, 16);
    chart_height.saturating_add(5)
}

fn session_item_height(width: u16) -> u16 {
    if width < 50 {
        12
    } else {
        18
    }
}

fn visual_item_height(visual: &ConsoleVisualBlock, width: u16) -> u16 {
    if width < 46 {
        return visual.fallback_lines.len().clamp(4, 14) as u16;
    }

    match &visual.kind {
        ConsoleVisualKind::TrigUnitCircle(_) => 21,
    }
}

fn render_visual_output(f: &mut Frame, visual: &ConsoleVisualBlock, area: Rect) {
    match &visual.kind {
        ConsoleVisualKind::TrigUnitCircle(circle) => {
            render_trig_unit_circle_output(f, visual, circle, area);
        }
    }
}

fn render_trig_unit_circle_output(
    f: &mut Frame,
    visual: &ConsoleVisualBlock,
    circle: &ConsoleTrigUnitCircleBlock,
    area: Rect,
) {
    if area.width < 46 || area.height < 12 {
        render_visual_fallback(f, visual, area);
        return;
    }

    let title = clamp_plain_text(&visual.title, area.width.saturating_sub(4) as usize);
    let block = UiBlock::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", title))
        .border_style(Style::default().fg(Color::LightMagenta))
        .style(Style::default().bg(Color::Rgb(12, 14, 16)));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 10 || inner.width < 34 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(6),
            Constraint::Length(2),
        ])
        .split(inner);

    let point_count = circle.solution_points.len() + circle.boundary_points.len();
    let info = vec![
        Line::from(vec![
            Span::styled(" expr ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                clamp_plain_text(&circle.expression, inner.width.saturating_sub(8) as usize),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled(" axis ", Style::default().fg(Color::DarkGray)),
            Span::styled("cos=x  ", Style::default().fg(Color::Cyan)),
            Span::styled("sin=y  ", Style::default().fg(Color::Cyan)),
            Span::styled(" relation ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(
                    "{}(x) {} {}",
                    circle.function, circle.relation, circle.value_label
                ),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(" points ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                point_count.to_string(),
                Style::default().fg(Color::LightMagenta),
            ),
        ]),
    ];
    f.render_widget(Paragraph::new(info), chunks[0]);

    let canvas = Canvas::default()
        .marker(symbols::Marker::Braille)
        .x_bounds([-1.18, 1.18])
        .y_bounds([-1.18, 1.18])
        .paint(|ctx| {
            ctx.draw(&Circle {
                x: 0.0,
                y: 0.0,
                radius: 1.0,
                color: Color::DarkGray,
            });
            ctx.draw(&CanvasLine {
                x1: -1.12,
                y1: 0.0,
                x2: 1.12,
                y2: 0.0,
                color: Color::Cyan,
            });
            ctx.draw(&CanvasLine {
                x1: 0.0,
                y1: -1.12,
                x2: 0.0,
                y2: 1.12,
                color: Color::Cyan,
            });
            if !circle.arc_points.is_empty() {
                ctx.draw(&Points {
                    coords: circle.arc_points.as_slice(),
                    color: Color::Green,
                });
            }
            if !circle.boundary_points.is_empty() {
                ctx.draw(&Points {
                    coords: circle.boundary_points.as_slice(),
                    color: Color::Yellow,
                });
            }
            if !circle.solution_points.is_empty() {
                ctx.draw(&Points {
                    coords: circle.solution_points.as_slice(),
                    color: Color::LightMagenta,
                });
            }
            ctx.print(
                1.02,
                -0.08,
                Span::styled("cos", Style::default().fg(Color::Cyan)),
            );
            ctx.print(
                0.04,
                1.04,
                Span::styled("sin", Style::default().fg(Color::Cyan)),
            );
        });
    f.render_widget(canvas, chunks[1]);

    let legend = vec![
        Line::from(vec![
            Span::styled(" arc ", Style::default().fg(Color::DarkGray)),
            Span::styled("solution range  ", Style::default().fg(Color::Green)),
            Span::styled("boundary  ", Style::default().fg(Color::Yellow)),
            Span::styled("exact point", Style::default().fg(Color::LightMagenta)),
        ]),
        Line::from(Span::styled(
            "unit circle | one period | exact markers",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    f.render_widget(Paragraph::new(legend), chunks[2]);
}

fn render_visual_fallback(f: &mut Frame, visual: &ConsoleVisualBlock, area: Rect) {
    let lines = visual
        .fallback_lines
        .iter()
        .take(area.height as usize)
        .map(output_line_content_line)
        .collect::<Vec<_>>();
    let fallback = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::Cyan));
    f.render_widget(fallback, area);
}

fn output_line_content_line(output_line: &crate::app::console_state::OutputLine) -> Line<'static> {
    if let Some(output_spans) = &output_line.spans {
        Line::from(
            output_spans
                .iter()
                .map(|span| Span::styled(span.text.clone(), Style::default().fg(span.color)))
                .collect::<Vec<_>>(),
        )
    } else {
        Line::from(Span::styled(
            output_line.text.clone(),
            Style::default().fg(output_line.stream.color()),
        ))
    }
}

fn render_plot_output(f: &mut Frame, plot: &ConsolePlotBlock, area: Rect) {
    if area.width < 46 || area.height < 8 || plot.series.is_empty() {
        render_plot_fallback(f, plot, area);
        return;
    }

    let title = clamp_plain_text(&plot.title, area.width.saturating_sub(4) as usize);
    let block = UiBlock::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", title))
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Rgb(12, 14, 16)));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 5 || inner.width < 24 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3)])
        .split(inner);

    let cache = if plot.cache_hit { "hit" } else { "miss" };
    let info = vec![
        Line::from(vec![
            Span::styled(" mode ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:<9}", plot.mode.label()),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(" var ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{:<8}", plot.variable),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(" samples ", Style::default().fg(Color::DarkGray)),
            Span::styled(plot.samples.to_string(), Style::default().fg(Color::White)),
            Span::styled(" cache ", Style::default().fg(Color::DarkGray)),
            Span::styled(cache, Style::default().fg(Color::Green)),
            Span::styled(" x ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("{}..{}", plot.x_min_label, plot.x_max_label),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(" y ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("{}..{}", plot.y_min_label, plot.y_max_label),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(vec![
            Span::styled(" finite ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                plot.finite_samples.to_string(),
                Style::default().fg(Color::Green),
            ),
            Span::styled(" invalid ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                plot.invalid_samples.to_string(),
                Style::default().fg(if plot.invalid_samples > 0 {
                    Color::Yellow
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(" clipped ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                plot.clipped_samples.to_string(),
                Style::default().fg(if plot.clipped_samples > 0 {
                    Color::Yellow
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(" breaks ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                plot.discontinuities.to_string(),
                Style::default().fg(if plot.discontinuities > 0 {
                    Color::Yellow
                } else {
                    Color::DarkGray
                }),
            ),
        ]),
    ];
    f.render_widget(Paragraph::new(info), chunks[0]);

    if plot.mode == ConsolePlotMode::Sparkline {
        render_plot_sparkline(f, plot, chunks[1]);
        return;
    }

    let graph_type = match plot.mode {
        ConsolePlotMode::Bars => GraphType::Bar,
        ConsolePlotMode::Points => GraphType::Scatter,
        ConsolePlotMode::Line | ConsolePlotMode::Sparkline => GraphType::Line,
    };
    let marker = match plot.mode {
        ConsolePlotMode::Bars => symbols::Marker::Bar,
        ConsolePlotMode::Points => symbols::Marker::Dot,
        ConsolePlotMode::Line | ConsolePlotMode::Sparkline => symbols::Marker::Braille,
    };
    let style = match plot.mode {
        ConsolePlotMode::Bars => Style::default().fg(Color::Green),
        ConsolePlotMode::Points => Style::default().fg(Color::Yellow),
        ConsolePlotMode::Line | ConsolePlotMode::Sparkline => Style::default().fg(Color::Cyan),
    };
    let datasets = plot
        .series
        .iter()
        .enumerate()
        .filter(|(_, series)| !series.points.is_empty())
        .map(|(idx, series)| {
            let mut dataset = Dataset::default()
                .marker(marker)
                .graph_type(graph_type)
                .style(style)
                .data(series.points.as_slice());
            if idx == 0 {
                dataset = dataset.name(clamp_plain_text(
                    &plot.expression,
                    chunks[1].width.saturating_sub(4) as usize,
                ));
            }
            dataset
        })
        .collect::<Vec<_>>();

    let x_labels = axis_labels(&plot.x_min_label, &plot.x_max_label, plot.x_min, plot.x_max);
    let y_labels = axis_labels(&plot.y_min_label, &plot.y_max_label, plot.y_min, plot.y_max);
    let chart = Chart::new(datasets)
        .x_axis(
            Axis::default()
                .title(Line::from(Span::styled(
                    "x",
                    Style::default().fg(Color::Cyan),
                )))
                .style(Style::default().fg(Color::DarkGray))
                .bounds([plot.x_min, plot.x_max])
                .labels(x_labels),
        )
        .y_axis(
            Axis::default()
                .title(Line::from(Span::styled(
                    "y",
                    Style::default().fg(Color::Cyan),
                )))
                .style(Style::default().fg(Color::DarkGray))
                .bounds([plot.y_min, plot.y_max])
                .labels(y_labels),
        );

    f.render_widget(chart, chunks[1]);
}

fn render_plot_sparkline(f: &mut Frame, plot: &ConsolePlotBlock, area: Rect) {
    if area.height >= 3 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(area);
        render_plot_sparkline_values(f, plot, chunks[0]);
        let labels = Line::from(vec![
            Span::styled(" x ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("{}..{}", plot.x_min_label, plot.x_max_label),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled("   y ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("{}..{}", plot.y_min_label, plot.y_max_label),
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        f.render_widget(Paragraph::new(labels), chunks[1]);
        return;
    }

    render_plot_sparkline_values(f, plot, area);
}

fn render_plot_sparkline_values(f: &mut Frame, plot: &ConsolePlotBlock, area: Rect) {
    let values = plot
        .series
        .iter()
        .flat_map(|series| series.points.iter().map(|(_, y)| *y))
        .map(|y| ((y - plot.y_min) / (plot.y_max - plot.y_min)).clamp(0.0, 1.0))
        .map(|t| (t * 100.0).round() as u64)
        .collect::<Vec<_>>();
    let max_value = values.iter().copied().max().unwrap_or(1).max(1);
    let sparkline = Sparkline::default()
        .data(&values)
        .style(Style::default().fg(Color::Cyan))
        .max(max_value);
    f.render_widget(sparkline, area);
}

fn render_plot_fallback(f: &mut Frame, plot: &ConsolePlotBlock, area: Rect) {
    let lines = plot
        .fallback_lines
        .iter()
        .take(area.height as usize)
        .map(|line| Line::from(Span::styled(line.clone(), Style::default().fg(Color::Cyan))))
        .collect::<Vec<_>>();
    let fallback = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .style(Style::default().fg(Color::Cyan));
    f.render_widget(fallback, area);
}

fn axis_labels(min_label: &str, max_label: &str, min: f64, max: f64) -> Vec<Line<'static>> {
    if min < 0.0 && max > 0.0 {
        vec![
            Line::from(min_label.to_string()),
            Line::from("0"),
            Line::from(max_label.to_string()),
        ]
    } else {
        vec![
            Line::from(min_label.to_string()),
            Line::from(max_label.to_string()),
        ]
    }
}

fn clamp_plain_text(input: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }

    let mut out = String::new();
    let mut width = 0usize;
    for ch in input.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > max_width {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out
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
            let badge = format!("[run {}s]", secs);
            Some((badge, Color::Yellow))
        }
        Some(task_state) => Some((task_state.badge(), task_state.badge_color())),
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
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
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
            Constraint::Min(1),    // Results list
        ])
        .split(inner);

    // Search query line
    let query_line = Line::from(vec![
        Span::styled(" > ", Style::default().fg(Color::Yellow)),
        Span::styled(
            &state.console_state.history_search_query,
            Style::default().fg(Color::White),
        ),
        Span::styled("_", Style::default().fg(Color::DarkGray)), // cursor
    ]);

    f.render_widget(Paragraph::new(query_line), search_chunks[0]);

    // Separator
    let sep = "-".repeat(search_chunks[1].width as usize);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            sep,
            Style::default().fg(Color::DarkGray),
        ))),
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
            let indicator = if is_selected { " > " } else { "   " };
            let style = if is_selected {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Rgb(50, 50, 70))
                    .add_modifier(Modifier::BOLD)
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
    let cmd = state
        .console_state
        .confirm_command
        .as_deref()
        .unwrap_or("???");
    let action = state
        .console_state
        .confirm_action
        .as_deref()
        .unwrap_or("Confirm action");

    let panel_width = (area.width.saturating_sub(10)).min(60);
    let panel_height = 7;
    let panel_area = centered_rect(panel_width, panel_height, area);

    // Clear background
    f.render_widget(Clear, panel_area);

    let panel_block = UiBlock::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(format!(" {} ", action))
        .title_alignment(Alignment::Center)
        .style(Style::default().bg(Color::Rgb(25, 25, 35)));

    let inner = panel_block.inner(panel_area);
    f.render_widget(panel_block, panel_area);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Command: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                cmd,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                "[Enter]",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Execute   ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "[Esc]",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
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

    let mut status_spans = vec![Span::styled(
        mode_str,
        Style::default().bg(mode_color).fg(Color::Black),
    )];

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
            format!(" [SCROLL +{}] ", state.console_state.scroll_offset),
            Style::default().fg(Color::Cyan),
        ));
    }

    // Help text
    let help = match state.console_state.mode {
        ConsoleMode::Normal => " | 'i' insert  Up/Down scroll  Ctrl+S sudo",
        ConsoleMode::Insert => " | Esc normal  Ctrl+R history  Tab/Right accept  Up/Down prev/next",
        ConsoleMode::HistorySearch => " | Esc cancel  Enter accept  Up/Down navigate",
        ConsoleMode::Confirm => " | Enter confirm  Esc cancel",
    };

    status_spans.push(Span::styled(help, Style::default().fg(Color::DarkGray)));

    let status_line = Line::from(status_spans);
    let status_paragraph =
        Paragraph::new(status_line).style(Style::default().bg(Color::Rgb(30, 30, 40)));

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
    let prompt_width = UnicodeWidthStr::width(prompt);
    let ghost_remainder = state.console_state.ghost_text.as_ref().and_then(|ghost| {
        let input_char_count = input.chars().count();
        let ghost_char_count = ghost.chars().count();
        if ghost_char_count > input_char_count && ghost.starts_with(input.as_str()) {
            Some(ghost.chars().skip(input_char_count).collect::<String>())
        } else {
            None
        }
    });
    let total_input_width = prompt_width
        + UnicodeWidthStr::width(input.as_str())
        + ghost_remainder
            .as_deref()
            .map(UnicodeWidthStr::width)
            .unwrap_or(0);
    let should_clip = inner.width > 0 && total_input_width > inner.width as usize;

    let mut spans = vec![Span::styled(prompt, Style::default().fg(Color::Cyan))];

    let cursor_visual_width;
    if should_clip {
        let available_width = (inner.width as usize).saturating_sub(prompt_width).max(1);
        let (visible_input, visible_cursor_width) =
            visible_input_window(input, state.console_state.cursor_position, available_width);
        spans.push(Span::styled(
            visible_input,
            Style::default().fg(Color::White),
        ));
        cursor_visual_width = visible_cursor_width;
    } else if !state.console_state.highlighted_input.is_empty() {
        // Use syntax-highlighted tokens if available, otherwise plain white
        for (text, color) in &state.console_state.highlighted_input {
            spans.push(Span::styled(text.as_str(), Style::default().fg(*color)));
        }
        let text_before_cursor: String = input
            .chars()
            .take(state.console_state.cursor_position)
            .collect();
        cursor_visual_width = UnicodeWidthStr::width(text_before_cursor.as_str());
    } else {
        spans.push(Span::styled(
            input.as_str(),
            Style::default().fg(Color::White),
        ));
        let text_before_cursor: String = input
            .chars()
            .take(state.console_state.cursor_position)
            .collect();
        cursor_visual_width = UnicodeWidthStr::width(text_before_cursor.as_str());
    }

    // Ghost text: show the remainder of the suggestion in dark gray
    if !should_clip {
        if let Some(remainder) = ghost_remainder {
            spans.push(Span::styled(
                remainder,
                Style::default().fg(Color::Rgb(80, 80, 100)),
            ));
        };
    }

    let input_paragraph = Paragraph::new(Line::from(spans)).block(input_block);
    f.render_widget(input_paragraph, area);

    // Render cursor if in insert mode
    if state.console_state.mode == ConsoleMode::Insert {
        let cursor_x =
            inner.x + prompt_width as u16 + cursor_visual_width.min(inner.width as usize) as u16;
        let cursor_y = inner.y;

        if cursor_x < inner.x + inner.width {
            f.set_cursor_position((cursor_x, cursor_y));
        }
    }
}

fn visible_input_window(input: &str, cursor_position: usize, max_width: usize) -> (String, usize) {
    let max_width = max_width.max(1);
    let chars: Vec<char> = input.chars().collect();
    let cursor = cursor_position.min(chars.len());

    let mut start = 0;
    while start < cursor {
        let before_cursor: String = chars[start..cursor].iter().collect();
        if UnicodeWidthStr::width(before_cursor.as_str()) < max_width {
            break;
        }
        start += 1;
    }

    let before_cursor: String = chars[start..cursor].iter().collect();
    let cursor_width = UnicodeWidthStr::width(before_cursor.as_str());

    let mut visible = String::new();
    let mut visible_width = 0usize;
    for ch in chars.iter().skip(start).copied() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if visible_width + ch_width > max_width {
            break;
        }
        visible.push(ch);
        visible_width += ch_width;
    }

    (visible, cursor_width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::console_state::{
        ConsolePlotSeries, ConsoleTrigUnitCircleBlock, ConsoleVisualBlock, ConsoleVisualKind,
        OutputLine,
    };
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn visible_input_window_keeps_cursor_visible_for_long_ascii() {
        let (visible, cursor_width) = visible_input_window("abcdef", 6, 3);
        assert_eq!(visible, "ef");
        assert_eq!(cursor_width, 2);
    }

    #[test]
    fn visible_input_window_counts_wide_characters() {
        let (visible, cursor_width) = visible_input_window("ab界cd", 4, 4);
        assert_eq!(visible, "界cd");
        assert_eq!(cursor_width, 3);
    }

    #[test]
    fn plot_output_renders_with_ratatui_chart_block() {
        let backend = TestBackend::new(80, 18);
        let mut terminal = Terminal::new(backend).expect("test backend");
        let plot = sample_plot();

        terminal
            .draw(|frame| render_plot_output(frame, &plot, Rect::new(0, 0, 80, 18)))
            .expect("render plot");

        let text = buffer_text(terminal.backend().buffer(), 80, 18);
        assert!(text.contains("PLOT / function"));
        assert!(text.contains("samples"));
        assert!(text.contains("sin(x)"));
        assert!(text.contains(" y"));
    }

    #[test]
    fn plot_output_uses_fallback_when_area_is_too_narrow() {
        let backend = TestBackend::new(32, 8);
        let mut terminal = Terminal::new(backend).expect("test backend");
        let plot = sample_plot();

        terminal
            .draw(|frame| render_plot_output(frame, &plot, Rect::new(0, 0, 32, 8)))
            .expect("render plot fallback");

        let text = buffer_text(terminal.backend().buffer(), 32, 8);
        assert!(text.contains("fallback plot"));
    }

    #[test]
    fn trig_unit_circle_visual_renders_with_ratatui_canvas_block() {
        let backend = TestBackend::new(80, 22);
        let mut terminal = Terminal::new(backend).expect("test backend");
        let visual = sample_trig_visual();

        terminal
            .draw(|frame| render_visual_output(frame, &visual, Rect::new(0, 0, 80, 22)))
            .expect("render trig visual");

        let text = buffer_text(terminal.backend().buffer(), 80, 22);
        assert!(text.contains("TRIG UNIT CIRCLE"));
        assert!(text.contains("cos=x"));
        assert!(text.contains("sin=y"));
        assert!(text.contains("exact point"));
    }

    #[test]
    fn trig_unit_circle_visual_uses_fallback_when_area_is_too_narrow() {
        let backend = TestBackend::new(32, 8);
        let mut terminal = Terminal::new(backend).expect("test backend");
        let visual = sample_trig_visual();

        terminal
            .draw(|frame| render_visual_output(frame, &visual, Rect::new(0, 0, 32, 8)))
            .expect("render trig visual fallback");

        let text = buffer_text(terminal.backend().buffer(), 32, 8);
        assert!(text.contains("fallback visual"));
    }

    fn sample_plot() -> ConsolePlotBlock {
        ConsolePlotBlock {
            title: "PLOT / function".to_string(),
            expression: "sin(x)".to_string(),
            variable: "x".to_string(),
            mode: ConsolePlotMode::Line,
            x_min: -std::f64::consts::PI,
            x_max: std::f64::consts::PI,
            y_min: -1.0,
            y_max: 1.0,
            x_min_label: "-pi".to_string(),
            x_max_label: "pi".to_string(),
            y_min_label: "-1".to_string(),
            y_max_label: "1".to_string(),
            samples: 64,
            finite_samples: 64,
            invalid_samples: 0,
            clipped_samples: 0,
            discontinuities: 0,
            cache_hit: false,
            requested_width: 72,
            requested_height: 10,
            series: vec![ConsolePlotSeries {
                points: (0..64)
                    .map(|idx| {
                        let t = idx as f64 / 63.0;
                        let x = -std::f64::consts::PI + std::f64::consts::TAU * t;
                        (x, x.sin())
                    })
                    .collect(),
            }],
            fallback_lines: vec!["fallback plot".to_string()],
        }
    }

    fn sample_trig_visual() -> ConsoleVisualBlock {
        ConsoleVisualBlock {
            title: "TRIG UNIT CIRCLE".to_string(),
            kind: ConsoleVisualKind::TrigUnitCircle(ConsoleTrigUnitCircleBlock {
                expression: "sin(x) = 1".to_string(),
                function: "sin".to_string(),
                relation: "=".to_string(),
                value_label: "1".to_string(),
                solution_points: vec![(0.0, 1.0)],
                boundary_points: Vec::new(),
                arc_points: Vec::new(),
            }),
            fallback_lines: vec![OutputLine::stdout("fallback visual")],
        }
    }

    fn buffer_text(buffer: &ratatui::buffer::Buffer, width: u16, height: u16) -> String {
        let mut out = String::new();
        for y in 0..height {
            for x in 0..width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }
}
