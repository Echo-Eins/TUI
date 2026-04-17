use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::app::console_state::OutputLine;
use crate::app::math::{
    convert_base, parse_expression, parse_relation, solve_relation, BaseConversion, EvalContext,
    Interval, MathError, Relation, SolveOptions, SolveReport,
};

use super::{
    ConsoleCommandResponse, ConsoleCommandSpec, ConsoleContext, ConsoleExtension,
    ConsoleExtensionRegistry, ExtensionKind, ExtensionMetadata,
};

pub(super) struct MathExtension {
    metadata: ExtensionMetadata,
    memory: Mutex<MathMemory>,
}

#[derive(Debug, Clone, Default)]
struct MathMemory {
    ans: Option<f64>,
    variables: BTreeMap<String, f64>,
}

impl MathMemory {
    fn evaluation_variables(&self) -> BTreeMap<String, f64> {
        let mut variables = self.variables.clone();
        if let Some(ans) = self.ans {
            variables.insert("ans".to_string(), ans);
        }
        variables
    }
}

impl MathExtension {
    pub(super) fn new() -> Self {
        Self {
            metadata: ExtensionMetadata {
                id: "math",
                title: "Math",
                version: "0.1.0",
                kind: ExtensionKind::Builtin,
                description:
                    "Engineering calculator, numeral base conversion, and shared math parser.",
                commands: vec![
                    ConsoleCommandSpec {
                        name: "base",
                        summary: "Convert integers between bases 2..16.",
                        usage: ":base <value> [from <base>] to <base|start..end|all>",
                        tags: &["math", "base", "conversion"],
                    },
                    ConsoleCommandSpec {
                        name: "calc",
                        summary: "Evaluate expressions and solve one-variable equations.",
                        usage: ":calc <expr> | :calc let <name> = <expr> | :calc solve <relation> [for x] [from a..b]",
                        tags: &["math", "calculator", "solver"],
                    },
                ],
                tags: &["math", "builtin"],
                permissions: &[],
            },
            memory: Mutex::new(MathMemory::default()),
        }
    }

    fn execute_base(&self, args: &[String]) -> ConsoleCommandResponse {
        let input = args.join(" ");
        match convert_base(input.trim()) {
            Ok(conversion) => ConsoleCommandResponse::ok(base_output(conversion)),
            Err(error) => ConsoleCommandResponse::error(error.message, 2),
        }
    }

    fn execute_calc(&self, args: &[String]) -> ConsoleCommandResponse {
        let input = args.join(" ");
        let input = input.trim();
        if input.is_empty() || input.eq_ignore_ascii_case("help") {
            return ConsoleCommandResponse::ok(calc_help());
        }

        if input.eq_ignore_ascii_case("vars") {
            return ConsoleCommandResponse::ok(self.variables_output());
        }

        if input.eq_ignore_ascii_case("clear") {
            let mut memory = self.lock_memory();
            memory.ans = None;
            memory.variables.clear();
            return ConsoleCommandResponse::ok(vec![OutputLine::system(
                "Calculator memory cleared.",
            )]);
        }

        if let Some(rest) = strip_prefix_ci(input, "let ") {
            return self.execute_let(rest.trim());
        }

        if let Some(rest) = strip_prefix_ci(input, "solve ") {
            return self.execute_solve(rest.trim());
        }

        match parse_relation(input) {
            Ok(Some(relation)) => self.execute_relation(input, relation),
            Ok(None) => self.execute_expression(input),
            Err(error) => math_error(input, error),
        }
    }

