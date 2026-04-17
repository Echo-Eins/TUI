use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Variable(String),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Function {
        name: String,
        args: Vec<Expr>,
    },
}

impl Expr {
    pub fn variables(&self) -> BTreeSet<String> {
        let mut vars = BTreeSet::new();
        self.collect_variables(&mut vars);
        vars
    }

    fn collect_variables(&self, vars: &mut BTreeSet<String>) {
        match self {
            Self::Number(_) => {}
            Self::Variable(name) => {
                if !is_constant(name) {
                    vars.insert(name.clone());
                }
            }
            Self::Unary { expr, .. } => expr.collect_variables(vars),
            Self::Binary { left, right, .. } => {
                left.collect_variables(vars);
                right.collect_variables(vars);
            }
            Self::Function { args, .. } => {
                for arg in args {
                    arg.collect_variables(vars);
                }
            }
        }
    }

    pub fn eval(&self, ctx: &EvalContext) -> Result<f64, MathError> {
        match self {
            Self::Number(value) => Ok(*value),
            Self::Variable(name) => ctx
                .resolve(name)
                .ok_or_else(|| MathError::new(format!("unknown variable '{name}'"))),
            Self::Unary { op, expr } => {
                let value = expr.eval(ctx)?;
                match op {
                    UnaryOp::Positive => Ok(value),
                    UnaryOp::Negative => Ok(-value),
                }
            }
            Self::Binary { op, left, right } => {
                let lhs = left.eval(ctx)?;
                let rhs = right.eval(ctx)?;
                eval_binary(*op, lhs, rhs)
            }
            Self::Function { name, args } => {
                let values = args
                    .iter()
                    .map(|arg| arg.eval(ctx))
                    .collect::<Result<Vec<_>, _>>()?;
                eval_function(name, &values)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Positive,
    Negative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    Power,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MathError {
    pub message: String,
    pub position: Option<usize>,
}

impl MathError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            position: None,
        }
    }

    pub fn at(message: impl Into<String>, position: usize) -> Self {
        Self {
            message: message.into(),
            position: Some(position),
        }
    }

    pub fn with_input(&self, input: &str) -> String {
        let Some(position) = self.position else {
            return self.message.clone();
        };

        let caret_offset = input
            .char_indices()
            .take_while(|(idx, _)| *idx < position)
            .map(|(_, ch)| if ch == '\t' { '\t' } else { ' ' })
            .collect::<String>();

        format!("{}\n{}\n{}^", self.message, input, caret_offset)
    }
}

impl fmt::Display for MathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for MathError {}

#[derive(Debug, Clone, Default)]
pub struct EvalContext {
    variables: BTreeMap<String, f64>,
}

impl EvalContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_variables(variables: BTreeMap<String, f64>) -> Self {
        Self { variables }
    }

    pub fn set(&mut self, name: impl Into<String>, value: f64) {
        self.variables.insert(name.into(), value);
    }

    pub fn resolve(&self, name: &str) -> Option<f64> {
        self.variables
            .get(name)
            .copied()
            .or_else(|| constant_value(name))
    }
}

pub fn parse_expression(input: &str) -> Result<Expr, MathError> {
    let tokens = Lexer::new(input).tokenize()?;
    let mut parser = Parser::new(tokens);
    let expr = parser.parse_expression(0)?;
    parser.expect_end()?;
    Ok(expr)
}

pub fn is_valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

pub fn is_reserved_identifier(name: &str) -> bool {
    is_constant(name) || is_function(name) || name == "ans"
}

fn eval_binary(op: BinaryOp, lhs: f64, rhs: f64) -> Result<f64, MathError> {
    match op {
        BinaryOp::Add => Ok(lhs + rhs),
        BinaryOp::Subtract => Ok(lhs - rhs),
        BinaryOp::Multiply => Ok(lhs * rhs),
        BinaryOp::Divide => {
            if rhs == 0.0 {
                Err(MathError::new("division by zero"))
            } else {
                Ok(lhs / rhs)
            }
        }
        BinaryOp::Remainder => {
            if rhs == 0.0 {
                Err(MathError::new("remainder by zero"))
            } else {
                Ok(lhs % rhs)
            }
        }
        BinaryOp::Power => Ok(lhs.powf(rhs)),
    }
}

