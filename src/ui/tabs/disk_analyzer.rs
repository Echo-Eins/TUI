use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Gauge},
    Frame,
};
use unicode_width::UnicodeWidthStr;
use crate::app::App;
use crate::app::state::{TreeNode, DiskAnalyzerSortColumn};
use crate::ui::theme::Theme;
use crate::utils::format::format_bytes;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let analyzer_data = app.state.disk_analyzer_data.read();
    let analyzer_error = app.state.disk_analyzer_error.read();
    let config = app.state.config.read();
    let theme = Theme::from_config(&config);

    if let Some(message) = analyzer_error.as_ref() {
        let block = Block::default()
            .title("Disk Analyzer")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.warning_color));

        let text = Paragraph::new(format!("Disk analyzer unavailable: {}", message))
            .block(block)
            .style(Style::default().fg(Color::White));

        f.render_widget(text, area);
        return;
    }

    let data = match analyzer_data.as_ref() {
        Some(data) if !data.drives.is_empty() => data,
        Some(_) => {
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
        None => {
            let block = Block::default()
                .title("Disk Analyzer")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red));

            let text = Paragraph::new("Loading disk analyzer data...")
                .block(block)
                .style(Style::default().fg(Color::White));

            f.render_widget(text, area);
            return;
        }
    };

    // Main layout: header with tabs, content, footer
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Drive tabs
            Constraint::Length(3),  // Usage bar
            Constraint::Min(5),     // Tree content
            Constraint::Length(2),  // Footer
        ])
        .split(area);

    let selected_drive = app.state.disk_analyzer_state.selected_drive.min(data.drives.len().saturating_sub(1));

    // Render drive tabs
    render_drive_tabs(f, main_chunks[0], data, selected_drive, &theme);

    // Render selected drive content
    if let Some(drive) = data.drives.get(selected_drive) {
        render_usage_bar(f, main_chunks[1], drive, &theme);
        render_tree_content(f, main_chunks[2], app, drive, &theme);
    }

    // Render footer
    render_footer(f, main_chunks[3], app, &theme);
}

fn render_drive_tabs(
    f: &mut Frame,
    area: Rect,
    data: &crate::monitors::DiskAnalyzerData,
    selected: usize,
    theme: &Theme,
) {
    let system_drive = system_drive_letter();

    let mut spans = vec![Span::raw(" ")];

    for (i, drive) in data.drives.iter().enumerate() {
        let is_selected = i == selected;
        let is_system = system_drive
            .as_ref()
            .map(|letter| drive.letter.eq_ignore_ascii_case(letter))
            .unwrap_or(false);

        let label = if is_system {
            format!("{} System", drive.letter)
        } else if !drive.name.is_empty() {
            format!("{} {}", drive.letter, drive.name)
        } else {
            drive.letter.clone()
        };

        if is_selected {
            // Selected: yellow color with round brackets
            if i == 0 {
                spans.push(Span::styled("< ", Style::default().fg(Color::Yellow)));
            }
            spans.push(Span::styled("(", Style::default().fg(Color::Yellow)));
            spans.push(Span::styled(label.clone(), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)));
            spans.push(Span::styled(")", Style::default().fg(Color::Yellow)));
            if i == data.drives.len() - 1 {
                spans.push(Span::styled(" >", Style::default().fg(Color::Yellow)));
            }
        } else {
            // Not selected: normal color
            spans.push(Span::raw(" "));
            spans.push(Span::styled(label.clone(), Style::default().fg(Color::DarkGray)));
            spans.push(Span::raw(" "));
        }

        if i < data.drives.len() - 1 {
            spans.push(Span::raw("   "));
        }
    }

    let block = Block::default()
        .title("Disk Analyzer")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.disk_color));

    let paragraph = Paragraph::new(Line::from(spans)).block(block);
    f.render_widget(paragraph, area);
}

fn render_usage_bar(
    f: &mut Frame,
    area: Rect,
    drive: &crate::monitors::AnalyzedDrive,
    theme: &Theme,
) {
    let used_pct = if drive.total > 0 {
        (drive.used as f64 / drive.total as f64 * 100.0).min(100.0)
    } else {
        0.0
    };

    let label = format!(
        "Used {} / {} ({:.0}%)  Free {}",
        format_bytes(drive.used),
        format_bytes(drive.total),
        used_pct,
        format_bytes(drive.free)
    );

    let gauge_color = if used_pct > 90.0 {
        Color::Red
    } else if used_pct > 75.0 {
        Color::Yellow
    } else {
        theme.disk_color
    };

    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(theme.disk_color)))
        .gauge_style(Style::default().fg(gauge_color).bg(Color::Black))
        .percent(used_pct as u16)
        .label(label);

    f.render_widget(gauge, area);
}

