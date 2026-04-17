use std::collections::BTreeMap;

use super::expr::{BinaryOp, EvalContext, Expr, UnaryOp};
use super::formula::format_expr;
use super::solver::{Relation, RelationOp};

#[derive(Debug, Clone, PartialEq)]
pub struct ExactSolveReport {
    pub variable: String,
    pub method: &'static str,
    pub status: &'static str,
    pub exact_lines: Vec<String>,
    pub numeric_lines: Vec<String>,
    pub domain: Option<(f64, f64)>,
}

pub fn solve_exact(
    relation: &Relation,
    variable: &str,
    variables: &BTreeMap<String, f64>,
    domain: Option<(f64, f64)>,
) -> Option<ExactSolveReport> {
    solve_exact_trig(relation, variable, domain)
        .or_else(|| solve_exact_radical(relation, variable, variables, domain))
        .or_else(|| solve_exact_polynomial(relation, variable, variables, domain))
}

fn solve_exact_radical(
    relation: &Relation,
    variable: &str,
    variables: &BTreeMap<String, f64>,
    domain: Option<(f64, f64)>,
) -> Option<ExactSolveReport> {
    if relation.op != RelationOp::Equal {
        return None;
    }

    let left_sqrt = sqrt_linear_coeff(&relation.left, variable, variables);
    let right_sqrt = sqrt_linear_coeff(&relation.right, variable, variables);
    match (left_sqrt, right_sqrt) {
        (Some(coeff), None) => {
            let rhs = constant_value(&relation.right, variables)?;
            radical_report(variable, coeff, rhs, domain)
        }
        (None, Some(coeff)) => {
            let lhs = constant_value(&relation.left, variables)?;
            radical_report(variable, coeff, lhs, domain)
        }
        _ => None,
    }
}

fn radical_report(
    variable: &str,
    coeff: f64,
    target: f64,
    domain: Option<(f64, f64)>,
) -> Option<ExactSolveReport> {
    if coeff.abs() <= EPS {
        return None;
    }
    let root_value = target / coeff;
    let solution = root_value * root_value;
    if !solution.is_finite() {
        return None;
    }
    Some(ExactSolveReport {
        variable: variable.to_string(),
        method: "radical",
        status: "exact",
        exact_lines: vec![format!("{variable} = {}", format_exact_number(solution))],
        numeric_lines: vec![format!("{variable} ~= {}", format_number(solution))],
        domain,
    })
}

fn solve_exact_polynomial(
    relation: &Relation,
    variable: &str,
    variables: &BTreeMap<String, f64>,
    domain: Option<(f64, f64)>,
) -> Option<ExactSolveReport> {
    if relation.op != RelationOp::Equal {
        return None;
    }

    let left = polynomial_coeffs(&relation.left, variable, variables)?;
    let right = polynomial_coeffs(&relation.right, variable, variables)?;
    let coeffs = [left[0] - right[0], left[1] - right[1], left[2] - right[2]];

    let roots = solve_polynomial_roots(coeffs)?;
    Some(ExactSolveReport {
        variable: variable.to_string(),
        method: if coeffs[2].abs() > EPS {
            "quadratic"
        } else {
            "linear"
        },
        status: "exact",
        exact_lines: roots
            .iter()
            .map(|root| {
                if root.numeric.is_none() {
                    root.exact.clone()
                } else {
                    format!("{variable} = {}", root.exact)
                }
            })
            .collect(),
        numeric_lines: roots
            .iter()
            .filter_map(|root| {
                root.numeric
                    .map(|numeric| format!("{variable} ~= {}", format_number(numeric)))
            })
            .collect(),
        domain,
    })
}