fn eval_function(name: &str, args: &[f64]) -> Result<f64, MathError> {
    let name = name.to_ascii_lowercase();
    match name.as_str() {
        "sin" => unary(args, name.as_str(), f64::sin),
        "cos" => unary(args, name.as_str(), f64::cos),
        "tan" => unary(args, name.as_str(), f64::tan),
        "asin" => unary(args, name.as_str(), f64::asin),
        "acos" => unary(args, name.as_str(), f64::acos),
        "atan" => unary(args, name.as_str(), f64::atan),
        "sinh" => unary(args, name.as_str(), f64::sinh),
        "cosh" => unary(args, name.as_str(), f64::cosh),
        "tanh" => unary(args, name.as_str(), f64::tanh),
        "asinh" => unary(args, name.as_str(), f64::asinh),
        "acosh" => unary(args, name.as_str(), f64::acosh),
        "atanh" => unary(args, name.as_str(), f64::atanh),
        "sec" => unary(args, name.as_str(), |x| 1.0 / x.cos()),
        "csc" => unary(args, name.as_str(), |x| 1.0 / x.sin()),
        "cot" => unary(args, name.as_str(), |x| 1.0 / x.tan()),
        "ln" => unary(args, name.as_str(), f64::ln),
        "log" => match args {
            [value] => Ok(value.ln()),
            [value, base] => Ok(value.log(*base)),
            _ => Err(arity_error(name.as_str(), "one value or value, base")),
        },
        "log10" => unary(args, name.as_str(), f64::log10),
        "log2" => unary(args, name.as_str(), f64::log2),
        "exp" => unary(args, name.as_str(), f64::exp),
        "sqrt" => unary(args, name.as_str(), f64::sqrt),
        "cbrt" => unary(args, name.as_str(), f64::cbrt),
        "abs" => unary(args, name.as_str(), f64::abs),
        "floor" => unary(args, name.as_str(), f64::floor),
        "ceil" => unary(args, name.as_str(), f64::ceil),
        "round" => unary(args, name.as_str(), f64::round),
        "trunc" => unary(args, name.as_str(), f64::trunc),
        "fract" => unary(args, name.as_str(), f64::fract),
        "sign" | "signum" => unary(args, name.as_str(), f64::signum),
        "deg" | "degrees" => unary(args, name.as_str(), f64::to_degrees),
        "rad" | "radians" => unary(args, name.as_str(), f64::to_radians),
        "pow" => binary(args, name.as_str(), f64::powf),
        "root" => binary(args, name.as_str(), |value, degree| {
            value.powf(1.0 / degree)
        }),
        "hypot" => binary(args, name.as_str(), f64::hypot),
        "atan2" => binary(args, name.as_str(), f64::atan2),
        "min" => variadic(args, name.as_str(), f64::min),
        "max" => variadic(args, name.as_str(), f64::max),
        "clamp" => match args {
            [value, min, max] => Ok(value.clamp(*min, *max)),
            _ => Err(arity_error(name.as_str(), "value, min, max")),
        },
        _ => Err(MathError::new(format!("unknown function '{name}'"))),
    }
}

fn unary(args: &[f64], name: &str, f: impl FnOnce(f64) -> f64) -> Result<f64, MathError> {
    match args {
        [value] => Ok(f(*value)),
        _ => Err(arity_error(name, "one argument")),
    }
}

fn binary(args: &[f64], name: &str, f: impl FnOnce(f64, f64) -> f64) -> Result<f64, MathError> {
    match args {
        [lhs, rhs] => Ok(f(*lhs, *rhs)),
        _ => Err(arity_error(name, "two arguments")),
    }
}

fn variadic(args: &[f64], name: &str, f: impl Fn(f64, f64) -> f64) -> Result<f64, MathError> {
    let Some((first, rest)) = args.split_first() else {
        return Err(arity_error(name, "one or more arguments"));
    };
    Ok(rest.iter().fold(*first, |acc, value| f(acc, *value)))
}

fn arity_error(name: &str, expected: &str) -> MathError {
    MathError::new(format!("function '{name}' expects {expected}"))
}

fn constant_value(name: &str) -> Option<f64> {
    match name.to_ascii_lowercase().as_str() {
        "pi" => Some(std::f64::consts::PI),
        "tau" => Some(std::f64::consts::TAU),
        "e" => Some(std::f64::consts::E),
        "phi" => Some(1.618_033_988_749_895),
        "inf" | "infinity" => Some(f64::INFINITY),
        "nan" => Some(f64::NAN),
        _ => None,
    }
}

fn is_constant(name: &str) -> bool {
    constant_value(name).is_some()
}

fn is_function(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "sin"
            | "cos"
            | "tan"
            | "asin"
            | "acos"
            | "atan"
            | "sinh"
            | "cosh"
            | "tanh"
            | "asinh"
            | "acosh"
            | "atanh"
            | "sec"
            | "csc"
            | "cot"
            | "ln"
            | "log"
            | "log10"
            | "log2"
            | "exp"
            | "sqrt"
            | "cbrt"
            | "abs"
            | "floor"
            | "ceil"
            | "round"
            | "trunc"
            | "fract"
            | "sign"
            | "signum"
            | "deg"
            | "degrees"
            | "rad"
            | "radians"
            | "pow"
            | "root"
            | "hypot"
            | "atan2"
            | "min"
            | "max"
            | "clamp"
    )
}

