use unicode_width::UnicodeWidthStr;

use super::expr::{BinaryOp, Expr, UnaryOp};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormulaRender {
    pub pretty: Vec<String>,
    pub fallback: String,
}

#[derive(Debug, Clone)]
struct LayoutBox {
    lines: Vec<String>,
    baseline: usize,
}

impl LayoutBox {
    fn text(value: impl Into<String>) -> Self {
        Self {
            lines: vec![value.into()],
            baseline: 0,
        }
    }

    fn width(&self) -> usize {
        self.lines
            .iter()
            .map(|line| UnicodeWidthStr::width(line.as_str()))
            .max()
            .unwrap_or(0)
    }

    fn height(&self) -> usize {
        self.lines.len()
    }

    fn padded_lines(&self, baseline: usize, height: usize) -> Vec<String> {
        let width = self.width();
        let top = baseline.saturating_sub(self.baseline);
        let mut out = Vec::with_capacity(height);
        for row in 0..height {
            if row < top || row >= top + self.height() {
                out.push(" ".repeat(width));
            } else {
                out.push(pad_to_width(&self.lines[row - top], width));
            }
        }
        out
    }
}

pub fn render_formula(expr: &Expr, lhs: Option<&str>, max_width: usize) -> FormulaRender {
    let fallback_expr = format_expr(expr);
    let fallback = lhs
        .map(|lhs| format!("{lhs} = {fallback_expr}"))
        .unwrap_or_else(|| fallback_expr.clone());

    let mut layout = render_expr(expr);
    if let Some(lhs) = lhs {
        layout = hstack(vec![LayoutBox::text(format!("{lhs} = ")), layout]);
    }

    let pretty = if layout.width() <= max_width.max(16) {
        layout.lines
    } else {
        vec![fallback.clone()]
    };

    FormulaRender { pretty, fallback }
}

pub fn format_expr(expr: &Expr) -> String {
    format_expr_prec(expr, 0)
}

fn render_expr(expr: &Expr) -> LayoutBox {
    match expr {
        Expr::Number(value) => LayoutBox::text(format_number_literal(*value)),
        Expr::Variable(name) => LayoutBox::text(name.clone()),
        Expr::Unary { op, expr } => match op {
            UnaryOp::Positive => render_expr(expr),
            UnaryOp::Negative => hstack(vec![LayoutBox::text("-"), render_factor(expr)]),
        },
        Expr::Binary { op, left, right } => match op {
            BinaryOp::Divide => fraction(render_expr(left), render_expr(right)),
            BinaryOp::Power => superscript(render_factor(left), render_factor(right)),
            BinaryOp::Multiply => hstack(vec![
                render_factor(left),
                LayoutBox::text("*"),
                render_factor(right),
            ]),
            BinaryOp::Add => hstack(vec![
                render_expr(left),
                LayoutBox::text(" + "),
                render_expr(right),
            ]),
            BinaryOp::Subtract => hstack(vec![
                render_expr(left),
                LayoutBox::text(" - "),
                render_expr(right),
            ]),
            BinaryOp::Remainder => hstack(vec![
                render_factor(left),
                LayoutBox::text(" % "),
                render_factor(right),
            ]),
        },
        Expr::Function { name, args } if name == "sqrt" && args.len() == 1 => {
            sqrt(render_expr(&args[0]))
        }
        Expr::Function { name, args } => {
            let mut parts = vec![LayoutBox::text(format!("{name}("))];
            for (idx, arg) in args.iter().enumerate() {
                if idx > 0 {
                    parts.push(LayoutBox::text(", "));
                }
                parts.push(render_expr(arg));
            }
            parts.push(LayoutBox::text(")"));
            hstack(parts)
        }
    }
}

fn render_factor(expr: &Expr) -> LayoutBox {
    match expr {
        Expr::Number(_) | Expr::Variable(_) | Expr::Function { .. } => render_expr(expr),
        Expr::Unary { .. } | Expr::Binary { .. } => hstack(vec![
            LayoutBox::text("("),
            render_expr(expr),
            LayoutBox::text(")"),
        ]),
    }
}

