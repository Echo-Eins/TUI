pub mod base;
pub mod exact;
pub mod expr;
pub mod formula;
pub mod solver;

pub use base::{convert_base, BaseConversion, BaseRow};
pub use exact::{
    format_exact_number, format_number, format_pi_multiple, relation_fallback, solve_exact,
    ExactSolveReport,
};
pub use expr::{parse_expression, EvalContext, Expr, MathError};
pub use formula::{format_expr, render_formula, FormulaRender};
pub use solver::{
    parse_relation, solve_relation, Interval, Relation, RelationOp, SolveOptions, SolveReport,
};
