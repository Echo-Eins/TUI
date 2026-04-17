use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use unicode_width::UnicodeWidthStr;

use crate::app::console_state::{ConsolePlotBlock, ConsolePlotMode, ConsolePlotSeries, OutputLine};
use crate::app::math::{
    convert_base, format_exact_number, format_expr, format_number as format_math_number,
    format_pi_multiple, parse_expression, parse_relation, relation_fallback, render_formula,
    solve_exact, solve_relation, BaseConversion, EvalContext, ExactSolveReport, FormulaRender,
    Interval, MathError, PlotCache, PlotMode, PlotRender, PlotRequest, Relation, SolveOptions,
    SolveReport, MAX_PLOT_HEIGHT, MAX_PLOT_SAMPLES, MAX_PLOT_WIDTH, MIN_PLOT_HEIGHT,
    MIN_PLOT_SAMPLES, MIN_PLOT_WIDTH,
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
    plot_cache: PlotCache,
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

#[derive(Debug, Clone, Copy, Default)]
struct CalcFlags {
    numeric: bool,
    math_block: bool,
    pretty: bool,
}

#[derive(Debug, Clone)]
struct CalcInput {
    body: String,
    flags: CalcFlags,
}

#[derive(Debug, Clone, Default)]
struct SolveQueryOptions {
    variable: Option<String>,
    domain: Option<(f64, f64)>,
    domain_provided: bool,
    samples: Option<usize>,
}

#[derive(Debug, Clone)]
struct FormulaQuery {
    expression: String,
    target: Option<String>,
}

#[derive(Debug, Clone)]
struct PlotQuery {
    expression: String,
    expr: crate::app::math::Expr,
    variable: Option<String>,
    x_range: Option<(f64, f64)>,
    y_range: Option<(f64, f64)>,
    samples: PlotSampleSetting,
    mode: PlotMode,
    width: Option<usize>,
    height: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlotSampleSetting {
    Auto,
    Count(usize),
}

#[derive(Debug, Clone)]
struct MathBlock {
    title: String,
    mode: String,
    method: String,
    status: String,
    input: String,
    exact_lines: Vec<String>,
    formula: Option<FormulaRender>,
    approx_lines: Vec<String>,
    domain: String,
    vars: Vec<String>,
}

impl MathExtension {
    pub(super) fn new() -> Self {
        Self {
            metadata: ExtensionMetadata {
                id: "math",
                title: "Math",
                version: "0.3.0",
                kind: ExtensionKind::Builtin,
                description: "Engineering calculator, numeral base conversion, formula rendering, plotting, and shared math parser.",
                commands: vec![
                    ConsoleCommandSpec {
                        name: "base",
                        summary: "Convert integers between bases 2..16.",
                        usage: ":base <value> [from <base>] to <base|start..end|all>",
                        tags: &["math", "base", "conversion"],
                    },
                    ConsoleCommandSpec {
                        name: "calc",
                        summary: "Evaluate expressions, render formulas, and solve equations.",
                        usage: ":calc <expr> [-num] [-mb] | :calc --pretty <expr> | :calc formula <expr> [for x] [-mb] | :calc solve <relation> [for x] [from a..b]",
                        tags: &["math", "calculator", "solver", "formula"],
                    },
                    ConsoleCommandSpec {
                        name: "formula",
                        summary: "Render a LaTeX-like terminal formula.",
                        usage: ":formula <expr> [for <var>] [-mb]",
                        tags: &["math", "formula"],
                    },
                    ConsoleCommandSpec {
                        name: "plot",
                        summary: "Render a bounded terminal function plot.",
                        usage: ":plot <expr> [for x] [from a..b|x=a..b] [y=a..b] [--samples auto|N] [--mode line|points|bars|sparkline]",
                        tags: &["math", "plot", "modulator"],
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

    fn execute_plot(&self, args: &[String], ctx: &ConsoleContext) -> ConsoleCommandResponse {
        let query = match parse_plot_query(args) {
            Ok(query) => query,
            Err(error) => return ConsoleCommandResponse::error(error.message, 2),
        };

        let mut memory = self.lock_memory();
        let variables = memory.evaluation_variables();
        let variable = match choose_plot_variable(&query, &variables) {
            Ok(variable) => variable,
            Err(error) => return ConsoleCommandResponse::error(error.message, 2),
        };
        let unresolved = query
            .expr
            .variables()
            .into_iter()
            .filter(|name| name != &variable)
            .filter(|name| !variables.contains_key(name))
            .collect::<Vec<_>>();
        if !unresolved.is_empty() {
            return ConsoleCommandResponse::error(
                format!(
                    "plot has unresolved parameters: {}; assign them with ':calc let' or plot for a single target variable",
                    unresolved.join(", ")
                ),
                2,
            );
        }

        let block_width = math_block_width(ctx);
        let content_width = block_width
            .saturating_sub(6)
            .clamp(MIN_PLOT_WIDTH, MAX_PLOT_WIDTH);
        let plot_width = query
            .width
            .unwrap_or(content_width)
            .clamp(MIN_PLOT_WIDTH, content_width);
        let default_height = if ctx.theme.compact_mode { 10 } else { 14 };
        let plot_height = query
            .height
            .unwrap_or(default_height)
            .clamp(MIN_PLOT_HEIGHT, MAX_PLOT_HEIGHT);
        let samples = match query.samples {
            PlotSampleSetting::Auto => (plot_width * 4).clamp(MIN_PLOT_SAMPLES, MAX_PLOT_SAMPLES),
            PlotSampleSetting::Count(samples) => samples.clamp(MIN_PLOT_SAMPLES, MAX_PLOT_SAMPLES),
        };

        let request = PlotRequest {
            expression: query.expression.clone(),
            expr: query.expr.clone(),
            variable,
            x_min: query
                .x_range
                .map(|range| range.0)
                .unwrap_or(-std::f64::consts::TAU),
            x_max: query
                .x_range
                .map(|range| range.1)
                .unwrap_or(std::f64::consts::TAU),
            y_range: query.y_range,
            samples,
            width: plot_width,
            height: plot_height,
            mode: query.mode,
        };

        match memory.plot_cache.render(&request, &variables) {
            Ok((render, cache_hit)) => ConsoleCommandResponse::plot(console_plot_block(
                &request,
                &render,
                cache_hit,
                block_width,
            )),
            Err(error) => math_error(&query.expression, error),
        }
    }

    fn execute_calc(&self, args: &[String], ctx: &ConsoleContext) -> ConsoleCommandResponse {
        let input = parse_calc_input(args);
        let body = input.body.trim();
        if body.is_empty() || body.eq_ignore_ascii_case("help") {
            return ConsoleCommandResponse::ok(calc_help());
        }

        if input.flags.pretty {
            return self.execute_formula_text(body, input.flags, ctx);
        }

        if !input.flags.math_block && !input.flags.numeric && body.eq_ignore_ascii_case("vars") {
            return ConsoleCommandResponse::ok(self.variables_output());
        }

        if !input.flags.math_block && !input.flags.numeric && body.eq_ignore_ascii_case("clear") {
            let mut memory = self.lock_memory();
            memory.ans = None;
            memory.variables.clear();
            return ConsoleCommandResponse::ok(vec![OutputLine::system(
                "Calculator memory cleared.",
            )]);
        }

        if let Some(rest) = strip_prefix_ci(body, "let ") {
            return self.execute_let(rest.trim(), input.flags, ctx);
        }

        if let Some(rest) = strip_prefix_ci(body, "formula ") {
            return self.execute_formula_text(rest.trim(), input.flags, ctx);
        }

        if let Some(rest) = strip_prefix_ci(body, "solve ") {
            return self.execute_solve(rest.trim(), input.flags, ctx);
        }

        let (relation_text, options) = match parse_solve_query_options(body) {
            Ok(parsed) => parsed,
            Err(error) => return ConsoleCommandResponse::error(error.message, 2),
        };

        match parse_relation(relation_text) {
            Ok(Some(relation)) => {
                self.execute_relation(relation_text, relation, options, input.flags, ctx)
            }
            Ok(None) => self.execute_expression(body, input.flags, ctx),
            Err(error) => math_error(relation_text, error),
        }
    }

    fn execute_formula_command(
        &self,
        args: &[String],
        ctx: &ConsoleContext,
    ) -> ConsoleCommandResponse {
        let input = parse_calc_input(args);
        self.execute_formula_text(input.body.trim(), input.flags, ctx)
    }

    fn execute_formula_text(
        &self,
        input: &str,
        flags: CalcFlags,
        ctx: &ConsoleContext,
    ) -> ConsoleCommandResponse {
        let query = match parse_formula_query(input) {
            Ok(query) => query,
            Err(error) => return ConsoleCommandResponse::error(error.message, 2),
        };
        let expr = match parse_expression(&query.expression) {
            Ok(expr) => expr,
            Err(error) => return math_error(&query.expression, error),
        };

        let variables = self.lock_memory().evaluation_variables();
        let render = render_formula(&expr, query.target.as_deref(), formula_width(ctx));
        let approx = expr
            .eval(&EvalContext::with_variables(variables.clone()))
            .and_then(require_finite)
            .ok();

        if flags.math_block {
            let mut vars = expr.variables();
            if let Some(target) = &query.target {
                vars.insert(target.clone());
            }
            return ConsoleCommandResponse::ok(render_math_block(
                MathBlock {
                    title: "MATH BLOCK / formula".to_string(),
                    mode: "formula".to_string(),
                    method: "ast-layout".to_string(),
                    status: if approx.is_some() {
                        "numeric-ready".to_string()
                    } else {
                        "symbolic".to_string()
                    },
                    input: input.to_string(),
                    exact_lines: vec![render.fallback.clone()],
                    formula: Some(render),
                    approx_lines: approx
                        .map(|value| vec![format!("~= {}", format_math_number(value))])
                        .unwrap_or_else(|| {
                            vec!["unavailable until all variables are assigned".to_string()]
                        }),
                    domain: "n/a".to_string(),
                    vars: variable_rows(vars, query.target.as_deref(), &variables),
                },
                math_block_width(ctx),
            ));
        }

        let mut lines = vec![OutputLine::system("Formula:")];
        lines.extend(render.pretty.into_iter().map(OutputLine::stdout));
        lines.push(OutputLine::system("ASCII fallback:"));
        lines.push(OutputLine::stdout(render.fallback));
        if flags.numeric {
            if let Some(value) = approx {
                lines.push(OutputLine::system("Numeric:"));
                lines.push(OutputLine::stdout(format_math_number(value)));
            } else {
                lines.push(OutputLine::system(
                    "Numeric unavailable until all variables are assigned.",
                ));
            }
        }
        ConsoleCommandResponse::ok(lines)
    }

    fn execute_let(
        &self,
        input: &str,
        flags: CalcFlags,
        ctx: &ConsoleContext,
    ) -> ConsoleCommandResponse {
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
        let expr = match parse_expression(expr_input) {
            Ok(expr) => expr,
            Err(error) => return math_error(expr_input, error),
        };
        let value = match expr
            .eval(&EvalContext::with_variables(variables))
            .and_then(require_finite)
        {
            Ok(value) => value,
            Err(error) => return math_error(expr_input, error),
        };

        let mut memory = self.lock_memory();
        memory.variables.insert(name.clone(), value);
        memory.ans = Some(value);
        drop(memory);

        let exact = expression_exact_text(&expr, value);
        if flags.math_block {
            return ConsoleCommandResponse::ok(render_math_block(
                MathBlock {
                    title: "MATH BLOCK / let".to_string(),
                    mode: "exact + numeric".to_string(),
                    method: "assignment".to_string(),
                    status: "stored".to_string(),
                    input: format!("let {name} = {expr_input}"),
                    exact_lines: vec![format!("{name} = {exact}")],
                    formula: Some(render_formula(&expr, Some(&name), formula_width(ctx))),
                    approx_lines: vec![format!("{name} ~= {}", format_math_number(value))],
                    domain: "n/a".to_string(),
                    vars: self.variables_output_plain(Some(&name)),
                },
                math_block_width(ctx),
            ));
        }

        if flags.numeric && exact != format_math_number(value) {
            ConsoleCommandResponse::ok(vec![OutputLine::stdout(format!(
                "{name} = {exact}    ~= {}",
                format_math_number(value)
            ))])
        } else {
            ConsoleCommandResponse::ok(vec![OutputLine::stdout(format!("{name} = {exact}"))])
        }
    }

    fn execute_expression(
        &self,
        input: &str,
        flags: CalcFlags,
        ctx: &ConsoleContext,
    ) -> ConsoleCommandResponse {
        let variables = self.lock_memory().evaluation_variables();
        let expr = match parse_expression(input) {
            Ok(expr) => expr,
            Err(error) => return math_error(input, error),
        };
        let value = match expr
            .eval(&EvalContext::with_variables(variables.clone()))
            .and_then(require_finite)
        {
            Ok(value) => value,
            Err(error) => return math_error(input, error),
        };

        self.lock_memory().ans = Some(value);
        let exact = expression_exact_text(&expr, value);

        if flags.math_block {
            return ConsoleCommandResponse::ok(render_math_block(
                MathBlock {
                    title: "MATH BLOCK / calc".to_string(),
                    mode: "exact + numeric".to_string(),
                    method: "expression".to_string(),
                    status: "evaluated".to_string(),
                    input: input.to_string(),
                    exact_lines: vec![exact.clone()],
                    formula: Some(render_formula(&expr, None, formula_width(ctx))),
                    approx_lines: vec![format!("~= {}", format_math_number(value))],
                    domain: "n/a".to_string(),
                    vars: variable_rows(expr.variables(), None, &variables),
                },
                math_block_width(ctx),
            ));
        }

        if flags.numeric && exact != format_math_number(value) {
            ConsoleCommandResponse::ok(vec![OutputLine::stdout(format!(
                "{exact}    ~= {}",
                format_math_number(value)
            ))])
        } else {
            ConsoleCommandResponse::ok(vec![OutputLine::stdout(exact)])
        }
    }

    fn execute_relation(
        &self,
        input: &str,
        relation: Relation,
        options: SolveQueryOptions,
        flags: CalcFlags,
        ctx: &ConsoleContext,
    ) -> ConsoleCommandResponse {
        let variables = self.lock_memory().evaluation_variables();
        let unknowns = unknown_variables(&relation, &variables);

        if unknowns.is_empty() && options.variable.is_none() {
            return match solve_relation(&relation, None, &variables) {
                Ok(SolveReport::Constant { holds }) => {
                    ConsoleCommandResponse::ok(vec![OutputLine::stdout(holds.to_string())])
                }
                Ok(report) => ConsoleCommandResponse::ok(solve_output(report, flags, ctx, None)),
                Err(error) => math_error(input, error),
            };
        }

        self.execute_solve_with_relation(input, relation, options, flags, ctx)
    }

    fn execute_solve(
        &self,
        input: &str,
        flags: CalcFlags,
        ctx: &ConsoleContext,
    ) -> ConsoleCommandResponse {
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

        self.execute_solve_with_relation(relation_text, relation, options, flags, ctx)
    }

    fn execute_solve_with_relation(
        &self,
        input: &str,
        relation: Relation,
        options: SolveQueryOptions,
        flags: CalcFlags,
        ctx: &ConsoleContext,
    ) -> ConsoleCommandResponse {
        let variables = self.lock_memory().evaluation_variables();
        let unknowns = unknown_variables(&relation, &variables);

        let variable = match options.variable.clone() {
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

        let exact = solve_exact(
            &relation,
            &variable,
            &variables,
            options.domain.filter(|_| options.domain_provided),
        );
        if let Some(exact) = exact {
            return ConsoleCommandResponse::ok(exact_solve_output(
                exact, &relation, input, flags, ctx, &variables,
            ));
        }

        let unresolved = unknowns
            .iter()
            .filter(|name| *name != &variable)
            .cloned()
            .collect::<Vec<_>>();
        if !unresolved.is_empty() {
            return ConsoleCommandResponse::error(
                format!(
                    "numeric fallback cannot resolve symbolic parameters: {}; assign them with ':calc let' or use an exact-supported equation form",
                    unresolved.join(", ")
                ),
                2,
            );
        }

        let (min, max) = options.domain.unwrap_or((-100.0, 100.0));
        let mut solve_options = SolveOptions::new(variable, min, max);
        solve_options.samples = options.samples.unwrap_or(2048);

        match solve_relation(&relation, Some(solve_options), &variables) {
            Ok(report) => ConsoleCommandResponse::ok(solve_output(
                report,
                flags,
                ctx,
                Some((&relation, input, &variables)),
            )),
            Err(error) => math_error(input, error),
        }
    }

    fn variables_output(&self) -> Vec<OutputLine> {
        self.variables_output_plain(None)
            .into_iter()
            .map(OutputLine::stdout)
            .collect()
    }

    fn variables_output_plain(&self, target: Option<&str>) -> Vec<String> {
        let memory = self.lock_memory();
        let mut lines = Vec::new();
        if let Some(ans) = memory.ans {
            lines.push(format!("ans  memory  {}", format_math_number(ans)));
        }
        for (name, value) in &memory.variables {
            let role = if Some(name.as_str()) == target {
                "target"
            } else {
                "param"
            };
            lines.push(format!(
                "{name:<4} {role:<7} {}",
                format_math_number(*value)
            ));
        }
        if lines.is_empty() {
            lines.push("none".to_string());
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
        ctx: &ConsoleContext,
        _registry: &ConsoleExtensionRegistry,
    ) -> ConsoleCommandResponse {
        match command {
            "base" => self.execute_base(args),
            "calc" => self.execute_calc(args, ctx),
            "formula" => self.execute_formula_command(args, ctx),
            "plot" => self.execute_plot(args, ctx),
            _ => ConsoleCommandResponse::error(
                format!("math extension does not handle :{command}"),
                127,
            ),
        }
    }
}

fn parse_calc_input(args: &[String]) -> CalcInput {
    let mut flags = CalcFlags::default();
    let mut body = Vec::new();
    for arg in args {
        match arg.as_str() {
            "-mb" | "--mb" | "--math-block" => flags.math_block = true,
            "-num" | "--num" | "--numeric" => flags.numeric = true,
            "--pretty" | "-pretty" => flags.pretty = true,
            _ => body.push(arg.clone()),
        }
    }
    CalcInput {
        body: body.join(" "),
        flags,
    }
}

fn parse_formula_query(input: &str) -> Result<FormulaQuery, MathError> {
    let (expression, target) = if let Some((before, after)) = split_keyword_outside(input, " for ")
    {
        let target = after.trim().to_ascii_lowercase();
        if !crate::app::math::expr::is_valid_identifier(&target) {
            return Err(MathError::new(format!("invalid formula target '{target}'")));
        }
        (before.trim().to_string(), Some(target))
    } else {
        (input.trim().to_string(), None)
    };

    if expression.is_empty() {
        return Err(MathError::new("formula expression is empty"));
    }
    Ok(FormulaQuery { expression, target })
}

fn parse_plot_query(args: &[String]) -> Result<PlotQuery, MathError> {
    let mut body = Vec::new();
    let mut samples = PlotSampleSetting::Auto;
    let mut mode = PlotMode::Line;
    let mut width = None;
    let mut height = None;
    let mut x_range = None;
    let mut y_range = None;
    let mut idx = 0usize;

    while idx < args.len() {
        let arg = &args[idx];
        match arg.as_str() {
            "--line" => mode = PlotMode::Line,
            "--points" => mode = PlotMode::Points,
            "--bars" => mode = PlotMode::Bars,
            "--spark" | "--sparkline" => mode = PlotMode::Sparkline,
            "--samples" => {
                idx += 1;
                let Some(value) = args.get(idx) else {
                    return Err(MathError::new("--samples expects auto or a number"));
                };
                samples = parse_plot_samples(value)?;
            }
            "--mode" => {
                idx += 1;
                let Some(value) = args.get(idx) else {
                    return Err(MathError::new(
                        "--mode expects line, points, bars, or sparkline",
                    ));
                };
                mode = parse_plot_mode(value)?;
            }
            "--width" => {
                idx += 1;
                let Some(value) = args.get(idx) else {
                    return Err(MathError::new("--width expects a number"));
                };
                width = Some(parse_plot_dimension(value, "width")?);
            }
            "--height" => {
                idx += 1;
                let Some(value) = args.get(idx) else {
                    return Err(MathError::new("--height expects a number"));
                };
                height = Some(parse_plot_dimension(value, "height")?);
            }
            _ if arg.starts_with("--samples=") => {
                samples = parse_plot_samples(&arg["--samples=".len()..])?;
            }
            _ if arg.starts_with("--mode=") => {
                mode = parse_plot_mode(&arg["--mode=".len()..])?;
            }
            _ if arg.starts_with("--width=") => {
                width = Some(parse_plot_dimension(&arg["--width=".len()..], "width")?);
            }
            _ if arg.starts_with("--height=") => {
                height = Some(parse_plot_dimension(&arg["--height=".len()..], "height")?);
            }
            _ if arg.starts_with("x=") => {
                x_range = Some(parse_domain(&arg["x=".len()..])?);
            }
            _ if arg.starts_with("y=") => {
                y_range = Some(parse_domain(&arg["y=".len()..])?);
            }
            _ => body.push(arg.clone()),
        }
        idx += 1;
    }

    let mut expression_text = body.join(" ");
    if let Some((before, after)) = split_keyword_outside(&expression_text, " from ") {
        x_range = Some(parse_domain(after.trim())?);
        expression_text = before.trim().to_string();
    }

    let (expression_text, variable) =
        if let Some((before, after)) = split_keyword_outside(&expression_text, " for ") {
            let variable = after.trim().to_ascii_lowercase();
            if !crate::app::math::expr::is_valid_identifier(&variable) {
                return Err(MathError::new(format!(
                    "invalid plot variable '{variable}'"
                )));
            }
            (before.trim().to_string(), Some(variable))
        } else {
            (expression_text.trim().to_string(), None)
        };

    if expression_text.is_empty() {
        return Err(MathError::new("plot expression is empty"));
    }
    let expr = parse_expression(&expression_text)?;

    Ok(PlotQuery {
        expression: expression_text,
        expr,
        variable,
        x_range,
        y_range,
        samples,
        mode,
        width,
        height,
    })
}

fn parse_plot_samples(input: &str) -> Result<PlotSampleSetting, MathError> {
    if input.eq_ignore_ascii_case("auto") {
        return Ok(PlotSampleSetting::Auto);
    }
    let samples = input
        .parse::<usize>()
        .map_err(|_| MathError::new(format!("invalid plot sample count '{input}'")))?;
    if !(MIN_PLOT_SAMPLES..=MAX_PLOT_SAMPLES).contains(&samples) {
        return Err(MathError::new(format!(
            "plot samples must be in {MIN_PLOT_SAMPLES}..{MAX_PLOT_SAMPLES}"
        )));
    }
    Ok(PlotSampleSetting::Count(samples))
}

fn parse_plot_mode(input: &str) -> Result<PlotMode, MathError> {
    PlotMode::parse(input).ok_or_else(|| MathError::new(format!("invalid plot mode '{input}'")))
}

fn parse_plot_dimension(input: &str, name: &str) -> Result<usize, MathError> {
    input
        .parse::<usize>()
        .map_err(|_| MathError::new(format!("invalid plot {name} '{input}'")))
}

fn choose_plot_variable(
    query: &PlotQuery,
    variables: &BTreeMap<String, f64>,
) -> Result<String, MathError> {
    if let Some(variable) = &query.variable {
        return Ok(variable.clone());
    }

    let expr_vars = query.expr.variables();
    let unresolved = expr_vars
        .iter()
        .filter(|name| !variables.contains_key(*name))
        .cloned()
        .collect::<Vec<_>>();
    if unresolved.len() == 1 {
        return Ok(unresolved[0].clone());
    }
    if unresolved.is_empty() || unresolved.iter().any(|name| name == "x") {
        return Ok("x".to_string());
    }

    Err(MathError::new(format!(
        "plot expects one target variable, found {}; use 'for <name>'",
        unresolved.len()
    )))
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
        options.domain_provided = true;
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

fn console_plot_block(
    request: &PlotRequest,
    render: &PlotRender,
    cache_hit: bool,
    fallback_width: usize,
) -> ConsolePlotBlock {
    let fallback_lines = render_plot_block(request, render, cache_hit, fallback_width)
        .into_iter()
        .map(|line| line.text)
        .collect();

    ConsolePlotBlock {
        title: "PLOT / function".to_string(),
        expression: request.expression.clone(),
        variable: request.variable.clone(),
        mode: console_plot_mode(render.mode),
        x_min: render.x_min,
        x_max: render.x_max,
        y_min: render.y_min,
        y_max: render.y_max,
        x_min_label: format_pi_multiple(render.x_min),
        x_max_label: format_pi_multiple(render.x_max),
        y_min_label: format_math_number(render.y_min),
        y_max_label: format_math_number(render.y_max),
        samples: render.samples,
        finite_samples: render.finite_samples,
        invalid_samples: render.invalid_samples,
        clipped_samples: render.clipped_samples,
        discontinuities: render.discontinuities,
        cache_hit,
        requested_width: request.width,
        requested_height: request.height,
        series: render
            .series
            .iter()
            .map(|series| ConsolePlotSeries {
                points: series.points.clone(),
            })
            .collect(),
        fallback_lines,
    }
}

fn console_plot_mode(mode: PlotMode) -> ConsolePlotMode {
    match mode {
        PlotMode::Line => ConsolePlotMode::Line,
        PlotMode::Points => ConsolePlotMode::Points,
        PlotMode::Bars => ConsolePlotMode::Bars,
        PlotMode::Sparkline => ConsolePlotMode::Sparkline,
    }
}

fn render_plot_block(
    request: &PlotRequest,
    render: &PlotRender,
    cache_hit: bool,
    width: usize,
) -> Vec<OutputLine> {
    let width = width.clamp(48, 120);
    let cache = if cache_hit { "hit" } else { "miss" };
    let mut lines = Vec::new();
    lines.push(OutputLine::stdout(top_border("PLOT / function", width)));
    lines.push(row(
        &format!(
            "mode {:<10} variable {:<8} samples {:<5} cache {}",
            render.mode.label(),
            request.variable,
            render.samples,
            cache
        ),
        width,
    ));
    lines.push(row(
        &format!(
            "x [{}..{}]   y [{}..{}]",
            format_pi_multiple(render.x_min),
            format_pi_multiple(render.x_max),
            format_math_number(render.y_min),
            format_math_number(render.y_max)
        ),
        width,
    ));
    lines.push(row(
        &format!(
            "finite {} invalid {} clipped {} breaks {}",
            render.finite_samples,
            render.invalid_samples,
            render.clipped_samples,
            render.discontinuities
        ),
        width,
    ));
    section(&mut lines, "Input", &[request.expression.clone()], width);
    section(&mut lines, "Canvas", &render.canvas, width);
    lines.push(row(
        "actions: --mode line|points|bars|sparkline | --samples auto|N | x=a..b | y=a..b",
        width,
    ));
    lines.push(OutputLine::stdout(bottom_border(width)));
    lines
        .into_iter()
        .map(|mut line| {
            line.text = clamp_display_width(&line.text, width);
            line
        })
        .collect()
}

fn calc_help() -> Vec<OutputLine> {
    vec![
        OutputLine::system("Engineering calculator"),
        OutputLine::stdout(":calc 2 * sin(pi / 4)^2"),
        OutputLine::stdout(":calc 2 * sin(pi / 4)^2 -num"),
        OutputLine::stdout(":calc solve x^2 - 4 = 0 for x from -10..10 -mb"),
        OutputLine::stdout(":calc solve sin(x) > 0 from -pi..pi"),
        OutputLine::stdout(":calc formula \"(-b + sqrt(b^2 - 4*a*c)) / (2*a)\" for x -mb"),
        OutputLine::stdout(":calc --pretty \"x_1^2 / sqrt(y_2)\""),
        OutputLine::stdout(":formula sqrt(x^2 + y^2) for r -mb"),
        OutputLine::stdout(":calc let a = 42"),
        OutputLine::stdout(":calc vars"),
        OutputLine::stdout(":calc clear"),
    ]
}

fn exact_solve_output(
    report: ExactSolveReport,
    relation: &Relation,
    input: &str,
    flags: CalcFlags,
    ctx: &ConsoleContext,
    variables: &BTreeMap<String, f64>,
) -> Vec<OutputLine> {
    if flags.math_block {
        let formula = exact_result_formula(&report.exact_lines, ctx);
        return render_math_block(
            MathBlock {
                title: "MATH BLOCK / solve".to_string(),
                mode: "exact + numeric".to_string(),
                method: report.method.to_string(),
                status: report.status.to_string(),
                input: input.to_string(),
                exact_lines: report.exact_lines,
                formula: Some(formula),
                approx_lines: report.numeric_lines,
                domain: report
                    .domain
                    .map(|(min, max)| {
                        format!("[{}..{}]", format_pi_multiple(min), format_pi_multiple(max))
                    })
                    .unwrap_or_else(|| "real".to_string()),
                vars: variable_rows(relation.variables(), Some(&report.variable), variables),
            },
            math_block_width(ctx),
        );
    }

    let mut lines = Vec::new();
    for (idx, exact) in report.exact_lines.iter().enumerate() {
        if flags.numeric {
            if let Some(numeric) = report.numeric_lines.get(idx) {
                lines.push(OutputLine::stdout(format!("{exact}    {numeric}")));
            } else {
                lines.push(OutputLine::stdout(exact.clone()));
            }
        } else {
            lines.push(OutputLine::stdout(exact.clone()));
        }
    }
    lines
}

fn exact_result_formula(exact_lines: &[String], ctx: &ConsoleContext) -> FormulaRender {
    let mut pretty = Vec::new();
    let mut fallback = Vec::new();

    for line in exact_lines {
        fallback.push(line.clone());
        let Some((lhs, rhs)) = line.split_once(" = ") else {
            pretty.push(line.clone());
            continue;
        };

        match parse_expression(rhs.trim()) {
            Ok(expr) => {
                pretty.extend(render_formula(&expr, Some(lhs.trim()), formula_width(ctx)).pretty)
            }
            Err(_) => pretty.push(line.clone()),
        }
    }

    FormulaRender {
        pretty,
        fallback: fallback.join("; "),
    }
}

fn solve_output(
    report: SolveReport,
    flags: CalcFlags,
    ctx: &ConsoleContext,
    relation_context: Option<(&Relation, &str, &BTreeMap<String, f64>)>,
) -> Vec<OutputLine> {
    match report {
        SolveReport::Constant { holds } => vec![OutputLine::stdout(holds.to_string())],
        SolveReport::Equality {
            variable,
            min,
            max,
            roots,
        } => {
            let result_lines = if roots.is_empty() {
                vec![
                    "No roots found inside this domain.".to_string(),
                    "The numeric solver only searches the displayed range; use 'from <min>..<max>' or more samples when the expected solution is outside it.".to_string(),
                ]
            } else {
                roots
                    .iter()
                    .take(32)
                    .map(|root| format!("{variable} ~= {}", format_math_number(*root)))
                    .collect::<Vec<_>>()
            };

            if flags.math_block {
                let vars = relation_context
                    .map(|(relation, _, variables)| {
                        variable_rows(relation.variables(), Some(&variable), variables)
                    })
                    .unwrap_or_else(|| vec![format!("{variable:<4} target  real")]);
                return render_math_block(
                    MathBlock {
                        title: "MATH BLOCK / solve".to_string(),
                        mode: "numeric fallback".to_string(),
                        method: "sample + bisection".to_string(),
                        status: "approx".to_string(),
                        input: relation_context
                            .map(|(_, input, _)| input.to_string())
                            .unwrap_or_else(|| "solve".to_string()),
                        exact_lines: vec![
                            "No closed-form exact result available in the current solver."
                                .to_string(),
                        ],
                        formula: relation_context.map(|(relation, _, _)| FormulaRender {
                            pretty: vec![relation_fallback(relation)],
                            fallback: relation_fallback(relation),
                        }),
                        approx_lines: result_lines,
                        domain: format!(
                            "[{}..{}]",
                            format_pi_multiple(min),
                            format_pi_multiple(max)
                        ),
                        vars,
                    },
                    math_block_width(ctx),
                );
            }

            result_lines.into_iter().map(OutputLine::stdout).collect()
        }
        SolveReport::Inequality {
            variable,
            min,
            max,
            intervals,
        } => {
            let mut result_lines = Vec::new();
            if intervals.is_empty() {
                result_lines.push("No matching intervals found.".to_string());
            } else {
                for interval in intervals.iter().take(32) {
                    result_lines.push(format!("{variable} in {}", format_interval(interval)));
                }
                if intervals.len() > 32 {
                    result_lines.push(format!(
                        "{} additional intervals omitted",
                        intervals.len() - 32
                    ));
                }
            }

            if flags.math_block {
                let vars = relation_context
                    .map(|(relation, _, variables)| {
                        variable_rows(relation.variables(), Some(&variable), variables)
                    })
                    .unwrap_or_else(|| vec![format!("{variable:<4} target  real")]);
                return render_math_block(
                    MathBlock {
                        title: "MATH BLOCK / solve".to_string(),
                        mode: "numeric fallback".to_string(),
                        method: "sample + intervals".to_string(),
                        status: "approx".to_string(),
                        input: relation_context
                            .map(|(_, input, _)| input.to_string())
                            .unwrap_or_else(|| "solve".to_string()),
                        exact_lines: vec![
                            "No closed-form exact interval family available in the current solver."
                                .to_string(),
                        ],
                        formula: relation_context.map(|(relation, _, _)| FormulaRender {
                            pretty: vec![relation_fallback(relation)],
                            fallback: relation_fallback(relation),
                        }),
                        approx_lines: result_lines,
                        domain: format!(
                            "[{}..{}]",
                            format_pi_multiple(min),
                            format_pi_multiple(max)
                        ),
                        vars,
                    },
                    math_block_width(ctx),
                );
            }

            result_lines.into_iter().map(OutputLine::stdout).collect()
        }
    }
}

fn format_interval(interval: &Interval) -> String {
    format!(
        "{}{}, {}{}",
        if interval.include_start { "[" } else { "(" },
        format_pi_multiple(interval.start),
        format_pi_multiple(interval.end),
        if interval.include_end { "]" } else { ")" }
    )
}

fn expression_exact_text(expr: &crate::app::math::Expr, value: f64) -> String {
    let symbolic = format_expr(expr);
    if has_symbolic_form(expr) {
        symbolic
    } else {
        format_exact_number(value)
    }
}

fn has_symbolic_form(expr: &crate::app::math::Expr) -> bool {
    match expr {
        crate::app::math::Expr::Number(_) => false,
        crate::app::math::Expr::Variable(_) => true,
        crate::app::math::Expr::Unary { expr, .. } => has_symbolic_form(expr),
        crate::app::math::Expr::Binary { left, right, .. } => {
            has_symbolic_form(left) || has_symbolic_form(right)
        }
        crate::app::math::Expr::Function { .. } => true,
    }
}

fn variable_rows(
    vars: BTreeSet<String>,
    target: Option<&str>,
    assigned: &BTreeMap<String, f64>,
) -> Vec<String> {
    if vars.is_empty() && assigned.is_empty() {
        return vec!["none".to_string()];
    }

    let mut all = vars;
    all.extend(assigned.keys().cloned());
    let mut rows = Vec::new();
    for name in all {
        let role = if Some(name.as_str()) == target {
            "target"
        } else if assigned.contains_key(&name) {
            "param"
        } else {
            "unknown"
        };
        let value = assigned
            .get(&name)
            .map(|value| format_math_number(*value))
            .unwrap_or_else(|| "real".to_string());
        rows.push(format!("{name:<4} {role:<7} {value}"));
    }
    rows
}

fn render_math_block(block: MathBlock, width: usize) -> Vec<OutputLine> {
    let width = width.clamp(48, 120);
    let mut lines = Vec::new();
    lines.push(OutputLine::stdout(top_border(&block.title, width)));
    lines.push(row(
        &format!(
            "mode {:<22} method {:<18} status {}",
            block.mode, block.method, block.status
        ),
        width,
    ));
    lines.push(row(&format!("domain {}", block.domain), width));
    section(&mut lines, "Input", &[block.input], width);
    section(&mut lines, "Exact Result", &block.exact_lines, width);
    if let Some(formula) = block.formula {
        section(&mut lines, "Formula", &formula.pretty, width);
        section(&mut lines, "ASCII Fallback", &[formula.fallback], width);
    }
    section(&mut lines, "Approx", &block.approx_lines, width);
    section(&mut lines, "Vars", &block.vars, width);
    lines.push(row(
        "actions: -num numeric | -mb math block | :calc vars | :calc clear",
        width,
    ));
    lines.push(OutputLine::stdout(bottom_border(width)));

    lines
        .into_iter()
        .map(|mut line| {
            line.text = clamp_display_width(&line.text, width);
            line
        })
        .collect()
}

fn section(lines: &mut Vec<OutputLine>, title: &str, body: &[String], width: usize) {
    separator(lines, width);
    lines.push(row(title, width));
    if body.is_empty() {
        lines.push(row("  none", width));
    } else {
        for line in body {
            lines.push(row(&format!("  {line}"), width));
        }
    }
}

fn separator(lines: &mut Vec<OutputLine>, width: usize) {
    lines.push(OutputLine::stdout(format!(
        "+{}+",
        "-".repeat(width.saturating_sub(2))
    )));
}

fn top_border(title: &str, width: usize) -> String {
    let label = format!(" {title} ");
    let label_width = UnicodeWidthStr::width(label.as_str());
    let dash_count = width.saturating_sub(label_width + 4);
    format!("+--{label}{}+", "-".repeat(dash_count))
}

fn bottom_border(width: usize) -> String {
    format!("+{}+", "-".repeat(width.saturating_sub(2)))
}

fn row(text: &str, width: usize) -> OutputLine {
    let inner = width.saturating_sub(4);
    let text = clamp_display_width(text, inner);
    let pad = inner.saturating_sub(UnicodeWidthStr::width(text.as_str()));
    OutputLine::stdout(format!("| {text}{} |", " ".repeat(pad)))
}

fn clamp_display_width(text: &str, max_width: usize) -> String {
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    if max_width <= 3 {
        return text.chars().take(max_width).collect();
    }
    let target = max_width - 3;
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + width > target {
            break;
        }
        out.push(ch);
        used += width;
    }
    out.push_str("...");
    out
}

fn math_block_width(ctx: &ConsoleContext) -> usize {
    usize::from(ctx.terminal_size.0)
        .saturating_sub(6)
        .clamp(56, 118)
}

fn formula_width(ctx: &ConsoleContext) -> usize {
    math_block_width(ctx).saturating_sub(8)
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
        assert!(options.domain_provided);
    }

    #[test]
    fn parse_formula_query_extracts_target() {
        let query = parse_formula_query("sqrt(b^2 - 4*a*c) for x").unwrap();
        assert_eq!(query.expression, "sqrt(b^2 - 4*a*c)");
        assert_eq!(query.target.as_deref(), Some("x"));
    }

    #[test]
    fn parse_calc_input_extracts_pretty_flag() {
        let input = parse_calc_input(&["--pretty".to_string(), "x_1^2".to_string()]);
        assert!(input.flags.pretty);
        assert_eq!(input.body, "x_1^2");
    }

    #[test]
    fn parse_plot_query_extracts_ranges_and_mode() {
        let query = parse_plot_query(&[
            "sin(x)".to_string(),
            "from".to_string(),
            "-pi..pi".to_string(),
            "y=-1..1".to_string(),
            "--samples=128".to_string(),
            "--mode".to_string(),
            "points".to_string(),
        ])
        .unwrap();
        assert_eq!(query.expression, "sin(x)");
        assert_eq!(
            query.x_range,
            Some((-std::f64::consts::PI, std::f64::consts::PI))
        );
        assert_eq!(query.y_range, Some((-1.0, 1.0)));
        assert_eq!(query.samples, PlotSampleSetting::Count(128));
        assert_eq!(query.mode, PlotMode::Points);
    }
}
