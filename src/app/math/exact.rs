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
    solve_exact_trig(relation, variable, variables, domain)
        .or_else(|| solve_exact_radical(relation, variable, variables, domain))
        .or_else(|| solve_exact_polynomial(relation, variable, variables, domain))
        .or_else(|| solve_exact_symbolic_polynomial(relation, variable, variables, domain))
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

fn solve_exact_symbolic_polynomial(
    relation: &Relation,
    variable: &str,
    variables: &BTreeMap<String, f64>,
    domain: Option<(f64, f64)>,
) -> Option<ExactSolveReport> {
    if relation.op != RelationOp::Equal {
        return None;
    }

    let left = symbolic_polynomial_coeffs(&relation.left, variable, variables)?;
    let right = symbolic_polynomial_coeffs(&relation.right, variable, variables)?;
    let coeffs = left.sub(&right);
    let [c, b, a] = coeffs.coeffs;

    if !sym_is_zero(&a) {
        let roots = symbolic_quadratic_roots(variable, &a, &b, &c);
        return Some(ExactSolveReport {
            variable: variable.to_string(),
            method: "quadratic-symbolic",
            status: "symbolic",
            exact_lines: roots,
            numeric_lines: vec![
                "Symbolic form assumes non-zero denominators; assign parameter values for degenerate cases."
                    .to_string(),
                "Assign parameters with ':calc let ...' to request numeric approximation."
                    .to_string(),
            ],
            domain,
        });
    }

    if !sym_is_zero(&b) {
        return Some(ExactSolveReport {
            variable: variable.to_string(),
            method: "linear-symbolic",
            status: "symbolic",
            exact_lines: vec![format!("{variable} = {}", sym_div(&sym_neg(&c), &b))],
            numeric_lines: vec![
                "Symbolic form assumes non-zero denominators; assign parameter values for degenerate cases."
                    .to_string(),
                "Assign parameters with ':calc let ...' to request numeric approximation."
                    .to_string(),
            ],
            domain,
        });
    }

    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SymbolicPoly {
    coeffs: [String; 3],
}

impl SymbolicPoly {
    fn zero() -> Self {
        Self {
            coeffs: std::array::from_fn(|_| "0".to_string()),
        }
    }

    fn constant(value: impl Into<String>) -> Self {
        let mut poly = Self::zero();
        poly.coeffs[0] = value.into();
        poly
    }

    fn target_variable() -> Self {
        let mut poly = Self::zero();
        poly.coeffs[1] = "1".to_string();
        poly
    }

    fn add(&self, rhs: &Self) -> Self {
        Self {
            coeffs: std::array::from_fn(|idx| sym_add(&self.coeffs[idx], &rhs.coeffs[idx])),
        }
    }

    fn sub(&self, rhs: &Self) -> Self {
        Self {
            coeffs: std::array::from_fn(|idx| sym_sub(&self.coeffs[idx], &rhs.coeffs[idx])),
        }
    }

    fn neg(&self) -> Self {
        Self {
            coeffs: std::array::from_fn(|idx| sym_neg(&self.coeffs[idx])),
        }
    }

    fn mul(&self, rhs: &Self) -> Option<Self> {
        let mut out: [String; 3] = std::array::from_fn(|_| "0".to_string());
        for lhs_degree in 0..=2 {
            for rhs_degree in 0..=2 {
                let degree = lhs_degree + rhs_degree;
                let product = sym_mul(&self.coeffs[lhs_degree], &rhs.coeffs[rhs_degree]);
                if sym_is_zero(&product) {
                    continue;
                }
                if degree > 2 {
                    return None;
                }
                out[degree] = sym_add(&out[degree], &product);
            }
        }
        Some(Self { coeffs: out })
    }

    fn div_scalar(&self, divisor: &str) -> Option<Self> {
        if sym_is_zero(divisor) {
            return None;
        }
        Some(Self {
            coeffs: std::array::from_fn(|idx| sym_div(&self.coeffs[idx], divisor)),
        })
    }

    fn pow(&self, power: i32) -> Option<Self> {
        match power {
            0 => Some(Self::constant("1")),
            1 => Some(self.clone()),
            2 => self.mul(self),
            _ => None,
        }
    }
}

fn symbolic_polynomial_coeffs(
    expr: &Expr,
    variable: &str,
    variables: &BTreeMap<String, f64>,
) -> Option<SymbolicPoly> {
    match expr {
        Expr::Number(value) => Some(SymbolicPoly::constant(format_exact_number(*value))),
        Expr::Variable(name) if name == variable => Some(SymbolicPoly::target_variable()),
        Expr::Variable(name) => variables
            .get(name)
            .map(|value| SymbolicPoly::constant(format_exact_number(*value)))
            .or_else(|| Some(SymbolicPoly::constant(name.clone()))),
        Expr::Unary { op, expr } => {
            let poly = symbolic_polynomial_coeffs(expr, variable, variables)?;
            match op {
                UnaryOp::Positive => Some(poly),
                UnaryOp::Negative => Some(poly.neg()),
            }
        }
        Expr::Binary { op, left, right } => match op {
            BinaryOp::Add => {
                let lhs = symbolic_polynomial_coeffs(left, variable, variables)?;
                let rhs = symbolic_polynomial_coeffs(right, variable, variables)?;
                Some(lhs.add(&rhs))
            }
            BinaryOp::Subtract => {
                let lhs = symbolic_polynomial_coeffs(left, variable, variables)?;
                let rhs = symbolic_polynomial_coeffs(right, variable, variables)?;
                Some(lhs.sub(&rhs))
            }
            BinaryOp::Multiply => {
                let lhs = symbolic_polynomial_coeffs(left, variable, variables)?;
                let rhs = symbolic_polynomial_coeffs(right, variable, variables)?;
                lhs.mul(&rhs)
            }
            BinaryOp::Divide => {
                let lhs = symbolic_polynomial_coeffs(left, variable, variables)?;
                let divisor = symbolic_constant_expr(right, variable, variables)?;
                lhs.div_scalar(&divisor)
            }
            BinaryOp::Power => {
                if !contains_target_variable(expr, variable) {
                    return symbolic_constant_expr(expr, variable, variables)
                        .map(SymbolicPoly::constant);
                }
                let exponent = constant_value(right, variables)?;
                if !close(exponent, exponent.round()) {
                    return None;
                }
                let base = symbolic_polynomial_coeffs(left, variable, variables)?;
                base.pow(exponent.round() as i32)
            }
            BinaryOp::Remainder => None,
        },
        Expr::Function { .. } => {
            symbolic_constant_expr(expr, variable, variables).map(SymbolicPoly::constant)
        }
    }
}

fn symbolic_constant_expr(
    expr: &Expr,
    variable: &str,
    variables: &BTreeMap<String, f64>,
) -> Option<String> {
    if contains_target_variable(expr, variable) {
        return None;
    }
    constant_value(expr, variables)
        .map(format_exact_number)
        .or_else(|| Some(format_expr(expr)))
}

fn contains_target_variable(expr: &Expr, variable: &str) -> bool {
    expr.variables().contains(variable)
}

fn symbolic_quadratic_roots(variable: &str, a: &str, b: &str, c: &str) -> Vec<String> {
    let discriminant = sym_sub(&sym_pow2(b), &sym_mul(&sym_mul("4", a), c));
    let radical = format!("sqrt({discriminant})");
    let negative_b = sym_neg(b);
    let denominator = sym_mul("2", a);
    vec![
        format!(
            "{variable} = {}",
            sym_div(&sym_sub(&negative_b, &radical), &denominator)
        ),
        format!(
            "{variable} = {}",
            sym_div(&sym_add(&negative_b, &radical), &denominator)
        ),
    ]
}

fn sym_is_zero(value: &str) -> bool {
    matches!(value.trim(), "0" | "-0")
}

fn sym_is_one(value: &str) -> bool {
    value.trim() == "1"
}

fn sym_is_negative_one(value: &str) -> bool {
    value.trim() == "-1"
}

fn sym_add(lhs: &str, rhs: &str) -> String {
    if sym_is_zero(lhs) {
        return rhs.to_string();
    }
    if sym_is_zero(rhs) {
        return lhs.to_string();
    }
    if let Some(rest) = rhs.strip_prefix('-') {
        return sym_sub(lhs, rest);
    }
    format!("{lhs} + {rhs}")
}

fn sym_sub(lhs: &str, rhs: &str) -> String {
    if sym_is_zero(rhs) {
        return lhs.to_string();
    }
    if sym_is_zero(lhs) {
        return sym_neg(rhs);
    }
    if let Some(rest) = rhs.strip_prefix('-') {
        return sym_add(lhs, rest);
    }
    format!("{lhs} - {rhs}")
}

fn sym_neg(value: &str) -> String {
    if sym_is_zero(value) {
        return "0".to_string();
    }
    if let Some(rest) = value.strip_prefix('-') {
        if !rest.contains(' ') {
            return rest.to_string();
        }
    }
    format!("-{}", sym_factor(value))
}

fn sym_mul(lhs: &str, rhs: &str) -> String {
    if sym_is_zero(lhs) || sym_is_zero(rhs) {
        return "0".to_string();
    }
    if sym_is_one(lhs) {
        return rhs.to_string();
    }
    if sym_is_one(rhs) {
        return lhs.to_string();
    }
    if sym_is_negative_one(lhs) {
        return sym_neg(rhs);
    }
    if sym_is_negative_one(rhs) {
        return sym_neg(lhs);
    }
    if lhs == rhs {
        return sym_pow2(lhs);
    }
    format!("{}*{}", sym_factor(lhs), sym_factor(rhs))
}

fn sym_div(lhs: &str, rhs: &str) -> String {
    if sym_is_zero(lhs) {
        return "0".to_string();
    }
    if sym_is_one(rhs) {
        return lhs.to_string();
    }
    format!("{} / {}", sym_group(lhs), sym_group(rhs))
}

fn sym_pow2(value: &str) -> String {
    if sym_is_zero(value) {
        return "0".to_string();
    }
    if sym_is_one(value) || sym_is_negative_one(value) {
        return "1".to_string();
    }
    format!("{}^2", sym_factor(value))
}

fn sym_factor(value: &str) -> String {
    if is_symbolic_atom(value) {
        value.to_string()
    } else {
        format!("({value})")
    }
}

fn sym_group(value: &str) -> String {
    if is_symbolic_atom(value)
        || value
            .strip_prefix('-')
            .is_some_and(|rest| !rest.contains(' '))
    {
        value.to_string()
    } else {
        format!("({value})")
    }
}

fn is_symbolic_atom(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    if value.starts_with("sqrt(") && value.ends_with(')') {
        return true;
    }
    value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '^' | '/'))
}