fn solve_exact_trig(
    relation: &Relation,
    variable: &str,
    domain: Option<(f64, f64)>,
) -> Option<ExactSolveReport> {
    let trig = match_trig_zero_relation(relation, variable)?;
    if let Some((min, max)) = domain {
        let intervals = trig_domain_lines(variable, trig.func, trig.op, min, max)?;
        return Some(ExactSolveReport {
            variable: variable.to_string(),
            method: "trig-domain",
            status: "exact",
            numeric_lines: intervals.iter().map(|line| line.numeric.clone()).collect(),
            exact_lines: intervals.into_iter().map(|line| line.exact).collect(),
            domain,
        });
    }

    let line = match (trig.func, trig.op) {
        ("sin", RelationOp::Equal) => format!("{variable} = k*pi, k in Z"),
        ("sin", RelationOp::Greater) => {
            format!("{variable} in (2k*pi, (2k + 1)pi), k in Z")
        }
        ("sin", RelationOp::GreaterEqual) => {
            format!("{variable} in [2k*pi, (2k + 1)pi], k in Z")
        }
        ("sin", RelationOp::Less) => {
            format!("{variable} in ((2k - 1)pi, 2k*pi), k in Z")
        }
        ("sin", RelationOp::LessEqual) => {
            format!("{variable} in [(2k - 1)pi, 2k*pi], k in Z")
        }
        ("cos", RelationOp::Equal) => format!("{variable} = pi/2 + k*pi, k in Z"),
        ("cos", RelationOp::Greater) => {
            format!("{variable} in (-pi/2 + 2k*pi, pi/2 + 2k*pi), k in Z")
        }
        ("cos", RelationOp::GreaterEqual) => {
            format!("{variable} in [-pi/2 + 2k*pi, pi/2 + 2k*pi], k in Z")
        }
        ("cos", RelationOp::Less) => {
            format!("{variable} in (pi/2 + 2k*pi, 3pi/2 + 2k*pi), k in Z")
        }
        ("cos", RelationOp::LessEqual) => {
            format!("{variable} in [pi/2 + 2k*pi, 3pi/2 + 2k*pi], k in Z")
        }
        _ => return None,
    };

    Some(ExactSolveReport {
        variable: variable.to_string(),
        method: "trig-family",
        status: "exact-family",
        exact_lines: vec![line],
        numeric_lines: vec![
            "Use a bounded domain, for example 'from -2pi..2pi', for numeric windows.".to_string(),
        ],
        domain,
    })
}

#[derive(Debug, Clone, Copy)]
struct TrigRelation {
    func: &'static str,
    op: RelationOp,
}

fn match_trig_zero_relation(relation: &Relation, variable: &str) -> Option<TrigRelation> {
    if is_zero_expr(&relation.right) {
        let func = match_trig_variable(&relation.left, variable)?;
        return Some(TrigRelation {
            func,
            op: relation.op,
        });
    }

    if is_zero_expr(&relation.left) {
        let func = match_trig_variable(&relation.right, variable)?;
        return Some(TrigRelation {
            func,
            op: reverse_relation_op(relation.op),
        });
    }

    None
}

fn match_trig_variable(expr: &Expr, variable: &str) -> Option<&'static str> {
    let Expr::Function { name, args } = expr else {
        return None;
    };
    if args.len() != 1 {
        return None;
    }
    let Expr::Variable(arg) = &args[0] else {
        return None;
    };
    if arg != variable {
        return None;
    }
    match name.as_str() {
        "sin" => Some("sin"),
        "cos" => Some("cos"),
        _ => None,
    }
}

fn reverse_relation_op(op: RelationOp) -> RelationOp {
    match op {
        RelationOp::Equal => RelationOp::Equal,
        RelationOp::Less => RelationOp::Greater,
        RelationOp::LessEqual => RelationOp::GreaterEqual,
        RelationOp::Greater => RelationOp::Less,
        RelationOp::GreaterEqual => RelationOp::LessEqual,
    }
}

#[derive(Debug, Clone)]
struct TrigIntervalLine {
    exact: String,
    numeric: String,
}

