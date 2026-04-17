use std::collections::{BTreeMap, VecDeque};

use super::exact::{format_number, format_pi_multiple};
use super::expr::{EvalContext, Expr, MathError};

pub const MIN_PLOT_SAMPLES: usize = 16;
pub const MAX_PLOT_SAMPLES: usize = 4096;
pub const MIN_PLOT_WIDTH: usize = 24;
pub const MAX_PLOT_WIDTH: usize = 160;
pub const MIN_PLOT_HEIGHT: usize = 6;
pub const MAX_PLOT_HEIGHT: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlotMode {
    Line,
    Points,
    Bars,
    Sparkline,
}

impl PlotMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Points => "points",
            Self::Bars => "bars",
            Self::Sparkline => "sparkline",
        }
    }

    pub fn parse(input: &str) -> Option<Self> {
        match input.to_ascii_lowercase().as_str() {
            "line" => Some(Self::Line),
            "points" | "point" => Some(Self::Points),
            "bars" | "bar" => Some(Self::Bars),
            "spark" | "sparkline" | "compact" => Some(Self::Sparkline),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlotRequest {
    pub expression: String,
    pub expr: Expr,
    pub variable: String,
    pub x_min: f64,
    pub x_max: f64,
    pub y_range: Option<(f64, f64)>,
    pub samples: usize,
    pub width: usize,
    pub height: usize,
    pub mode: PlotMode,
}

impl PlotRequest {
    pub fn validate(&self) -> Result<(), MathError> {
        if self.expression.trim().is_empty() {
            return Err(MathError::new("plot expression is empty"));
        }
        if self.variable.trim().is_empty() {
            return Err(MathError::new("missing plot variable"));
        }
        if !self.x_min.is_finite() || !self.x_max.is_finite() || self.x_min >= self.x_max {
            return Err(MathError::new("plot x range must be finite and increasing"));
        }
        if let Some((min, max)) = self.y_range {
            if !min.is_finite() || !max.is_finite() || min >= max {
                return Err(MathError::new("plot y range must be finite and increasing"));
            }
        }
        if !(MIN_PLOT_SAMPLES..=MAX_PLOT_SAMPLES).contains(&self.samples) {
            return Err(MathError::new(format!(
                "plot samples must be in {MIN_PLOT_SAMPLES}..{MAX_PLOT_SAMPLES}"
            )));
        }
        if !(MIN_PLOT_WIDTH..=MAX_PLOT_WIDTH).contains(&self.width) {
            return Err(MathError::new(format!(
                "plot width must be in {MIN_PLOT_WIDTH}..{MAX_PLOT_WIDTH}"
            )));
        }
        if !(MIN_PLOT_HEIGHT..=MAX_PLOT_HEIGHT).contains(&self.height) {
            return Err(MathError::new(format!(
                "plot height must be in {MIN_PLOT_HEIGHT}..{MAX_PLOT_HEIGHT}"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct PlotRender {
    pub canvas: Vec<String>,
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
    pub samples: usize,
    pub finite_samples: usize,
    pub invalid_samples: usize,
    pub clipped_samples: usize,
    pub discontinuities: usize,
    pub mode: PlotMode,
}

#[derive(Debug, Clone, Default)]
pub struct PlotCache {
    entries: VecDeque<(PlotCacheKey, PlotRender)>,
}

impl PlotCache {
    const CAPACITY: usize = 16;

    pub fn render(
        &mut self,
        request: &PlotRequest,
        variables: &BTreeMap<String, f64>,
    ) -> Result<(PlotRender, bool), MathError> {
        let key = PlotCacheKey::new(request, variables);
        if let Some((_, render)) = self.entries.iter().find(|(entry_key, _)| entry_key == &key) {
            return Ok((render.clone(), true));
        }

        let render = render_plot(request, variables)?;
        self.entries.push_back((key, render.clone()));
        while self.entries.len() > Self::CAPACITY {
            self.entries.pop_front();
        }
        Ok((render, false))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlotCacheKey {
    expression: String,
    variable: String,
    x_min: u64,
    x_max: u64,
    y_range: Option<(u64, u64)>,
    samples: usize,
    width: usize,
    height: usize,
    mode: PlotMode,
    params: Vec<(String, u64)>,
}

impl PlotCacheKey {
    fn new(request: &PlotRequest, variables: &BTreeMap<String, f64>) -> Self {
        let params = request
            .expr
            .variables()
            .into_iter()
            .filter(|name| name != &request.variable)
            .filter_map(|name| variables.get(&name).map(|value| (name, value.to_bits())))
            .collect();

        Self {
            expression: request.expression.trim().to_string(),
            variable: request.variable.clone(),
            x_min: request.x_min.to_bits(),
            x_max: request.x_max.to_bits(),
            y_range: request
                .y_range
                .map(|(min, max)| (min.to_bits(), max.to_bits())),
            samples: request.samples,
            width: request.width,
            height: request.height,
            mode: request.mode,
            params,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PlotSample {
    x: f64,
    y: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
struct MappedPoint {
    col: usize,
    row: usize,
    y: f64,
}

pub fn render_plot(
    request: &PlotRequest,
    variables: &BTreeMap<String, f64>,
) -> Result<PlotRender, MathError> {
    request.validate()?;

    let samples = sample_expression(request, variables);
    let finite = samples
        .iter()
        .filter_map(|sample| sample.y)
        .collect::<Vec<_>>();
    if finite.is_empty() {
        return Err(MathError::new(
            "plot has no finite y values in the requested domain",
        ));
    }

    let (y_min, y_max) = request
        .y_range
        .unwrap_or_else(|| auto_y_range(&finite, request.mode));
    if y_min >= y_max {
        return Err(MathError::new("plot y range collapsed"));
    }

    let (canvas, stats) = match request.mode {
        PlotMode::Sparkline => render_sparkline(request, &samples, y_min, y_max),
        PlotMode::Line | PlotMode::Points | PlotMode::Bars => {
            render_grid(request, &samples, y_min, y_max)
        }
    };

    Ok(PlotRender {
        canvas,
        x_min: request.x_min,
        x_max: request.x_max,
        y_min,
        y_max,
        samples: request.samples,
        finite_samples: stats.finite_samples,
        invalid_samples: stats.invalid_samples,
        clipped_samples: stats.clipped_samples,
        discontinuities: stats.discontinuities,
        mode: request.mode,
    })
}

#[derive(Debug, Clone, Copy, Default)]
struct PlotStats {
    finite_samples: usize,
    invalid_samples: usize,
    clipped_samples: usize,
    discontinuities: usize,
}

fn sample_expression(request: &PlotRequest, variables: &BTreeMap<String, f64>) -> Vec<PlotSample> {
    let mut samples = Vec::with_capacity(request.samples);
    let denominator = request.samples.saturating_sub(1).max(1) as f64;
    for idx in 0..request.samples {
        let t = idx as f64 / denominator;
        let x = request.x_min + (request.x_max - request.x_min) * t;
        let mut local_vars = variables.clone();
        local_vars.insert(request.variable.clone(), x);
        let y = request
            .expr
            .eval(&EvalContext::with_variables(local_vars))
            .ok()
            .filter(|value| value.is_finite());
        samples.push(PlotSample { x, y });
    }
    samples
}

fn auto_y_range(values: &[f64], mode: PlotMode) -> (f64, f64) {
    let mut sorted = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    sorted.sort_by(|lhs, rhs| lhs.partial_cmp(rhs).unwrap_or(std::cmp::Ordering::Equal));

    let raw_min = *sorted.first().unwrap_or(&-1.0);
    let raw_max = *sorted.last().unwrap_or(&1.0);
    let (mut min, mut max) = if sorted.len() >= 20 {
        let lo = quantile(&sorted, 0.02);
        let hi = quantile(&sorted, 0.98);
        if lo < hi {
            (lo, hi)
        } else {
            (raw_min, raw_max)
        }
    } else {
        (raw_min, raw_max)
    };

    if matches!(mode, PlotMode::Bars) || raw_min <= 0.0 && raw_max >= 0.0 {
        min = min.min(0.0);
        max = max.max(0.0);
    }

    if min == max {
        let pad = min.abs().max(1.0) * 0.5;
        min -= pad;
        max += pad;
    } else {
        let pad = (max - min).abs() * 0.08;
        min -= pad;
        max += pad;
    }
    (min, max)
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    let idx = ((sorted.len().saturating_sub(1)) as f64 * q).round() as usize;
    sorted[idx.min(sorted.len().saturating_sub(1))]
}

fn render_grid(
    request: &PlotRequest,
    samples: &[PlotSample],
    y_min: f64,
    y_max: f64,
) -> (Vec<String>, PlotStats) {
    let label_width = 10usize;
    let plot_width = request.width.saturating_sub(label_width + 2).max(8);
    let height = request.height.max(MIN_PLOT_HEIGHT);
    let mut grid = vec![vec![' '; plot_width]; height];
    let mut stats = PlotStats::default();

    let zero_col = if request.x_min <= 0.0 && request.x_max >= 0.0 {
        Some(map_x(0.0, request.x_min, request.x_max, plot_width))
    } else {
        None
    };
    let zero_row = if y_min <= 0.0 && y_max >= 0.0 {
        Some(map_y(0.0, y_min, y_max, height))
    } else {
        None
    };

    if let Some(col) = zero_col {
        for row in &mut grid {
            row[col] = '|';
        }
    }
    if let Some(row) = zero_row {
        for col in 0..plot_width {
            grid[row][col] = '-';
        }
    }
    if let (Some(col), Some(row)) = (zero_col, zero_row) {
        grid[row][col] = '+';
    }

    let mut previous: Option<MappedPoint> = None;
    let y_span = y_max - y_min;
    for sample in samples {
        let Some(y) = sample.y else {
            stats.invalid_samples += 1;
            previous = None;
            continue;
        };
        stats.finite_samples += 1;

        let mapped = map_point(sample.x, y, request, y_min, y_max, plot_width, height);
        let Some(point) = mapped else {
            stats.clipped_samples += 1;
            previous = None;
            continue;
        };

        match request.mode {
            PlotMode::Points => put(&mut grid, point.row, point.col, '*'),
            PlotMode::Bars => draw_bar(&mut grid, point, zero_row.unwrap_or(height - 1)),
            PlotMode::Line | PlotMode::Sparkline => {
                if let Some(prev) = previous {
                    if is_discontinuity(prev.y, point.y, y_span) {
                        stats.discontinuities += 1;
                    } else {
                        draw_segment(&mut grid, prev, point, '*');
                    }
                }
                put(&mut grid, point.row, point.col, '*');
                previous = Some(point);
            }
        }
    }

    let mut lines = Vec::with_capacity(height + 1);
    for (row, cells) in grid.into_iter().enumerate() {
        let label = y_label(row, height, y_min, y_max, zero_row);
        lines.push(format!(
            "{label:>label_width$} |{}",
            cells.into_iter().collect::<String>()
        ));
    }
    lines.push(x_label_line(
        label_width,
        plot_width,
        request.x_min,
        request.x_max,
        zero_col,
    ));

    (lines, stats)
}

fn render_sparkline(
    request: &PlotRequest,
    samples: &[PlotSample],
    y_min: f64,
    y_max: f64,
) -> (Vec<String>, PlotStats) {
    let width = request.width.saturating_sub(12).max(8);
    let buckets = bucket_samples(samples, width);
    let mut stats = PlotStats::default();
    let palette = [' ', '.', ':', '-', '=', '+', '*', '#', '@'];
    let mut line = String::with_capacity(width);
    for value in buckets {
        let Some(y) = value else {
            stats.invalid_samples += 1;
            line.push(' ');
            continue;
        };
        stats.finite_samples += 1;
        if y < y_min || y > y_max {
            stats.clipped_samples += 1;
        }
        let t = ((y - y_min) / (y_max - y_min)).clamp(0.0, 1.0);
        let idx = (t * (palette.len() - 1) as f64).round() as usize;
        line.push(palette[idx]);
    }

    (
        vec![
            format!("sparkline |{line}|"),
            format!(
                "x {} -> {}   y {} -> {}",
                format_pi_multiple(request.x_min),
                format_pi_multiple(request.x_max),
                format_number(y_min),
                format_number(y_max)
            ),
        ],
        stats,
    )
}

fn bucket_samples(samples: &[PlotSample], width: usize) -> Vec<Option<f64>> {
    let mut buckets = vec![Vec::new(); width];
    let denominator = samples.len().saturating_sub(1).max(1) as f64;
    for (idx, sample) in samples.iter().enumerate() {
        let col = ((idx as f64 / denominator) * (width.saturating_sub(1)) as f64).round() as usize;
        if let Some(y) = sample.y {
            buckets[col.min(width - 1)].push(y);
        }
    }
    buckets
        .into_iter()
        .map(|values| {
            if values.is_empty() {
                None
            } else {
                Some(values.iter().sum::<f64>() / values.len() as f64)
            }
        })
        .collect()
}

fn map_point(
    x: f64,
    y: f64,
    request: &PlotRequest,
    y_min: f64,
    y_max: f64,
    width: usize,
    height: usize,
) -> Option<MappedPoint> {
    if y < y_min || y > y_max {
        return None;
    }
    Some(MappedPoint {
        col: map_x(x, request.x_min, request.x_max, width),
        row: map_y(y, y_min, y_max, height),
        y,
    })
}

fn map_x(x: f64, min: f64, max: f64, width: usize) -> usize {
    (((x - min) / (max - min)).clamp(0.0, 1.0) * width.saturating_sub(1) as f64).round() as usize
}

fn map_y(y: f64, min: f64, max: f64, height: usize) -> usize {
    (((max - y) / (max - min)).clamp(0.0, 1.0) * height.saturating_sub(1) as f64).round() as usize
}

fn is_discontinuity(prev_y: f64, next_y: f64, y_span: f64) -> bool {
    if y_span <= 0.0 {
        return false;
    }
    let jump = (next_y - prev_y).abs();
    jump > y_span * 0.65 && prev_y.abs().max(next_y.abs()) > y_span * 0.45
}

fn draw_bar(grid: &mut [Vec<char>], point: MappedPoint, baseline: usize) {
    let start = point.row.min(baseline);
    let end = point.row.max(baseline);
    for row in start..=end {
        put(grid, row, point.col, '#');
    }
}

fn draw_segment(grid: &mut [Vec<char>], start: MappedPoint, end: MappedPoint, ch: char) {
    let mut x0 = start.col as isize;
    let mut y0 = start.row as isize;
    let x1 = end.col as isize;
    let y1 = end.row as isize;
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        if y0 >= 0 && x0 >= 0 {
            put(grid, y0 as usize, x0 as usize, ch);
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

fn put(grid: &mut [Vec<char>], row: usize, col: usize, ch: char) {
    if let Some(row) = grid.get_mut(row) {
        if let Some(cell) = row.get_mut(col) {
            *cell = ch;
        }
    }
}

fn y_label(row: usize, height: usize, y_min: f64, y_max: f64, zero_row: Option<usize>) -> String {
    if row == 0 {
        format_number(y_max)
    } else if row + 1 == height {
        format_number(y_min)
    } else if Some(row) == zero_row {
        "0".to_string()
    } else {
        String::new()
    }
}

fn x_label_line(
    label_width: usize,
    plot_width: usize,
    x_min: f64,
    x_max: f64,
    zero_col: Option<usize>,
) -> String {
    let mut chars = vec![' '; label_width + 2 + plot_width];
    put_text(&mut chars, 0, "x");
    put_text(&mut chars, label_width + 2, &format_pi_multiple(x_min));
    if let Some(col) = zero_col {
        put_text(&mut chars, label_width + 2 + col, "0");
    }
    let max_label = format_pi_multiple(x_max);
    let max_start = chars.len().saturating_sub(max_label.len());
    put_text(&mut chars, max_start, &max_label);
    chars.into_iter().collect()
}

fn put_text(chars: &mut [char], start: usize, text: &str) {
    for (offset, ch) in text.chars().enumerate() {
        if let Some(slot) = chars.get_mut(start + offset) {
            *slot = ch;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::math::parse_expression;

    fn request(expr: &str) -> PlotRequest {
        PlotRequest {
            expression: expr.to_string(),
            expr: parse_expression(expr).unwrap(),
            variable: "x".to_string(),
            x_min: -std::f64::consts::PI,
            x_max: std::f64::consts::PI,
            y_range: None,
            samples: 128,
            width: 80,
            height: 12,
            mode: PlotMode::Line,
        }
    }

    #[test]
    fn renders_basic_function_plot() {
        let render = render_plot(&request("sin(x)"), &BTreeMap::new()).unwrap();
        assert!(!render.canvas.is_empty());
        assert!(render.finite_samples > 100);
        assert!(render.canvas.iter().any(|line| line.contains('*')));
    }

    #[test]
    fn rejects_empty_finite_domain() {
        let mut request = request("sqrt(x)");
        request.x_min = -4.0;
        request.x_max = -1.0;
        let error = render_plot(&request, &BTreeMap::new()).unwrap_err();
        assert!(error.message.contains("no finite"));
    }

    #[test]
    fn cache_hits_identical_request() {
        let request = request("x^2");
        let mut cache = PlotCache::default();
        let (_, first_hit) = cache.render(&request, &BTreeMap::new()).unwrap();
        let (_, second_hit) = cache.render(&request, &BTreeMap::new()).unwrap();
        assert!(!first_hit);
        assert!(second_hit);
    }
}