fn solve_exact_trig(
    relation: &Relation,
    variable: &str,
    variables: &BTreeMap<String, f64>,
    domain: Option<(f64, f64)>,
) -> Option<ExactSolveReport> {
    if let Some(trig) = match_trig_constant_relation(relation, variable, variables) {
        if let Some(report) = trig_constant_report(variable, trig, domain) {
            return Some(report);
        }
    }

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

#[derive(Debug, Clone, Copy)]
struct TrigConstantRelation {
    func: &'static str,
    op: RelationOp,
    value: f64,
}

#[derive(Debug, Clone, Copy)]
struct TrigPointFamily {
    base: f64,
    period: f64,
    line: &'static str,
}

fn match_trig_constant_relation(
    relation: &Relation,
    variable: &str,
    variables: &BTreeMap<String, f64>,
) -> Option<TrigConstantRelation> {
    if let Some(value) = constant_value(&relation.right, variables) {
        let func = match_trig_variable(&relation.left, variable)?;
        return Some(TrigConstantRelation {
            func,
            op: relation.op,
            value,
        });
    }

    if let Some(value) = constant_value(&relation.left, variables) {
        let func = match_trig_variable(&relation.right, variable)?;
        return Some(TrigConstantRelation {
            func,
            op: reverse_relation_op(relation.op),
            value,
        });
    }

    None
}

fn trig_constant_report(
    variable: &str,
    trig: TrigConstantRelation,
    domain: Option<(f64, f64)>,
) -> Option<ExactSolveReport> {
    if trig.op != RelationOp::Equal {
        return None;
    }

    let family = trig_point_family(trig.func, trig.value)?;
    if let Some((min, max)) = domain {
        let points = trig_point_domain_lines(variable, family, min, max)?;
        return Some(ExactSolveReport {
            variable: variable.to_string(),
            method: "trig-domain",
            status: "exact",
            numeric_lines: points.iter().map(|line| line.numeric.clone()).collect(),
            exact_lines: points.into_iter().map(|line| line.exact).collect(),
            domain,
        });
    }

    Some(ExactSolveReport {
        variable: variable.to_string(),
        method: "trig-family",
        status: "exact-family",
        exact_lines: vec![format!("{variable} = {}", family.line)],
        numeric_lines: vec![
            "Use a bounded domain, for example 'from -2pi..2pi', for numeric windows.".to_string(),
        ],
        domain,
    })
}

fn trig_point_family(func: &str, value: f64) -> Option<TrigPointFamily> {
    let pi = std::f64::consts::PI;
    let two_pi = std::f64::consts::TAU;
    match func {
        "sin" if close(value, 0.0) => Some(TrigPointFamily {
            base: 0.0,
            period: pi,
            line: "k*pi, k in Z",
        }),
        "sin" if close(value, 1.0) => Some(TrigPointFamily {
            base: pi / 2.0,
            period: two_pi,
            line: "pi/2 + 2k*pi, k in Z",
        }),
        "sin" if close(value, -1.0) => Some(TrigPointFamily {
            base: -pi / 2.0,
            period: two_pi,
            line: "-pi/2 + 2k*pi, k in Z",
        }),
        "cos" if close(value, 0.0) => Some(TrigPointFamily {
            base: pi / 2.0,
            period: pi,
            line: "pi/2 + k*pi, k in Z",
        }),
        "cos" if close(value, 1.0) => Some(TrigPointFamily {
            base: 0.0,
            period: two_pi,
            line: "2k*pi, k in Z",
        }),
        "cos" if close(value, -1.0) => Some(TrigPointFamily {
            base: pi,
            period: two_pi,
            line: "pi + 2k*pi, k in Z",
        }),
        _ => None,
    }
}

fn trig_point_domain_lines(
    variable: &str,
    family: TrigPointFamily,
    min: f64,
    max: f64,
) -> Option<Vec<TrigIntervalLine>> {
    let mut lines = Vec::new();
    let k_min = ((min - family.base) / family.period).floor() as i64 - 2;
    let k_max = ((max - family.base) / family.period).ceil() as i64 + 2;
    for k in k_min..=k_max {
        let value = family.base + k as f64 * family.period;
        if value >= min - EPS && value <= max + EPS {
            lines.push(TrigIntervalLine {
                exact: format!("{variable} = {}", format_pi_multiple(value)),
                numeric: format!("{variable} ~= {}", format_number(value)),
            });
        }
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines)
    }
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
