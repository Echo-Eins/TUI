use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use crate::app::App;
use crate::ui::theme::Theme;
use crate::utils::format::{create_progress_bar, format_bytes};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let analyzer_data = app.state.disk_analyzer_data.read();
    let analyzer_error = app.state.disk_analyzer_error.read();

    if let Some(message) = analyzer_error.as_ref() {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);
        let block = Block::default()
            .title("Disk Analyzer")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.warning_color));

        let text = Paragraph::new(format!("Disk analyzer unavailable: {}", message))
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(text, area);
    } else if let Some(data) = analyzer_data.as_ref() {
        let config = app.state.config.read();
        let theme = Theme::from_config(&config);

        if data.drives.is_empty() {
            let block = Block::default()
                .title("Disk Analyzer")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.disk_color));

            let text = Paragraph::new("No fixed drives found")
                .block(block)
                .style(Style::default().fg(Color::Gray));

            f.render_widget(text, area);
            return;
        }

        render_drives(f, area, data, &theme);
    } else {
        let block = Block::default()
            .title("Disk Analyzer")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red));

        let text = Paragraph::new("Loading disk analyzer data...")
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(text, area);
    }
}

fn render_drives(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::DiskAnalyzerData,
    theme: &Theme,
) {
    let drive_count = data.drives.len().max(1);
    let constraints: Vec<Constraint> = (0..drive_count)
        .map(|_| Constraint::Ratio(1, drive_count as u32))
        .collect();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    for (i, drive) in data.drives.iter().enumerate() {
        if let Some(chunk) = chunks.get(i) {
            render_drive_panel(f, *chunk, drive, theme);
        }
    }
}

fn render_drive_panel(
    f: &mut Frame,
    area: Rect,
    drive: &crate::monitors::AnalyzedDrive,
    theme: &Theme,
) {
    let system_drive = system_drive_letter();
    let is_system = system_drive
        .as_ref()
        .map(|letter| drive.letter.eq_ignore_ascii_case(letter))
        .unwrap_or(false);
    let label = if is_system {
        format!("{} (System)", drive.letter)
    } else {
        drive.letter.clone()
    };
    let title = if drive.name.is_empty() {
        format!("Drive {}", label)
    } else {
        format!("Drive {} ({})", label, drive.name)
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.disk_color));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    if let Some(err) = drive.error.as_ref() {
        let text = Paragraph::new(format!("Everything error: {}", err))
            .style(Style::default().fg(theme.warning_color));
        f.render_widget(text, inner);
        return;
    }

    let used_pct = if drive.total > 0 {
        (drive.used as f64 / drive.total as f64 * 100.0).min(100.0)
    } else {
        0.0
    };

    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::raw("Used "),
        Span::styled(
            format_bytes(drive.used),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" / "),
        Span::raw(format_bytes(drive.total)),
        Span::raw(format!(" ({:.0}%)  ", used_pct)),
        Span::raw("Free "),
        Span::styled(format_bytes(drive.free), Style::default().fg(Color::Green)),
    ]));

    if inner.height > 1 {
        lines.push(Line::from(vec![Span::styled(
            "Root folders (share of used space)",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]));
    }

    let remaining_rows = inner.height.saturating_sub(lines.len() as u16);
    if remaining_rows == 0 {
        let text = Paragraph::new(lines).style(Style::default().fg(Color::White));
        f.render_widget(text, inner);
        return;
    }

    if drive.root_folders.is_empty() {
        lines.push(Line::from("No root folder data"));
        let text = Paragraph::new(lines).style(Style::default().fg(Color::Gray));
        f.render_widget(text, inner);
        return;
    }

    let max_rows = remaining_rows as usize;
    let size_samples: Vec<String> = drive
        .root_folders
        .iter()
        .take(max_rows)
        .map(|entry| format_bytes(entry.size))
        .collect();
    let size_width = size_samples
        .iter()
        .map(|s| s.len())
        .max()
        .unwrap_or(8)
        .min(inner.width as usize);

    let percent_width = 4usize;
    let inner_width = inner.width.saturating_sub(1) as usize;
    let available = inner_width.saturating_sub(size_width + percent_width + 6);
    let (name_width, bar_width) = compute_column_widths(available);

    let denom = if drive.used > 0 { drive.used } else { drive.total };

    for (entry, size_str) in drive
        .root_folders
        .iter()
        .zip(size_samples.iter())
        .take(max_rows)
    {
        let pct = if denom > 0 {
            (entry.size as f64 / denom as f64 * 100.0).min(100.0)
        } else {
            0.0
        };

        let name = truncate_label(&entry.name, name_width);
        if bar_width > 0 {
            let bar = create_progress_bar(pct as f32, bar_width);
            lines.push(Line::from(format!(
                "{:<name_width$}  [{}] {:>percent_width$}% {:>size_width$}",
                name,
                bar,
                pct.round() as u16,
                size_str,
                name_width = name_width,
                percent_width = percent_width,
                size_width = size_width
            )));
        } else {
            lines.push(Line::from(format!(
                "{:<name_width$}  {:>percent_width$}% {:>size_width$}",
                name,
                pct.round() as u16,
                size_str,
                name_width = name_width,
                percent_width = percent_width,
                size_width = size_width
            )));
        }
    }

    let text = Paragraph::new(lines).style(Style::default().fg(Color::White));
    f.render_widget(text, inner);
}

fn system_drive_letter() -> Option<String> {
    let drive = std::env::var("SystemDrive").ok()?;
    let trimmed = drive.trim().trim_end_matches('\\');
    let normalized = if trimmed.ends_with(':') {
        trimmed.to_string()
    } else {
        format!("{}:", trimmed)
    };
    Some(normalized.to_uppercase())
}

fn compute_column_widths(available: usize) -> (usize, usize) {
    if available < 8 {
        return (available.max(1), 0);
    }

    let mut bar_width = available.min(24);
    let mut name_width = available.saturating_sub(bar_width);

    if name_width < 8 {
        let needed = 8 - name_width;
        if bar_width > needed + 4 {
            bar_width = bar_width.saturating_sub(needed);
            name_width = available.saturating_sub(bar_width);
        }
    }

    if bar_width < 4 {
        (available, 0)
    } else {
        (name_width.max(8), bar_width)
    }
}

fn truncate_label(label: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let mut chars = label.chars().collect::<Vec<char>>();
    if chars.len() <= width {
        return format!("{:<width$}", label, width = width);
    }

    if width <= 3 {
        return chars.into_iter().take(width).collect();
    }

    let mut truncated: String = chars.drain(..width - 3).collect();
    truncated.push_str("...");
    format!("{:<width$}", truncated, width = width)
}
