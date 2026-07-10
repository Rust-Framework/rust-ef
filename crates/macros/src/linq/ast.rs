//! AST types for the `linq!` macro.
//!
//! `LinqInput` is the top-level dispatch between query form (A/B) and value
//! form (C). `LinqClause` enumerates all `;`-separated clauses supported in
//! Form B. `HavingExprAst` mirrors `rust_ef::query::HavingExpr` but carries
//! `syn::Expr` nodes resolved at expansion time.

use syn::{Expr, Ident, Type};

// ---------------------------------------------------------------------------
// Input types
// ---------------------------------------------------------------------------

/// Top-level dispatch: query form (A/B) vs. value form (C).
#[allow(clippy::large_enum_variant)]
pub(crate) enum LinqInput {
    Query(QueryInput),
    Value(ValueInput),
}

/// Form A (filter closure) and Form B (multi-clause query) share this shape.
pub(crate) struct QueryInput {
    /// The source `QueryBuilder<T>` expression (e.g. `ctx.set::<Blog>()`).
    /// `None` for Form A reusable closures.
    pub source: Option<Expr>,
    pub entity: Type,
    pub where_param: Option<Ident>,
    /// `None` for pure-clause queries (Form B without a where closure).
    pub where_body: Option<Expr>,
    /// Form A `=> field` order syntax (kept for backward compatibility).
    pub order: Option<LinqOrder>,
    /// Form B `;`-separated clauses.
    pub clauses: Vec<LinqClause>,
}

pub(crate) struct LinqOrder {
    pub body: Expr,
    pub descending: bool,
}

/// Form C value-producing inputs.
pub(crate) enum ValueInput {
    /// `linq!(filter |b: T| <bool_expr>)` → `BoolExpr`
    Filter {
        entity: Type,
        param: Ident,
        body: Expr,
    },
    /// `linq!(index |b: T| (f1, f2, ...))` → `&'static [&'static str]`
    Index { entity: Type, fields: Vec<Expr> },
    /// `linq!(key |b: T| (f1, f2, ...))` → `&'static [&'static str]`
    Key { entity: Type, fields: Vec<Expr> },
}

/// A single `;`-separated clause in Form B.
#[allow(clippy::large_enum_variant)]
pub(crate) enum LinqClause {
    /// `include b.posts then b.comments then b.author`
    Include { primary: Expr, nested: Vec<Expr> },
    /// `order_by b.created_at [asc|desc]`
    OrderBy { field: Expr, descending: bool },
    /// `group_by (b.cat, b.author)` or `group_by b.cat`
    GroupBy { fields: Vec<Expr> },
    /// `select (b.id, b.title)` or `select b.id`
    Select { fields: Vec<Expr> },
    /// `having <expr>` — supports `agg(col) op value`, `AND`, `OR`, `NOT`,
    /// and `agg(col) op agg(col)`.
    HavingExpr { expr: HavingExprAst },
    /// `sum b.views` (terminal)
    Sum(Expr),
    /// `avg b.rating` (terminal)
    Avg(Expr),
    /// `min b.rating` (terminal)
    Min(Expr),
    /// `max b.rating` (terminal)
    Max(Expr),
    /// `count` (terminal)
    Count,
    /// `distinct`
    Distinct,
    /// `set b.views, 10` (only valid before `execute_update`)
    Set { field: Expr, value: Expr },
    /// `inner_join |a: T1, b: T2| a.col == b.col`
    InnerJoin {
        params: Vec<(Ident, Type)>,
        left: Expr,
        right: Expr,
    },
    /// `left_join |a: T1, b: T2| a.col == b.col`
    LeftJoin {
        params: Vec<(Ident, Type)>,
        left: Expr,
        right: Expr,
    },
    /// `right_join |a: T1, b: T2| a.col == b.col`
    RightJoin {
        params: Vec<(Ident, Type)>,
        left: Expr,
        right: Expr,
    },
    /// `full_join |a: T1, b: T2| a.col == b.col`
    FullJoin {
        params: Vec<(Ident, Type)>,
        left: Expr,
        right: Expr,
    },
    /// `cross_join b: T2` — no ON condition
    CrossJoin {
        #[allow(
            dead_code,
            reason = "parsed from syntax for future select-binding support; cross_join_internal currently only needs the table name"
        )]
        param: Ident,
        entity: Type,
    },
    /// `union <expr>` where `<expr>` evaluates to `(String, Vec<DbValue>)`.
    Union(Expr),
    /// `union_all <expr>`
    UnionAll(Expr),
    /// `intersect <expr>`
    Intersect(Expr),
    /// `except <expr>`
    Except(Expr),
    /// `execute_update` (terminal, triggers bulk update)
    ExecuteUpdate,
    /// `take N`
    Take(Expr),
    /// `skip N`
    Skip(Expr),
    /// `window <func> [<col>] [partition_by <cols>] [order_by <col> [asc|desc]] as <alias>`
    Window {
        func: String,
        column: Option<Expr>,
        partition_by: Vec<Expr>,
        order_by: Vec<(Expr, bool)>,
        alias: String,
    },
    /// `with [recursive] <name> as |param: Type| <body> [link <fk> to <pk>]`
    /// — typed CTE definition. When `recursive` is true, `link` must be present.
    ///
    /// The closure body is compiled to a `BoolExpr` via `compile_bool_expr`
    /// and the CTE body (`SELECT * FROM <table> WHERE <expr>`) is generated
    /// at `to_sql_with` time with provider-correct placeholders. For recursive
    /// CTEs, the body serves as the anchor WHERE, and `link` provides the
    /// recursive join condition (`t.fk = name.pk`).
    With {
        name: String,
        entity: Type,
        param: Ident,
        body: Expr,
        recursive: bool,
        /// `(fk_expr, pk_expr)` — only present for recursive CTEs.
        link: Option<(Expr, Expr)>,
    },
    /// `from <name>` — query from a CTE name or named source.
    From { name: String },
}

/// Macro-side AST for `HAVING` expressions.
///
/// Mirrors `rust_ef::query::HavingExpr` but carries `syn::Expr` nodes for
/// column references and values, which are resolved to column constants and
/// `DbValue` literals at expansion time by `compile_having_expr`.
#[derive(Debug)]
pub(crate) enum HavingExprAst {
    /// `agg(col) op value`
    Compare {
        agg: String,
        col: Expr,
        op: String,
        value: Expr,
    },
    /// `expr AND expr`
    And(Box<HavingExprAst>, Box<HavingExprAst>),
    /// `expr OR expr`
    Or(Box<HavingExprAst>, Box<HavingExprAst>),
    /// `NOT expr`
    Not(Box<HavingExprAst>),
    /// `agg(col1) op agg(col2)`
    CompareAgg {
        left_agg: String,
        left_col: Expr,
        op: String,
        right_agg: String,
        right_col: Expr,
    },
}