    fn execute_let(&self, input: &str) -> ConsoleCommandResponse {
        let Some((name, expr_input)) = input.split_once('=') else {
            return ConsoleCommandResponse::error("usage: :calc let <name> = <expr>", 2);
        };
        let name = name.trim().to_ascii_lowercase();
        if !crate::app::math::expr::is_valid_identifier(&name) {
            return ConsoleCommandResponse::error(format!("invalid variable name '{name}'"), 2);
        }
        if crate::app::math::expr::is_reserved_identifier(&name) {
            return ConsoleCommandResponse::error(format!("'{name}' is reserved"), 2);
        }

        let variables = self.lock_memory().evaluation_variables();
        let expr_input = expr_input.trim();
        let value = match parse_expression(expr_input)
            .and_then(|expr| expr.eval(&EvalContext::with_variables(variables)))
            .and_then(require_finite)
        {
            Ok(value) => value,
            Err(error) => return math_error(expr_input, error),
        };

        let mut memory = self.lock_memory();
        memory.variables.insert(name.clone(), value);
        memory.ans = Some(value);

        ConsoleCommandResponse::ok(vec![OutputLine::stdout(format!(
            "{name} = {}",
            format_number(value)
        ))])
    }

    fn execute_expression(&self, input: &str) -> ConsoleCommandResponse {
        let variables = self.lock_memory().evaluation_variables();
        let value = match parse_expression(input)
            .and_then(|expr| expr.eval(&EvalContext::with_variables(variables)))
            .and_then(require_finite)
        {
            Ok(value) => value,
            Err(error) => return math_error(input, error),
        };

        self.lock_memory().ans = Some(value);
        ConsoleCommandResponse::ok(vec![OutputLine::stdout(format_number(value))])
    }

    fn execute_relation(&self, input: &str, relation: Relation) -> ConsoleCommandResponse {
        let variables = self.lock_memory().evaluation_variables();
        let unknowns = unknown_variables(&relation, &variables);

        if unknowns.is_empty() {
            return match solve_relation(&relation, None, &variables) {
                Ok(SolveReport::Constant { holds }) => {
                    ConsoleCommandResponse::ok(vec![OutputLine::stdout(holds.to_string())])
                }
                Ok(report) => ConsoleCommandResponse::ok(solve_output(report)),
                Err(error) => math_error(input, error),
            };
        }

        self.execute_solve_with_relation(input, relation, SolveQueryOptions::default())
    }

    fn execute_solve(&self, input: &str) -> ConsoleCommandResponse {
        let (relation_text, options) = match parse_solve_query_options(input) {
            Ok(parsed) => parsed,
            Err(error) => return ConsoleCommandResponse::error(error.message, 2),
        };

        let relation = match parse_relation(relation_text) {
            Ok(Some(relation)) => relation,
            Ok(None) => {
                return ConsoleCommandResponse::error(
                    "solve expects a relation such as x^2 - 4 = 0 or sin(x) > 0",
                    2,
                )
            }
            Err(error) => return math_error(relation_text, error),
        };

        self.execute_solve_with_relation(relation_text, relation, options)
    }

    fn execute_solve_with_relation(
        &self,
        input: &str,
        relation: Relation,
        options: SolveQueryOptions,
    ) -> ConsoleCommandResponse {
        let variables = self.lock_memory().evaluation_variables();
        let unknowns = unknown_variables(&relation, &variables);

        let variable = match options.variable {
            Some(variable) => variable,
            None if unknowns.len() == 1 => unknowns[0].clone(),
            None => {
                return ConsoleCommandResponse::error(
                    format!(
                        "solve expects exactly one unknown variable, found {}; use 'for <name>'",
                        unknowns.len()
                    ),
                    2,
                )
            }
        };

        let unresolved = unknowns
            .iter()
            .filter(|name| *name != &variable)
            .cloned()
            .collect::<Vec<_>>();
        if !unresolved.is_empty() {
            return ConsoleCommandResponse::error(
                format!(
                    "unresolved variables: {}; assign them with ':calc let' or solve for one variable",
                    unresolved.join(", ")
                ),
                2,
            );
        }

        let (min, max) = match options.domain {
            Some(domain) => domain,
            None => (-100.0, 100.0),
        };
        let mut solve_options = SolveOptions::new(variable, min, max);
        solve_options.samples = options.samples.unwrap_or(2048);

        match solve_relation(&relation, Some(solve_options), &variables) {
            Ok(report) => ConsoleCommandResponse::ok(solve_output(report)),
            Err(error) => math_error(input, error),
        }
    }