fn render_tree_content(
    f: &mut Frame,
    area: Rect,
    app: &App,
    drive: &crate::monitors::AnalyzedDrive,
    theme: &Theme,
) {
    let block = Block::default()
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

    let drive_letter = &drive.letter;
    let state = &app.state.disk_analyzer_state;
    let horizontal_offset = state.horizontal_offset;
    let show_files = state.show_files;

    // Get tree nodes or create from root folders
    let all_nodes: Vec<TreeNode> = if let Some(tree) = state.trees.get(drive_letter) {
        tree.clone()
    } else {
        drive.root_folders.iter().map(TreeNode::from_root_folder).collect()
    };

    // Apply show_files filter
    let tree_nodes: Vec<TreeNode> = if show_files {
        all_nodes
    } else {
        all_nodes.into_iter().filter(|n| !n.is_file).collect()
    };

    if tree_nodes.is_empty() {
        let text = Paragraph::new("No items found")
            .style(Style::default().fg(Color::Gray));
        f.render_widget(text, inner);
        return;
    }

    let visible_height = inner.height as usize;
    let selected_idx = state.selected_index.min(tree_nodes.len().saturating_sub(1));
    let scroll_offset = state.scroll_offset;
    let extended_view = state.extended_view;

    // Get extension config
    let config = app.state.config.read();
    let show_extensions = &config.integrations.disk_analyzer.show_extensions;

    // Check if path was recently copied
    let path_copied_info = state.path_copied_at.as_ref().and_then(|(path, time)| {
        if time.elapsed().as_secs() < 1 {
            Some(path.clone())
        } else {
            None
        }
    });

    let mut lines = Vec::new();

    for (i, node) in tree_nodes.iter().enumerate().skip(scroll_offset).take(visible_height) {
        let is_selected = i == selected_idx;
        // Extended view shows info for ALL folders when enabled
        let show_extended_for_node = extended_view;

        // Build tree prefix
        let indent = "   ".repeat(node.depth);
        let tree_prefix = if node.depth > 0 {
            format!("{}|-- ", indent)
        } else {
            String::new()
        };

        // Icon and color based on type (file or folder)
        let (icon, icon_color) = if node.is_file {
            // File icon - no expansion indicator
            ("  ", Color::Gray)
        } else if node.loading {
            ("...", Color::Cyan)
        } else if node.expanded {
            ("v ", Color::Cyan)
        } else if node.has_children {
            ("> ", Color::Cyan)
        } else {
            ("  ", Color::Cyan)
        };

        // Build folder/file info string (extended view only shows for siblings at same depth)
        let item_info = if !node.is_file && (is_selected || show_extended_for_node) {
            build_folder_info(node, show_extensions)
        } else if node.is_file {
            // For files show extension if available
            node.extension.clone().unwrap_or_default()
        } else {
            String::new()
        };

        // Check if this path was just copied (compare trimmed paths)
        let show_copied = path_copied_info.as_ref()
            .map(|p| p == node.path.trim_end_matches('\\'))
            .unwrap_or(false) && is_selected;

        // Build the line
        let mut spans = Vec::new();

        // Selection arrow (yellow if selected)
        if is_selected {
            spans.push(Span::styled(" -> ", Style::default().fg(Color::Yellow)));
        } else {
            spans.push(Span::raw("    "));
        }

        // Apply horizontal scroll to content
        let displayed_prefix = if horizontal_offset < tree_prefix.len() {
            tree_prefix[horizontal_offset..].to_string()
        } else {
            String::new()
        };

        // Tree structure prefix
        spans.push(Span::styled(displayed_prefix, Style::default().fg(Color::DarkGray)));

        // Icon
        spans.push(Span::styled(icon, Style::default().fg(icon_color)));

        // Name (yellow if selected, gray for files when not selected)
        let name_style = if is_selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else if node.is_file {
            Style::default().fg(Color::Gray)
        } else {
            Style::default().fg(Color::White)
        };
        spans.push(Span::styled(node.name.clone(), name_style));

        // Item info (files/folders count for folders, extension for files)
        if !item_info.is_empty() {
            spans.push(Span::styled(
                format!(" {}", item_info),
                Style::default().fg(Color::DarkGray),
            ));
        }

        // "Path Copied!" indicator
        if show_copied {
            spans.push(Span::styled(" Path Copied!", Style::default().fg(Color::Green)));
        }

        // Size (right-aligned) - calculate remaining space using unicode width
        let size_str = format_bytes(node.size);
        let current_len: usize = spans.iter().map(|s| s.content.width()).sum();
        let available_width = inner.width as usize;

        if current_len + size_str.len() + 2 < available_width {
            let padding = available_width.saturating_sub(current_len + size_str.len() + 1);
            spans.push(Span::raw(" ".repeat(padding)));
            spans.push(Span::styled(size_str, Style::default().fg(Color::Cyan)));
        }

        lines.push(Line::from(spans));
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}

fn build_folder_info(node: &TreeNode, show_extensions: &[String]) -> String {
    let mut parts = Vec::new();

    if let Some(file_count) = node.file_count {
        let word = if file_count == 1 { "file" } else { "files" };
        parts.push(format!("{} {}", file_count, word));
    }

    if let Some(folder_count) = node.folder_count {
        let word = if folder_count == 1 { "folder" } else { "folders" };
        parts.push(format!("{} {}", folder_count, word));
    }

    if let Some(ext_counts) = &node.extension_counts {
        for ext in show_extensions {
            if let Some(count) = ext_counts.get(ext) {
                if *count > 0 {
                    parts.push(format!("{} {}", count, ext));
                }
            }
        }
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!("({})", parts.join(", "))
    }
}

fn render_footer(
    f: &mut Frame,
    area: Rect,
    app: &App,
    theme: &Theme,
) {
    let state = &app.state.disk_analyzer_state;
    let any_expanded = app.state.has_any_expanded_folder();
    let in_tree_mode = state.in_tree_mode;

    let mut spans = vec![Span::raw(" ")];

    // Navigation keys
    spans.push(Span::styled("[", Style::default().fg(Color::DarkGray)));
    spans.push(Span::styled("Up/Dn", Style::default().fg(Color::Yellow)));
    spans.push(Span::styled("]", Style::default().fg(Color::DarkGray)));
    spans.push(Span::raw(" Nav "));

    // Left/Right behavior
    spans.push(Span::styled("[", Style::default().fg(Color::DarkGray)));
    spans.push(Span::styled("Lt/Rt", Style::default().fg(Color::Yellow)));
    spans.push(Span::styled("]", Style::default().fg(Color::DarkGray)));
    if any_expanded && in_tree_mode {
        spans.push(Span::raw(" Scroll "));
    } else {
        spans.push(Span::raw(" Disk "));
    }

    // Enter
    spans.push(Span::styled("[", Style::default().fg(Color::DarkGray)));
    spans.push(Span::styled("Enter", Style::default().fg(Color::Yellow)));
    spans.push(Span::styled("]", Style::default().fg(Color::DarkGray)));
    spans.push(Span::raw(" Expand "));

    // Esc
    if in_tree_mode {
        spans.push(Span::styled("[", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled("Esc", Style::default().fg(Color::Yellow)));
        spans.push(Span::styled("]", Style::default().fg(Color::DarkGray)));
        spans.push(Span::raw(" Exit "));
    }

    // Separator
    spans.push(Span::styled(" | ", Style::default().fg(Color::DarkGray)));

    // Sort indicator - n,s,t keys like Services tab
    let sort_dir = if state.sort_ascending { "^" } else { "v" };

    // Show which key is active (highlighted) and which column is selected
    let n_style = if state.sort_column == DiskAnalyzerSortColumn::Name {
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Yellow)
    };
    let s_style = if state.sort_column == DiskAnalyzerSortColumn::Size {
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Yellow)
    };
    let t_style = if state.sort_column == DiskAnalyzerSortColumn::Type {
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Yellow)
    };

    spans.push(Span::styled("n", n_style));
    spans.push(Span::styled(",", Style::default().fg(Color::DarkGray)));
    spans.push(Span::styled("s", s_style));
    spans.push(Span::styled(",", Style::default().fg(Color::DarkGray)));
    spans.push(Span::styled("t", t_style));
    spans.push(Span::raw(" Sort"));

    // Show direction indicator for the active sort column
    let sort_name = match state.sort_column {
        DiskAnalyzerSortColumn::Name => "Name",
        DiskAnalyzerSortColumn::Size => "Size",
        DiskAnalyzerSortColumn::Type => "Type",
    };
    spans.push(Span::styled(format!(":{}{} ", sort_name, sort_dir), Style::default().fg(Color::DarkGray)));

    // Extended view indicator
    spans.push(Span::styled("[", Style::default().fg(Color::DarkGray)));
    spans.push(Span::styled("E", Style::default().fg(Color::Yellow)));
    spans.push(Span::styled("]", Style::default().fg(Color::DarkGray)));
    if state.extended_view {
        spans.push(Span::styled(" Ext:ON ", Style::default().fg(Color::Green)));
    } else {
        spans.push(Span::raw(" Ext:OFF "));
    }

    // Show files toggle
    spans.push(Span::styled("[", Style::default().fg(Color::DarkGray)));
    spans.push(Span::styled("F", Style::default().fg(Color::Yellow)));
    spans.push(Span::styled("]", Style::default().fg(Color::DarkGray)));
    if state.show_files {
        spans.push(Span::styled(" Files:ON ", Style::default().fg(Color::Green)));
    } else {
        spans.push(Span::raw(" Files:OFF "));
    }

    // Open in explorer
    spans.push(Span::styled("[", Style::default().fg(Color::DarkGray)));
    spans.push(Span::styled("O", Style::default().fg(Color::Yellow)));
    spans.push(Span::styled("]", Style::default().fg(Color::DarkGray)));
    spans.push(Span::raw(" Open "));

    // Copy path
    spans.push(Span::styled("[", Style::default().fg(Color::DarkGray)));
    spans.push(Span::styled("C", Style::default().fg(Color::Yellow)));
    spans.push(Span::styled("]", Style::default().fg(Color::DarkGray)));
    spans.push(Span::raw(" Copy"));

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(theme.disk_color));

    let paragraph = Paragraph::new(Line::from(spans)).block(block);
    f.render_widget(paragraph, area);
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
