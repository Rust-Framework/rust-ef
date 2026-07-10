//! Query builder & LINQ-style chainable query API.
//!
//! Accumulates filter conditions, orderings, pagination, includes, and
//! projection metadata through a fluent interface. Terminal methods
//! (`to_list`, `first`, `count`, etc.) produce real SQL that can be
//! executed against a database provider.

mod ast;
mod builder;
mod compile;
mod cte;
mod execute_update;
mod having_expr;
mod helpers;
mod select;
mod source;
mod state;
mod window;

pub use ast::*;
pub use builder::QueryBuilder;
pub use cte::*;
pub use execute_update::ExecuteUpdateBuilder;
pub use having_expr::*;
pub use helpers::*;
pub use select::SelectQueryBuilder;
pub use source::*;
pub use state::QueryState;
pub use window::*;

// crate-internal helpers (used by change_executor / lazy / navigation_loader)
pub(crate) use compile::{collect_bool_expr_values, compile_bool_expr};
