pub mod base;
pub mod expr;
pub mod solver;

pub use base::{convert_base, BaseConversion, BaseRow};
pub use expr::{parse_expression, EvalContext, Expr, MathError};
pub use solver::{
    parse_relation, solve_relation, Interval, Relation, RelationOp, SolveOptions, SolveReport,
};
