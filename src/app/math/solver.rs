use std::collections::BTreeMap;

use super::expr::{parse_expression, EvalContext, Expr, MathError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelationOp {
    Equal,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

impl RelationOp {
    pub fn label(self) -> &'static str {
        match self {
            Self::Equal => "=",
            Self::Less => "<",
            Self::LessEqual => "<=",
            Self::Greater => ">",
            Self::GreaterEqual => ">=",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Relation {
    pub left: Expr,
    pub op: RelationOp,
    pub right: Expr,
}

impl Relation {
    pub fn variables(&self) -> std::collections::BTreeSet<String> {
        let mut vars = self.left.variables();
        vars.extend(self.right.variables());
        vars
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SolveOptions {
    pub variable: String,
    pub min: f64,
    pub max: f64,
    pub samples: usize,
}

impl SolveOptions {
    pub fn new(variable: impl Into<String>, min: f64, max: f64) -> Self {
        Self {
            variable: variable.into(),
            min,
            max,
            samples: 2048,
        }
    }

    pub fn validate(&self) -> Result<(), MathError> {
        if self.variable.trim().is_empty() {
            return Err(MathError::new("missing solve variable"));
        }
        if !self.min.is_finite() || !self.max.is_finite() {
            return Err(MathError::new("solve domain must be finite"));
        }
        if self.min >= self.max {
            return Err(MathError::new(
                "solve domain start must be smaller than end",
            ));
        }
        if !(16..=4096).contains(&self.samples) {
            return Err(MathError::new("solve samples must be in 16..4096"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SolveReport {
    Equality {
        variable: String,
        min: f64,
        max: f64,
        roots: Vec<f64>,
    },
    Inequality {
        variable: String,
        min: f64,
        max: f64,
        intervals: Vec<Interval>,
    },
    Constant {
        holds: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Interval {
    pub start: f64,
    pub end: f64,
    pub include_start: bool,
    pub include_end: bool,
}

pub fn parse_relation(input: &str) -> Result<Option<Relation>, MathError> {
    let Some((start, end, op)) = find_relation_operator(input) else {
        return Ok(None);
    };

    let left = input[..start].trim();
    let right = input[end..].trim();
    if left.is_empty() || right.is_empty() {
        return Err(MathError::at(
            "relation requires expressions on both sides",
            start,
        ));
    }

    Ok(Some(Relation {
        left: parse_expression(left)?,
        op,
        right: parse_expression(right)?,
    }))
}

pub fn solve_relation(
    relation: &Relation,
    options: Option<SolveOptions>,
    variables: &BTreeMap<String, f64>,
) -> Result<SolveReport, MathError> {
    let unknowns = relation
        .variables()
        .into_iter()
        .filter(|name| !variables.contains_key(name))
        .collect::<Vec<_>>();

    if unknowns.is_empty() && options.is_none() {
        let residual = eval_residual(relation, variables, "", 0.0)?;
        return Ok(SolveReport::Constant {
            holds: relation_holds(relation.op, residual),
        });
    }

    let options = match options {
        Some(options) => options,
        None => {
            if unknowns.len() != 1 {
                return Err(MathError::new(format!(
                    "expected one unknown variable, found {}; use 'for <name>'",
                    unknowns.len()
                )));
            }
            SolveOptions::new(unknowns[0].clone(), -100.0, 100.0)
        }
    };
    options.validate()?;

    match relation.op {
        RelationOp::Equal => Ok(SolveReport::Equality {
            variable: options.variable.clone(),
            min: options.min,
            max: options.max,
            roots: find_roots(relation, &options, variables)?,
        }),
        RelationOp::Less
        | RelationOp::LessEqual
        | RelationOp::Greater
        | RelationOp::GreaterEqual => Ok(SolveReport::Inequality {
            variable: options.variable.clone(),
            min: options.min,
            max: options.max,
            intervals: find_intervals(relation, &options, variables)?,
        }),
    }
}

fn find_relation_operator(input: &str) -> Option<(usize, usize, RelationOp)> {
    let mut depth = 0i32;
    let mut chars = input.char_indices().peekable();

    while let Some((idx, ch)) = chars.next() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            '<' | '>' | '=' if depth == 0 => {
                let next = chars.peek().copied().map(|(_, ch)| ch);
                return match (ch, next) {
                    ('<', Some('=')) => Some((idx, idx + 2, RelationOp::LessEqual)),
                    ('>', Some('=')) => Some((idx, idx + 2, RelationOp::GreaterEqual)),
                    ('=', Some('=')) => Some((idx, idx + 2, RelationOp::Equal)),
                    ('<', _) => Some((idx, idx + 1, RelationOp::Less)),
                    ('>', _) => Some((idx, idx + 1, RelationOp::Greater)),
                    ('=', _) => Some((idx, idx + 1, RelationOp::Equal)),
                    _ => None,
                };
            }
            _ => {}
        }
    }

    None
}

fn find_roots(
    relation: &Relation,
    options: &SolveOptions,
    variables: &BTreeMap<String, f64>,
) -> Result<Vec<f64>, MathError> {
    let samples = options.samples.clamp(16, 4096);
    let step = (options.max - options.min) / samples as f64;
    let mut xs = Vec::with_capacity(samples + 1);
    let mut ys = Vec::with_capacity(samples + 1);
    let mut roots = Vec::new();

    for idx in 0..=samples {
        let x = options.min + step * idx as f64;
        let y = eval_residual(relation, variables, &options.variable, x).unwrap_or(f64::NAN);
        xs.push(x);
        ys.push(y);
    }

    for idx in 0..samples {
        let (x0, y0) = (xs[idx], ys[idx]);
        let (x1, y1) = (xs[idx + 1], ys[idx + 1]);
        if !y0.is_finite() || !y1.is_finite() {
            continue;
        }

        if near_zero(y0) {
            push_root(&mut roots, x0, options.min, options.max);
        }
        if y0.signum() != y1.signum() {
            if let Some(root) = bisect_root(relation, options, variables, x0, x1, y0, y1) {
                push_root(&mut roots, root, options.min, options.max);
            }
        }
    }

    if let Some((&last_x, &last_y)) = xs.last().zip(ys.last()) {
        if last_y.is_finite() && near_zero(last_y) {
            push_root(&mut roots, last_x, options.min, options.max);
        }
    }

    for idx in 1..samples {
        let y_prev = ys[idx - 1].abs();
        let y = ys[idx].abs();
        let y_next = ys[idx + 1].abs();
        if y.is_finite() && y < y_prev && y < y_next && y < 1e-4 {
            let (root, residual) =
                minimize_abs_residual(relation, options, variables, xs[idx - 1], xs[idx + 1]);
            if residual < 1e-7 {
                push_root(&mut roots, root, options.min, options.max);
            }
        }
    }

    roots.sort_by(|a, b| a.total_cmp(b));
    roots.dedup_by(|a, b| (*a - *b).abs() <= root_dedup_tolerance(options.min, options.max));
    Ok(roots)
}

fn find_intervals(
    relation: &Relation,
    options: &SolveOptions,
    variables: &BTreeMap<String, f64>,
) -> Result<Vec<Interval>, MathError> {
    let roots = find_roots(relation, options, variables)?;
    let mut points = Vec::with_capacity(roots.len() + 2);
    points.push(options.min);
    points.extend(
        roots
            .iter()
            .copied()
            .filter(|root| *root > options.min && *root < options.max),
    );
    points.push(options.max);
    points.sort_by(|a, b| a.total_cmp(b));
    points.dedup_by(|a, b| (*a - *b).abs() <= root_dedup_tolerance(options.min, options.max));

    let mut intervals = Vec::new();
    for pair in points.windows(2) {
        let start = pair[0];
        let end = pair[1];
        if end <= start {
            continue;
        }

        let probe = start + (end - start) / 2.0;
        let residual = eval_residual(relation, variables, &options.variable, probe)?;
        if relation_holds(relation.op, residual) {
            intervals.push(Interval {
                start,
                end,
                include_start: interval_endpoint_included(relation, options, variables, start),
                include_end: interval_endpoint_included(relation, options, variables, end),
            });
        }
    }

    if intervals.is_empty() && points.len() == 1 {
        let residual = eval_residual(relation, variables, &options.variable, options.min)?;
        if relation_holds(relation.op, residual) {
            intervals.push(Interval {
                start: options.min,
                end: options.max,
                include_start: true,
                include_end: true,
            });
        }
    }

    Ok(intervals)
}

fn bisect_root(
    relation: &Relation,
    options: &SolveOptions,
    variables: &BTreeMap<String, f64>,
    mut low: f64,
    mut high: f64,
    mut y_low: f64,
    mut y_high: f64,
) -> Option<f64> {
    if near_zero(y_low) {
        return Some(low);
    }
    if near_zero(y_high) {
        return Some(high);
    }
    if y_low.signum() == y_high.signum() {
        return None;
    }

    for _ in 0..80 {
        let mid = low + (high - low) / 2.0;
        let y_mid = eval_residual(relation, variables, &options.variable, mid).ok()?;
        if !y_mid.is_finite() {
            return None;
        }
        if near_zero(y_mid)
            || (high - low).abs() < root_precision_tolerance(options.min, options.max)
        {
            return Some(mid);
        }
        if y_low.signum() == y_mid.signum() {
            low = mid;
            y_low = y_mid;
        } else {
            high = mid;
            y_high = y_mid;
        }
    }

    let _ = y_high;
    Some(low + (high - low) / 2.0)
}

fn minimize_abs_residual(
    relation: &Relation,
    options: &SolveOptions,
    variables: &BTreeMap<String, f64>,
    mut left: f64,
    mut right: f64,
) -> (f64, f64) {
    let phi = (1.0 + 5.0_f64.sqrt()) / 2.0;
    let mut c = right - (right - left) / phi;
    let mut d = left + (right - left) / phi;

    for _ in 0..64 {
        let fc = eval_residual(relation, variables, &options.variable, c)
            .map(f64::abs)
            .unwrap_or(f64::INFINITY);
        let fd = eval_residual(relation, variables, &options.variable, d)
            .map(f64::abs)
            .unwrap_or(f64::INFINITY);
        if fc < fd {
            right = d;
        } else {
            left = c;
        }
        c = right - (right - left) / phi;
        d = left + (right - left) / phi;
    }

    let x = left + (right - left) / 2.0;
    let y = eval_residual(relation, variables, &options.variable, x)
        .map(f64::abs)
        .unwrap_or(f64::INFINITY);
    (x, y)
}

fn eval_residual(
    relation: &Relation,
    variables: &BTreeMap<String, f64>,
    variable: &str,
    value: f64,
) -> Result<f64, MathError> {
    let mut vars = variables.clone();
    if !variable.is_empty() {
        vars.insert(variable.to_string(), value);
    }
    let ctx = EvalContext::with_variables(vars);
    Ok(relation.left.eval(&ctx)? - relation.right.eval(&ctx)?)
}

fn relation_holds(op: RelationOp, residual: f64) -> bool {
    match op {
        RelationOp::Equal => near_zero(residual),
        RelationOp::Less => residual < 0.0,
        RelationOp::LessEqual => residual <= 0.0 || near_zero(residual),
        RelationOp::Greater => residual > 0.0,
        RelationOp::GreaterEqual => residual >= 0.0 || near_zero(residual),
    }
}

fn interval_endpoint_included(
    relation: &Relation,
    options: &SolveOptions,
    variables: &BTreeMap<String, f64>,
    value: f64,
) -> bool {
    if is_domain_endpoint(value, options.min) || is_domain_endpoint(value, options.max) {
        return endpoint_holds(relation, options, variables, value);
    }

    matches!(
        relation.op,
        RelationOp::LessEqual | RelationOp::GreaterEqual
    )
}

fn endpoint_holds(
    relation: &Relation,
    options: &SolveOptions,
    variables: &BTreeMap<String, f64>,
    value: f64,
) -> bool {
    eval_residual(relation, variables, &options.variable, value)
        .map(|residual| relation_holds(relation.op, residual))
        .unwrap_or(false)
}

fn near_zero(value: f64) -> bool {
    value.abs() <= 1e-10
}

fn root_dedup_tolerance(min: f64, max: f64) -> f64 {
    ((max - min).abs() * 1e-9).max(1e-9)
}

fn root_precision_tolerance(min: f64, max: f64) -> f64 {
    ((max - min).abs() * 1e-12).max(1e-12)
}

fn is_domain_endpoint(value: f64, endpoint: f64) -> bool {
    (value - endpoint).abs() <= root_precision_tolerance(endpoint, value)
}

fn push_root(roots: &mut Vec<f64>, root: f64, min: f64, max: f64) {
    if root < min || root > max || !root.is_finite() {
        return;
    }
    let tol = root_dedup_tolerance(min, max);
    if roots.iter().all(|existing| (*existing - root).abs() > tol) {
        roots.push(root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solves_quadratic_roots() {
        let relation = parse_relation("x^2 - 4 = 0").unwrap().unwrap();
        let report = solve_relation(
            &relation,
            Some(SolveOptions::new("x", -10.0, 10.0)),
            &BTreeMap::new(),
        )
        .unwrap();
        let SolveReport::Equality { roots, .. } = report else {
            panic!("expected equality report");
        };
        assert!(roots.iter().any(|root| (*root + 2.0).abs() < 1e-7));
        assert!(roots.iter().any(|root| (*root - 2.0).abs() < 1e-7));
    }

    #[test]
    fn solves_basic_inequality() {
        let relation = parse_relation("x^2 < 4").unwrap().unwrap();
        let report = solve_relation(
            &relation,
            Some(SolveOptions::new("x", -10.0, 10.0)),
            &BTreeMap::new(),
        )
        .unwrap();
        let SolveReport::Inequality { intervals, .. } = report else {
            panic!("expected inequality report");
        };
        assert_eq!(intervals.len(), 1);
        assert!((intervals[0].start + 2.0).abs() < 1e-7);
        assert!((intervals[0].end - 2.0).abs() < 1e-7);
    }
}