#[derive(Debug, Clone, PartialEq)]
struct Token {
    kind: TokenKind,
    start: usize,
    end: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum TokenKind {
    Number(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    LParen,
    RParen,
    Comma,
    End,
}

struct Lexer<'a> {
    input: &'a str,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a str) -> Self {
        Self { input }
    }

    fn tokenize(self) -> Result<Vec<Token>, MathError> {
        let mut tokens = Vec::new();
        let mut chars = self.input.char_indices().peekable();

        while let Some((idx, ch)) = chars.peek().copied() {
            match ch {
                ch if ch.is_whitespace() => {
                    chars.next();
                }
                '0'..='9' | '.' => {
                    tokens.push(read_number(self.input, &mut chars)?);
                }
                ch if ch.is_ascii_alphabetic() || ch == '_' => {
                    tokens.push(read_ident(&mut chars));
                }
                '+' => {
                    chars.next();
                    tokens.push(simple(TokenKind::Plus, idx, ch));
                }
                '-' => {
                    chars.next();
                    tokens.push(simple(TokenKind::Minus, idx, ch));
                }
                '*' => {
                    chars.next();
                    tokens.push(simple(TokenKind::Star, idx, ch));
                }
                '/' => {
                    chars.next();
                    tokens.push(simple(TokenKind::Slash, idx, ch));
                }
                '%' => {
                    chars.next();
                    tokens.push(simple(TokenKind::Percent, idx, ch));
                }
                '^' => {
                    chars.next();
                    tokens.push(simple(TokenKind::Caret, idx, ch));
                }
                '(' => {
                    chars.next();
                    tokens.push(simple(TokenKind::LParen, idx, ch));
                }
                ')' => {
                    chars.next();
                    tokens.push(simple(TokenKind::RParen, idx, ch));
                }
                ',' => {
                    chars.next();
                    tokens.push(simple(TokenKind::Comma, idx, ch));
                }
                _ => {
                    return Err(MathError::at(format!("unexpected character '{ch}'"), idx));
                }
            }
        }

        tokens.push(Token {
            kind: TokenKind::End,
            start: self.input.len(),
            end: self.input.len(),
        });
        Ok(tokens)
    }
}

fn simple(kind: TokenKind, idx: usize, ch: char) -> Token {
    Token {
        kind,
        start: idx,
        end: idx + ch.len_utf8(),
    }
}

fn read_ident(chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>) -> Token {
    let (start, first) = chars.next().expect("identifier starts at current char");
    let mut end = start + first.len_utf8();
    let mut ident = String::from(first);

    while let Some((idx, ch)) = chars.peek().copied() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            chars.next();
            ident.push(ch);
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }

    Token {
        kind: TokenKind::Ident(ident.to_ascii_lowercase()),
        start,
        end,
    }
}

