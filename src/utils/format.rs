/// Format bytes to human-readable format
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    format!("{:.2} {}", size, UNITS[unit_index])
}

/// Format percentage
pub fn format_percentage(value: f32) -> String {
    format!("{:.1}%", value)
}

/// Create progress bar string
pub fn create_progress_bar(percentage: f32, width: usize) -> String {
    let filled = ((percentage / 100.0) * width as f32) as usize;
    let empty = width.saturating_sub(filled);

    format!(
        "{}{}",
        "█".repeat(filled),
        "░".repeat(empty)
    )
}