    fn variables_output(&self) -> Vec<OutputLine> {
        let memory = self.lock_memory();
        let mut lines = Vec::new();
        if let Some(ans) = memory.ans {
            lines.push(OutputLine::stdout(format!("ans = {}", format_number(ans))));
        }
        for (name, value) in &memory.variables {
            lines.push(OutputLine::stdout(format!(
                "{name} = {}",
                format_number(*value)
            )));
        }
        if lines.is_empty() {
            lines.push(OutputLine::system("No calculator variables are set."));
        }
        lines
    }

    fn lock_memory(&self) -> std::sync::MutexGuard<'_, MathMemory> {
        self.memory
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl ConsoleExtension for MathExtension {
    fn metadata(&self) -> &ExtensionMetadata {
        &self.metadata
    }

    fn execute(
        &self,
        command: &str,
        args: &[String],
        _ctx: &ConsoleContext,
        _registry: &ConsoleExtensionRegistry,
    ) -> ConsoleCommandResponse {
        match command {
            "base" => self.execute_base(args),
            "calc" => self.execute_calc(args),
            _ => ConsoleCommandResponse::error(
                format!("math extension does not handle :{command}"),
                127,
            ),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct SolveQueryOptions {
    variable: Option<String>,
    domain: Option<(f64, f64)>,
    samples: Option<usize>,
}

fn parse_solve_query_options(input: &str) -> Result<(&str, SolveQueryOptions), MathError> {
    let mut relation_text = input.trim();
    let mut options = SolveQueryOptions::default();

    if let Some((before, after)) = split_keyword_outside(relation_text, " samples ") {
        let sample_text = after.trim();
        options.samples = Some(
            sample_text
                .parse::<usize>()
                .map_err(|_| MathError::new(format!("invalid sample count '{sample_text}'")))?,
        );
        relation_text = before.trim();
    }

    if let Some((before, after)) = split_keyword_outside(relation_text, " from ") {
        options.domain = Some(parse_domain(after.trim())?);
        relation_text = before.trim();
    }

    if let Some((before, after)) = split_keyword_outside(relation_text, " for ") {
        let variable = after.trim().to_ascii_lowercase();
        if !crate::app::math::expr::is_valid_identifier(&variable) {
            return Err(MathError::new(format!(
                "invalid solve variable '{variable}'"
            )));
        }
        options.variable = Some(variable);
        relation_text = before.trim();
    }

    Ok((relation_text, options))
}

fn parse_domain(input: &str) -> Result<(f64, f64), MathError> {
    let Some((min_text, max_text)) = input.split_once("..") else {
        return Err(MathError::new("domain must use '<min>..<max>'"));
    };
    let min = parse_expression(min_text.trim())
        .and_then(|expr| expr.eval(&EvalContext::new()))
        .and_then(require_finite)?;
    let max = parse_expression(max_text.trim())
        .and_then(|expr| expr.eval(&EvalContext::new()))
        .and_then(require_finite)?;
    if min >= max {
        return Err(MathError::new("domain start must be smaller than end"));
    }
    Ok((min, max))
}

fn split_keyword_outside<'a>(input: &'a str, keyword: &str) -> Option<(&'a str, &'a str)> {
    let lower = input.to_ascii_lowercase();
    let mut depth = 0i32;
    for (idx, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if depth == 0 && lower[idx..].starts_with(keyword) {
            let after = idx + keyword.len();
            return Some((&input[..idx], &input[after..]));
        }
    }
    None
}

fn unknown_variables(relation: &Relation, variables: &BTreeMap<String, f64>) -> Vec<String> {
    relation
        .variables()
        .into_iter()
        .filter(|name| !variables.contains_key(name))
        .collect()
}

fn base_output(conversion: BaseConversion) -> Vec<OutputLine> {
    let mut lines = vec![OutputLine::system(format!(
        "Base conversion: input base {} -> decimal {}",
        conversion.source_base, conversion.value
    ))];
    lines.push(OutputLine::stdout("base  value"));
    lines.push(OutputLine::stdout("----  -----"));
    for row in conversion.rows {
        lines.push(OutputLine::stdout(format!(
            "{:<4}  {}",
            row.base, row.value
        )));
    }
    lines
}

fn calc_help() -> Vec<OutputLine> {
    vec![
        OutputLine::system("Engineering calculator"),
        OutputLine::stdout(":calc 2 * sin(pi / 4)^2"),
        OutputLine::stdout(":calc let a = 42"),
        OutputLine::stdout(":calc ans + sqrt(a)"),
        OutputLine::stdout(":calc solve x^2 - 4 = 0 for x from -10..10"),
        OutputLine::stdout(":calc solve sin(x) > 0 from -pi..pi"),
        OutputLine::stdout(":calc vars"),
        OutputLine::stdout(":calc clear"),
    ]
}

fn solve_output(report: SolveReport) -> Vec<OutputLine> {
    match report {
        SolveReport::Constant { holds } => vec![OutputLine::stdout(holds.to_string())],
        SolveReport::Equality {
            variable,
            min,
            max,
            roots,
        } => {
            let mut lines = vec![OutputLine::system(format!(
                "Numeric solve for {variable} in [{}..{}]",
                format_number(min),
                format_number(max)
            ))];
            if roots.is_empty() {
                lines.push(OutputLine::stdout(
                    "No roots found. Try a wider domain or more samples.",
                ));
            } else {
                for root in roots.iter().take(32) {
                    lines.push(OutputLine::stdout(format!(
                        "{variable} ~= {}",
                        format_number(*root)
                    )));
                }
                if roots.len() > 32 {
                    lines.push(OutputLine::system(format!(
                        "{} additional roots omitted",
                        roots.len() - 32
                    )));
                }
            }
            lines
        }
        SolveReport::Inequality {
            variable,
            min,
            max,
            intervals,
        } => {
            let mut lines = vec![OutputLine::system(format!(
                "Numeric inequality solve for {variable} in [{}..{}]",
                format_number(min),
                format_number(max)
            ))];
            if intervals.is_empty() {
                lines.push(OutputLine::stdout("No matching intervals found."));
            } else {
                for interval in intervals.iter().take(32) {
                    lines.push(OutputLine::stdout(format!(
                        "{variable} in {}",
                        format_interval(interval)
                    )));
                }
                if intervals.len() > 32 {
                    lines.push(OutputLine::system(format!(
                        "{} additional intervals omitted",
                        intervals.len() - 32
                    )));
                }
            }
            lines
        }
    }
}

fn format_interval(interval: &Interval) -> String {
    format!(
        "{}{}, {}{}",
        if interval.include_start { "[" } else { "(" },
        format_number(interval.start),
        format_number(interval.end),
        if interval.include_end { "]" } else { ")" }
    )
}

fn format_number(value: f64) -> String {
    let value = if value.abs() < 1e-12 { 0.0 } else { value };
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

fn require_finite(value: f64) -> Result<f64, MathError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(MathError::new("result is not finite"))
    }
}

fn math_error(input: &str, error: MathError) -> ConsoleCommandResponse {
    ConsoleCommandResponse::error(error.with_input(input), 2)
}

fn strip_prefix_ci<'a>(input: &'a str, prefix: &str) -> Option<&'a str> {
    input
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        .then(|| &input[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_solve_query_extracts_options() {
        let (relation, options) =
            parse_solve_query_options("x^2 - 4 = 0 for x from -10..10").unwrap();
        assert_eq!(relation, "x^2 - 4 = 0");
        assert_eq!(options.variable.as_deref(), Some("x"));
        assert_eq!(options.domain, Some((-10.0, 10.0)));
    }
}