fn trig_domain_lines(
    variable: &str,
    func: &str,
    op: RelationOp,
    min: f64,
    max: f64,
) -> Option<Vec<TrigIntervalLine>> {
    let mut lines = Vec::new();
    let two_pi = std::f64::consts::TAU;
    let k_min = (min / two_pi).floor() as i64 - 2;
    let k_max = (max / two_pi).ceil() as i64 + 2;

    for k in k_min..=k_max {
        let intervals = match (func, op) {
            ("sin", RelationOp::Greater) => vec![(
                2.0 * k as f64 * std::f64::consts::PI,
                (2 * k + 1) as f64 * std::f64::consts::PI,
                false,
                false,
            )],
            ("sin", RelationOp::GreaterEqual) => vec![(
                2.0 * k as f64 * std::f64::consts::PI,
                (2 * k + 1) as f64 * std::f64::consts::PI,
                true,
                true,
            )],
            ("sin", RelationOp::Less) => vec![(
                ((2 * k - 1) as f64) * std::f64::consts::PI,
                2.0 * k as f64 * std::f64::consts::PI,
                false,
                false,
            )],
            ("sin", RelationOp::LessEqual) => vec![(
                ((2 * k - 1) as f64) * std::f64::consts::PI,
                2.0 * k as f64 * std::f64::consts::PI,
                true,
                true,
            )],
            ("cos", RelationOp::Greater) => vec![(
                (-0.5 + 2.0 * k as f64) * std::f64::consts::PI,
                (0.5 + 2.0 * k as f64) * std::f64::consts::PI,
                false,
                false,
            )],
            ("cos", RelationOp::GreaterEqual) => vec![(
                (-0.5 + 2.0 * k as f64) * std::f64::consts::PI,
                (0.5 + 2.0 * k as f64) * std::f64::consts::PI,
                true,
                true,
            )],
            ("cos", RelationOp::Less) => vec![(
                (0.5 + 2.0 * k as f64) * std::f64::consts::PI,
                (1.5 + 2.0 * k as f64) * std::f64::consts::PI,
                false,
                false,
            )],
            ("cos", RelationOp::LessEqual) => vec![(
                (0.5 + 2.0 * k as f64) * std::f64::consts::PI,
                (1.5 + 2.0 * k as f64) * std::f64::consts::PI,
                true,
                true,
            )],
            _ => Vec::new(),
        };

        for (start, end, include_start, include_end) in intervals {
            let start = start.max(min);
            let end = end.min(max);
            if start < end {
                let left = if include_start || close(start, min) {
                    "["
                } else {
                    "("
                };
                let right = if include_end || close(end, max) {
                    "]"
                } else {
                    ")"
                };
                lines.push(TrigIntervalLine {
                    exact: format!(
                        "{variable} in {left}{}, {}{right}",
                        format_pi_multiple(start),
                        format_pi_multiple(end)
                    ),
                    numeric: format!(
                        "{variable} in {left}{}, {}{right}",
                        format_number(start),
                        format_number(end)
                    ),
                });
            }
        }
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
}

fn polynomial_coeffs(
    expr: &Expr,
    variable: &str,
    variables: &BTreeMap<String, f64>,
) -> Option<[f64; 3]> {
    match expr {
        Expr::Number(value) => Some([*value, 0.0, 0.0]),
        Expr::Variable(name) if name == variable => Some([0.0, 1.0, 0.0]),
        Expr::Variable(name) => {
            let value = EvalContext::with_variables(variables.clone()).resolve(name)?;
            Some([value, 0.0, 0.0])
        }
        Expr::Unary { op, expr } => {
            let coeffs = polynomial_coeffs(expr, variable, variables)?;
            match op {
                UnaryOp::Positive => Some(coeffs),
                UnaryOp::Negative => Some([-coeffs[0], -coeffs[1], -coeffs[2]]),
            }
        }
        Expr::Binary { op, left, right } => {
            let lhs = polynomial_coeffs(left, variable, variables)?;
            let rhs = polynomial_coeffs(right, variable, variables)?;
            match op {
                BinaryOp::Add => Some([lhs[0] + rhs[0], lhs[1] + rhs[1], lhs[2] + rhs[2]]),
                BinaryOp::Subtract => Some([lhs[0] - rhs[0], lhs[1] - rhs[1], lhs[2] - rhs[2]]),
                BinaryOp::Multiply => multiply_poly(lhs, rhs),
                BinaryOp::Divide
                    if rhs[1].abs() <= EPS && rhs[2].abs() <= EPS && rhs[0].abs() > EPS =>
                {
                    Some([lhs[0] / rhs[0], lhs[1] / rhs[0], lhs[2] / rhs[0]])
                }
                BinaryOp::Power if rhs[1].abs() <= EPS && rhs[2].abs() <= EPS => {
                    power_poly(lhs, rhs[0])
                }
                _ => None,
            }
        }
        Expr::Function { .. } => None,
    }
}

fn sqrt_linear_coeff(
    expr: &Expr,
    variable: &str,
    variables: &BTreeMap<String, f64>,
) -> Option<f64> {
    match expr {
        Expr::Function { name, args } if name == "sqrt" && args.len() == 1 => {
            if matches!(&args[0], Expr::Variable(name) if name == variable) {
                Some(1.0)
            } else {
                None
            }
        }
        Expr::Binary {
            op: BinaryOp::Divide,
            left,
            right,
        } => {
            let coeff = sqrt_linear_coeff(left, variable, variables)?;
            let divisor = constant_value(right, variables)?;
            if divisor.abs() <= EPS {
                None
            } else {
                Some(coeff / divisor)
            }
        }
        Expr::Binary {
            op: BinaryOp::Multiply,
            left,
            right,
        } => sqrt_linear_coeff(left, variable, variables)
            .and_then(|coeff| constant_value(right, variables).map(|constant| coeff * constant))
            .or_else(|| {
                sqrt_linear_coeff(right, variable, variables).and_then(|coeff| {
                    constant_value(left, variables).map(|constant| coeff * constant)
                })
            }),
        Expr::Unary {
            op: UnaryOp::Negative,
            expr,
        } => sqrt_linear_coeff(expr, variable, variables).map(|coeff| -coeff),
        Expr::Unary {
            op: UnaryOp::Positive,
            expr,
        } => sqrt_linear_coeff(expr, variable, variables),
        _ => None,
    }
}

fn constant_value(expr: &Expr, variables: &BTreeMap<String, f64>) -> Option<f64> {
    if expr
        .variables()
        .iter()
        .any(|name| !variables.contains_key(name))
    {
        return None;
    }
    expr.eval(&EvalContext::with_variables(variables.clone()))
        .ok()
}

fn multiply_poly(lhs: [f64; 3], rhs: [f64; 3]) -> Option<[f64; 3]> {
    let mut out = [0.0; 5];
    for (i, lhs) in lhs.iter().enumerate() {
        for (j, rhs) in rhs.iter().enumerate() {
            out[i + j] += lhs * rhs;
        }
    }
    if out[3].abs() > EPS || out[4].abs() > EPS {
        None
    } else {
        Some([out[0], out[1], out[2]])
    }
}

fn power_poly(base: [f64; 3], power: f64) -> Option<[f64; 3]> {
    if !close(power, power.round()) || power < 0.0 || power > 2.0 {
        return None;
    }
    match power.round() as i32 {
        0 => Some([1.0, 0.0, 0.0]),
        1 => Some(base),
        2 => multiply_poly(base, base),
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct RootDisplay {
    exact: String,
    numeric: Option<f64>,
}

fn solve_polynomial_roots(coeffs: [f64; 3]) -> Option<Vec<RootDisplay>> {
    let [c, b, a] = coeffs.map(clean_zero);
    if a.abs() <= EPS && b.abs() <= EPS {
        return Some(Vec::new());
    }
    if a.abs() <= EPS {
        let root = -c / b;
        return Some(vec![RootDisplay {
            exact: format_exact_number(root),
            numeric: Some(root),
        }]);
    }

    let discriminant = b * b - 4.0 * a * c;
    if discriminant < -EPS {
        return Some(vec![RootDisplay {
            exact: "no real roots".to_string(),
            numeric: None,
        }]);
    }
    if close(discriminant, 0.0) {
        let root = -b / (2.0 * a);
        return Some(vec![RootDisplay {
            exact: format_exact_number(root),
            numeric: Some(root),
        }]);
    }

    let a_i = near_i64(a)?;
    let b_i = near_i64(b)?;
    let c_i = near_i64(c)?;
    let disc_i = b_i.checked_mul(b_i)?.checked_sub(4 * a_i * c_i)?;
    if disc_i < 0 {
        return None;
    }
    let sqrt = simplify_sqrt(disc_i as u64);
    let denom = 2 * a_i;
    let roots = format_quadratic_roots(-b_i, sqrt, denom);
    let sqrt_disc = discriminant.sqrt();
    Some(vec![
        RootDisplay {
            exact: roots.0,
            numeric: Some((-b - sqrt_disc) / (2.0 * a)),
        },
        RootDisplay {
            exact: roots.1,
            numeric: Some((-b + sqrt_disc) / (2.0 * a)),
        },
    ])
}

#[derive(Debug, Clone, Copy)]
struct SqrtPart {
    coeff: i64,
    radicand: u64,
}

fn simplify_sqrt(value: u64) -> SqrtPart {
    if value == 0 {
        return SqrtPart {
            coeff: 0,
            radicand: 1,
        };
    }
    let mut coeff = 1u64;
    let mut radicand = value;
    let mut factor = 2u64;
    while factor * factor <= radicand {
        let square = factor * factor;
        while radicand % square == 0 {
            radicand /= square;
            coeff *= factor;
        }
        factor += 1;
    }
    SqrtPart {
        coeff: coeff as i64,
        radicand,
    }
}

fn format_quadratic_roots(constant: i64, sqrt: SqrtPart, denom: i64) -> (String, String) {
    let sign = if denom < 0 { -1 } else { 1 };
    let mut constant = constant * sign;
    let mut sqrt_coeff = sqrt.coeff * sign;
    let mut denom = denom.abs();

    let gcd = gcd3(constant.abs(), sqrt_coeff.abs(), denom.abs()).max(1);
    constant /= gcd;
    sqrt_coeff /= gcd;
    denom /= gcd;

    let sqrt_text = format_sqrt_part(sqrt_coeff.abs(), sqrt.radicand);
    let minus = combine_root_terms(constant, "-", &sqrt_text, denom);
    let plus = combine_root_terms(constant, "+", &sqrt_text, denom);
    (minus, plus)
}

fn combine_root_terms(constant: i64, op: &str, sqrt_text: &str, denom: i64) -> String {
    let numerator = if constant == 0 {
        if op == "-" {
            format!("-{sqrt_text}")
        } else {
            sqrt_text.to_string()
        }
    } else {
        format!("{constant} {op} {sqrt_text}")
    };

    if denom == 1 {
        numerator
    } else {
        format!("({numerator}) / {denom}")
    }
}

fn format_sqrt_part(coeff: i64, radicand: u64) -> String {
    if radicand == 1 {
        coeff.to_string()
    } else if coeff == 1 {
        format!("sqrt({radicand})")
    } else {
        format!("{coeff}sqrt({radicand})")
    }
}

pub fn format_exact_number(value: f64) -> String {
    if let Some(int) = near_i64(value) {
        int.to_string()
    } else {
        format_number(value)
    }
}

pub fn format_number(value: f64) -> String {
    let value = clean_zero(value);
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_992.0 {
        return format!("{value:.0}");
    }
    let abs = value.abs();
    let raw = if abs != 0.0 && !(1e-6..1e12).contains(&abs) {
        format!("{value:.12e}")
    } else {
        format!("{value:.12}")
    };
    trim_float(raw)
}

pub fn format_pi_multiple(value: f64) -> String {
    let ratio = value / std::f64::consts::PI;
    if close(ratio, 0.0) {
        return "0".to_string();
    }
    if let Some(int) = near_i64(ratio) {
        return match int {
            1 => "pi".to_string(),
            -1 => "-pi".to_string(),
            _ => format!("{int}pi"),
        };
    }
    let doubled = ratio * 2.0;
    if let Some(int) = near_i64(doubled) {
        return match int {
            1 => "pi/2".to_string(),
            -1 => "-pi/2".to_string(),
            _ if int % 2 == 0 => format_pi_multiple((int / 2) as f64 * std::f64::consts::PI),
            _ => format!("{int}pi/2"),
        };
    }
    format_number(value)
}

pub fn relation_fallback(relation: &Relation) -> String {
    format!(
        "{} {} {}",
        format_expr(&relation.left),
        relation.op.label(),
        format_expr(&relation.right)
    )
}

fn trim_float(mut value: String) -> String {
    if let Some((head, exp)) = value.split_once('e') {
        let mut head = head.to_string();
        trim_decimal_tail(&mut head);
        return format!("{head}e{exp}");
    }
    trim_decimal_tail(&mut value);
    value
}

fn trim_decimal_tail(value: &mut String) {
    if value.contains('.') {
        while value.ends_with('0') {
            value.pop();
        }
        if value.ends_with('.') {
            value.pop();
        }
    }
}

fn near_i64(value: f64) -> Option<i64> {
    if !value.is_finite() {
        return None;
    }
    let rounded = value.round();
    if (value - rounded).abs() <= EPS && rounded >= i64::MIN as f64 && rounded <= i64::MAX as f64 {
        Some(rounded as i64)
    } else {
        None
    }
}

fn is_zero_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::Number(value) if close(*value, 0.0))
}

fn clean_zero(value: f64) -> f64 {
    if value.abs() <= EPS {
        0.0
    } else {
        value
    }
}

fn close(lhs: f64, rhs: f64) -> bool {
    (lhs - rhs).abs() <= EPS * lhs.abs().max(rhs.abs()).max(1.0)
}

fn gcd3(a: i64, b: i64, c: i64) -> i64 {
    gcd(gcd(a, b), c)
}

fn gcd(mut a: i64, mut b: i64) -> i64 {
    while b != 0 {
        let next = a % b;
        a = b;
        b = next;
    }
    a.abs()
}

const EPS: f64 = 1e-10;