fn hstack(parts: Vec<LayoutBox>) -> LayoutBox {
    let baseline = parts.iter().map(|part| part.baseline).max().unwrap_or(0);
    let below = parts
        .iter()
        .map(|part| part.height().saturating_sub(part.baseline + 1))
        .max()
        .unwrap_or(0);
    let height = baseline + below + 1;
    let padded = parts
        .iter()
        .map(|part| part.padded_lines(baseline, height))
        .collect::<Vec<_>>();

    let mut lines = vec![String::new(); height];
    for part in padded {
        for (row, line) in part.into_iter().enumerate() {
            lines[row].push_str(&line);
        }
    }

    LayoutBox { lines, baseline }
}

fn fraction(numerator: LayoutBox, denominator: LayoutBox) -> LayoutBox {
    let width = numerator.width().max(denominator.width()).max(1);
    let mut lines = Vec::new();
    for line in numerator.lines {
        lines.push(center_to_width(&line, width));
    }
    lines.push("-".repeat(width));
    let baseline = lines.len() - 1;
    for line in denominator.lines {
        lines.push(center_to_width(&line, width));
    }
    LayoutBox { lines, baseline }
}

fn superscript(base: LayoutBox, exponent: LayoutBox) -> LayoutBox {
    let mut top = exponent.lines;
    let base_width = base.width();
    let base_baseline = base.baseline;
    let base_lines = base.lines;
    let top_height = top.len();
    for line in &mut top {
        *line = format!("{}{}", " ".repeat(base_width), line);
    }
    let mut lines = top;
    lines.extend(base_lines);
    LayoutBox {
        lines,
        baseline: top_height + base_baseline,
    }
}

fn sqrt(inner: LayoutBox) -> LayoutBox {
    let width = inner.width();
    let mut lines = Vec::with_capacity(inner.height() + 1);
    lines.push(format!("  {}", "_".repeat(width)));
    for (idx, line) in inner.lines.iter().enumerate() {
        let prefix = if idx + 1 == inner.height() {
            "\\/"
        } else {
            " |"
        };
        lines.push(format!("{prefix} {}", pad_to_width(line, width)));
    }
    LayoutBox {
        lines,
        baseline: inner.baseline + 1,
    }
}

fn format_expr_prec(expr: &Expr, parent_prec: u8) -> String {
    match expr {
        Expr::Number(value) => format_number_literal(*value),
        Expr::Variable(name) => name.clone(),
        Expr::Unary { op, expr } => match op {
            UnaryOp::Positive => format_expr_prec(expr, 8),
            UnaryOp::Negative => format!("-{}", format_expr_prec(expr, 8)),
        },
        Expr::Binary { op, left, right } => {
            let (prec, symbol, right_prec) = match op {
                BinaryOp::Add => (1, " + ", 2),
                BinaryOp::Subtract => (1, " - ", 2),
                BinaryOp::Multiply => (3, "*", 4),
                BinaryOp::Divide => (3, " / ", 4),
                BinaryOp::Remainder => (3, " % ", 4),
                BinaryOp::Power => (6, "^", 6),
            };
            let rendered = format!(
                "{}{}{}",
                format_expr_prec(left, prec),
                symbol,
                format_expr_prec(right, right_prec)
            );
            if prec < parent_prec {
                format!("({rendered})")
            } else {
                rendered
            }
        }
        Expr::Function { name, args } => {
            let args = args.iter().map(format_expr).collect::<Vec<_>>().join(", ");
            format!("{name}({args})")
        }
    }
}

fn format_number_literal(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_992.0 {
        format!("{value:.0}")
    } else {
        let mut value = format!("{value:.12}");
        while value.contains('.') && value.ends_with('0') {
            value.pop();
        }
        if value.ends_with('.') {
            value.pop();
        }
        value
    }
}

fn pad_to_width(input: &str, width: usize) -> String {
    let current = UnicodeWidthStr::width(input);
    if current >= width {
        input.to_string()
    } else {
        format!("{input}{}", " ".repeat(width - current))
    }
}

fn center_to_width(input: &str, width: usize) -> String {
    let current = UnicodeWidthStr::width(input);
    if current >= width {
        return input.to_string();
    }
    let left = (width - current) / 2;
    let right = width - current - left;
    format!("{}{}{}", " ".repeat(left), input, " ".repeat(right))
}