fn read_number(
    input: &str,
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
) -> Result<Token, MathError> {
    let (start, _) = chars
        .peek()
        .copied()
        .expect("number starts at current char");
    let mut end = start;
    let mut saw_digit = false;
    let mut saw_dot = false;

    while let Some((idx, ch)) = chars.peek().copied() {
        match ch {
            '0'..='9' => {
                saw_digit = true;
                chars.next();
                end = idx + ch.len_utf8();
            }
            '.' if !saw_dot => {
                saw_dot = true;
                chars.next();
                end = idx + ch.len_utf8();
            }
            _ => break,
        }
    }

    if !saw_digit {
        return Err(MathError::at("expected digit after decimal point", start));
    }

    if let Some((idx, 'e' | 'E')) = chars.peek().copied() {
        let mut lookahead = chars.clone();
        lookahead.next();
        if let Some((_, '+' | '-')) = lookahead.peek().copied() {
            lookahead.next();
        }

        let mut exp_end = idx + 1;
        let mut exp_digits = 0usize;
        while let Some((digit_idx, digit)) = lookahead.peek().copied() {
            if digit.is_ascii_digit() {
                lookahead.next();
                exp_end = digit_idx + digit.len_utf8();
                exp_digits += 1;
            } else {
                break;
            }
        }

        if exp_digits > 0 {
            while chars.peek().is_some_and(|(pos, _)| *pos < exp_end) {
                chars.next();
            }
            end = exp_end;
        }
    }

    let literal = &input[start..end];
    let value = literal
        .parse::<f64>()
        .map_err(|_| MathError::at(format!("invalid number '{literal}'"), start))?;

    Ok(Token {
        kind: TokenKind::Number(value),
        start,
        end,
    })
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn parse_expression(&mut self, min_bp: u8) -> Result<Expr, MathError> {
        let mut lhs = self.parse_prefix()?;

        loop {
            if self.is_implicit_multiply() {
                let (left_bp, right_bp) = (5, 6);
                if left_bp < min_bp {
                    break;
                }
                let rhs = self.parse_expression(right_bp)?;
                lhs = Expr::Binary {
                    op: BinaryOp::Multiply,
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                };
                continue;
            }

            let Some((op, left_bp, right_bp)) = self.infix_binding_power() else {
                break;
            };
            if left_bp < min_bp {
                break;
            }

            self.bump();
            let rhs = self.parse_expression(right_bp)?;
            lhs = Expr::Binary {
                op,
                left: Box::new(lhs),
                right: Box::new(rhs),
            };
        }

        Ok(lhs)
    }

    fn parse_prefix(&mut self) -> Result<Expr, MathError> {
        let token = self.bump().clone();
        match token.kind {
            TokenKind::Number(value) => Ok(Expr::Number(value)),
            TokenKind::Ident(name) => {
                if self.consume(TokenKind::LParen) {
                    let mut args = Vec::new();
                    if !self.check(TokenKind::RParen) {
                        loop {
                            args.push(self.parse_expression(0)?);
                            if !self.consume(TokenKind::Comma) {
                                break;
                            }
                        }
                    }
                    self.expect(TokenKind::RParen, "expected ')' after function arguments")?;
                    Ok(Expr::Function { name, args })
                } else {
                    Ok(Expr::Variable(name))
                }
            }
            TokenKind::Plus => Ok(Expr::Unary {
                op: UnaryOp::Positive,
                expr: Box::new(self.parse_expression(7)?),
            }),
            TokenKind::Minus => Ok(Expr::Unary {
                op: UnaryOp::Negative,
                expr: Box::new(self.parse_expression(7)?),
            }),
            TokenKind::LParen => {
                let expr = self.parse_expression(0)?;
                self.expect(TokenKind::RParen, "expected ')'")?;
                Ok(expr)
            }
            TokenKind::End => Err(MathError::at("expected expression", token.start)),
            _ => Err(MathError::at("expected expression", token.start)),
        }
    }

    fn infix_binding_power(&self) -> Option<(BinaryOp, u8, u8)> {
        match self.current().kind {
            TokenKind::Plus => Some((BinaryOp::Add, 3, 4)),
            TokenKind::Minus => Some((BinaryOp::Subtract, 3, 4)),
            TokenKind::Star => Some((BinaryOp::Multiply, 5, 6)),
            TokenKind::Slash => Some((BinaryOp::Divide, 5, 6)),
            TokenKind::Percent => Some((BinaryOp::Remainder, 5, 6)),
            TokenKind::Caret => Some((BinaryOp::Power, 8, 8)),
            _ => None,
        }
    }

    fn is_implicit_multiply(&self) -> bool {
        matches!(
            self.current().kind,
            TokenKind::Number(_) | TokenKind::Ident(_) | TokenKind::LParen
        )
    }

    fn expect_end(&self) -> Result<(), MathError> {
        if matches!(self.current().kind, TokenKind::End) {
            Ok(())
        } else {
            Err(MathError::at(
                "unexpected token after expression",
                self.current().start,
            ))
        }
    }

    fn expect(&mut self, expected: TokenKind, message: &'static str) -> Result<(), MathError> {
        if self.consume(expected) {
            Ok(())
        } else {
            Err(MathError::at(message, self.current().start))
        }
    }

    fn consume(&mut self, expected: TokenKind) -> bool {
        if self.check(expected) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn check(&self, expected: TokenKind) -> bool {
        std::mem::discriminant(&self.current().kind) == std::mem::discriminant(&expected)
    }

    fn bump(&mut self) -> &Token {
        let token = &self.tokens[self.pos];
        if !matches!(token.kind, TokenKind::End) {
            self.pos += 1;
        }
        token
    }

    fn current(&self) -> &Token {
        &self.tokens[self.pos]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(input: &str) -> f64 {
        parse_expression(input)
            .unwrap()
            .eval(&EvalContext::new())
            .unwrap()
    }

    #[test]
    fn parser_respects_precedence_and_power_associativity() {
        assert_eq!(eval("2 + 3 * 4"), 14.0);
        assert_eq!(eval("2^3^2"), 512.0);
        assert_eq!(eval("-2^2"), -4.0);
    }

    #[test]
    fn parser_supports_functions_constants_and_implicit_multiply() {
        let value = eval("2pi + sin(pi / 2)");
        assert!((value - (2.0 * std::f64::consts::PI + 1.0)).abs() < 1e-12);
    }
}
