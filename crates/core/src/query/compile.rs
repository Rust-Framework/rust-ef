//! SQL compilation helpers and the portable placeholder generator.
//!
//! `compile_bool_expr` compiles a `BoolExpr` tree into a parameterized SQL
//! WHERE fragment using the provider's `ISqlGenerator` for placeholder
//! syntax. `collect_bool_expr_values` extracts inline parameter values from
//! self-contained `FilterCondition`s produced by `linq!(|b: T| ...)`.
//!
//! `PortablePlaceholderGenerator` is a fallback `ISqlGenerator` used for
//! SQL-only queries without an attached provider (emits `?` placeholders and
//! `"identifier"` quoting).

use crate::error::EFResult;
use crate::metadata::EntityTypeMeta;
use crate::provider::{DbValue, DbValueConvertError};

use super::ast::{BoolExpr, InSubquerySpec, SubquerySpec};
use super::state::QueryState;

// ---------------------------------------------------------------------------
// Portable placeholder generator
// ---------------------------------------------------------------------------

/// Fallback generator for SQL-only queries without an attached provider.
pub(crate) struct PortablePlaceholderGenerator;

impl crate::provider::ISqlGenerator for PortablePlaceholderGenerator {
    fn select(&self, _: &str, _: &[&str]) -> String {
        String::new()
    }
    fn insert(&self, _: &str, _: &[&str], _: bool) -> String {
        String::new()
    }
    fn update(&self, _: &str, _: &[&str], _: &str) -> String {
        String::new()
    }
    fn delete(&self, _: &str, _: &str) -> String {
        String::new()
    }
    fn create_table(&self, _: &str, _: &[(String, String)]) -> String {
        String::new()
    }
    fn drop_table(&self, _: &str) -> String {
        String::new()
    }
    fn pagination(&self, skip: Option<usize>, take: Option<usize>) -> String {
        // Portable fallback: emit the most widely-supported form
        // `LIMIT x OFFSET y`. SQLite and MySQL accept this verbatim;
        // PostgreSQL also accepts this clause order (though it prefers
        // `OFFSET x LIMIT y`). Offset-only without a LIMIT is only
        // supported by PostgreSQL and SQLite, so we emit `OFFSET y` and
        // rely on the caller to attach a real provider when targeting
        // MySQL (whose offset-only case requires a sentinel LIMIT).
        match (skip, take) {
            (Some(s), Some(t)) => format!("LIMIT {} OFFSET {}", t, s),
            (None, Some(t)) => format!("LIMIT {}", t),
            (Some(s), None) => format!("OFFSET {}", s),
            (None, None) => String::new(),
        }
    }
    fn parameter_placeholder(&self, _: usize) -> String {
        "?".to_string()
    }
    fn quote_identifier(&self, identifier: &str) -> String {
        identifier
            .split('.')
            .map(|part| format!("\"{}\"", part.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(".")
    }
    fn auto_increment_syntax(&self) -> &'static str {
        "AUTOINCREMENT"
    }
}

// ---------------------------------------------------------------------------
// Aggregate cell conversion
// ---------------------------------------------------------------------------

/// Converts the first cell of the first row of an aggregation query result
/// into the target type `V`. Returns `None` when no rows, no cells, or a SQL
/// NULL was returned (e.g. `MIN`/`MAX` over an empty input set). The driver
/// returns native `DbValue` cells, so `TryFrom` conversion is applied
/// directly.
pub(crate) fn convert_aggregate_cell<V>(rows: Vec<Vec<DbValue>>) -> EFResult<Option<V>>
where
    V: TryFrom<DbValue, Error = DbValueConvertError>,
{
    match rows.first().and_then(|r| r.first()) {
        Some(DbValue::Null) => Ok(None),
        Some(cell) => V::try_from(cell.clone())
            .map(Some)
            .map_err(crate::error::EFError::from),
        None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// BoolExpr compilation
// ---------------------------------------------------------------------------

/// Compiles a `BoolExpr` tree into a SQL fragment, advancing `param_idx` and
/// collecting the bound values **in the same pass**.
///
/// This is the single boolean compiler for WHERE, HAVING (lowered from
/// `HavingExpr`), CTE predicates, subquery predicates and DML filters: SQL text
/// order and the parameter vector cannot drift because they are produced
/// together.
pub(crate) fn compile_bool_expr_into(
    expr: &BoolExpr,
    gen: &dyn crate::provider::ISqlGenerator,
    param_idx: &mut usize,
    params: &mut Vec<DbValue>,
) -> String {
    match expr {
        BoolExpr::Filter(filter) => filter.compile(gen, param_idx, params),
        BoolExpr::Raw(raw) => {
            emit_raw_fragment(&raw.0, &[], gen, param_idx, params, "BoolExpr::Raw")
        }
        BoolExpr::And(a, b) => format!(
            "({}) AND ({})",
            compile_bool_expr_into(a, gen, param_idx, params),
            compile_bool_expr_into(b, gen, param_idx, params)
        ),
        BoolExpr::Or(a, b) => format!(
            "({}) OR ({})",
            compile_bool_expr_into(a, gen, param_idx, params),
            compile_bool_expr_into(b, gen, param_idx, params)
        ),
        BoolExpr::Not(inner) => format!(
            "NOT ({})",
            compile_bool_expr_into(inner, gen, param_idx, params)
        ),
        BoolExpr::Compare { left, op, right } => format!(
            "{} {} {}",
            left.compile(gen, param_idx, params),
            op.sql_name(),
            right.compile(gen, param_idx, params)
        ),
        BoolExpr::Exists(spec) => compile_subquery(spec, gen, param_idx, params, false),
        BoolExpr::NotExists(spec) => compile_subquery(spec, gen, param_idx, params, true),
        BoolExpr::InSubquery(spec) => compile_in_subquery(spec, gen, param_idx, params, false),
        BoolExpr::NotInSubquery(spec) => compile_in_subquery(spec, gen, param_idx, params, true),
    }
}

/// SQL-only compilation (no parameter values are returned). Prefer
/// [`compile_bool_expr_into`] when the values are needed as well; this wrapper
/// exists for call sites that render SQL only (lazy/navigation loaders, DDL
/// expressions) and must not execute with placeholders.
pub(crate) fn compile_bool_expr(
    expr: &BoolExpr,
    gen: &dyn crate::provider::ISqlGenerator,
    param_idx: &mut usize,
) -> String {
    let mut discarded = Vec::new();
    compile_bool_expr_into(expr, gen, param_idx, &mut discarded)
}

/// G5: compiles a `SubquerySpec` into `EXISTS (SELECT 1 FROM ...)` SQL.
fn compile_subquery(
    spec: &SubquerySpec,
    gen: &dyn crate::provider::ISqlGenerator,
    param_idx: &mut usize,
    params: &mut Vec<DbValue>,
    negated: bool,
) -> String {
    let related_tbl = gen.quote_identifier(&spec.related_table);
    let fk_col = gen.quote_identifier(&spec.fk_column);
    let outer_tbl = gen.quote_identifier(&spec.outer_table);
    let outer_pk = gen.quote_identifier(&spec.outer_pk_column);

    let mut sql = if negated {
        format!("NOT EXISTS (SELECT 1 FROM {related_tbl} WHERE {related_tbl}.{fk_col} = {outer_tbl}.{outer_pk}")
    } else {
        format!("EXISTS (SELECT 1 FROM {related_tbl} WHERE {related_tbl}.{fk_col} = {outer_tbl}.{outer_pk}")
    };

    if let Some(pred) = &spec.predicate {
        let pred_sql = compile_bool_expr_into(pred, gen, param_idx, params);
        // The predicate may be an OR tree; without parentheses the outer AND
        // would re-associate as `(fk = outer AND a) OR b`.
        sql.push_str(&format!(" AND ({pred_sql})"));
    }
    sql.push(')');
    sql
}

/// Compiles an `InSubquerySpec` into
/// `column IN (SELECT projection FROM source_table [WHERE predicate])` SQL.
fn compile_in_subquery(
    spec: &InSubquerySpec,
    gen: &dyn crate::provider::ISqlGenerator,
    param_idx: &mut usize,
    params: &mut Vec<DbValue>,
    negated: bool,
) -> String {
    let outer_col = gen.quote_identifier(&spec.outer_column);
    let src_tbl = gen.quote_identifier(&spec.source_table);
    let proj_col = gen.quote_identifier(&spec.projection_column);

    let kw = if negated { "NOT IN" } else { "IN" };
    let mut sql = format!("{outer_col} {kw} (SELECT {proj_col} FROM {src_tbl}");

    if let Some(pred) = &spec.predicate {
        let pred_sql = compile_bool_expr_into(pred, gen, param_idx, params);
        sql.push_str(&format!(" WHERE {pred_sql}"));
    }
    sql.push(')');
    sql
}

/// Collects the inline values of a self-contained `BoolExpr` in SQL text order.
///
/// Compatibility helper for call sites that only need the values (DTO/statement
/// assembly); new code should prefer [`compile_bool_expr_into`], which produces
/// SQL and values together.
pub(crate) fn collect_bool_expr_values(expr: &BoolExpr) -> Vec<DbValue> {
    let mut param_idx = 1usize;
    let mut params = Vec::new();
    let _ = compile_bool_expr_into(
        expr,
        &PortablePlaceholderGenerator,
        &mut param_idx,
        &mut params,
    );
    params
}

/// One lexed segment of a pre-compiled raw SQL fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RawSegment {
    /// Verbatim SQL (string literals, quoted identifiers, comments and ordinary
    /// text); `??` has already been decoded to a literal `?`.
    Verbatim(String),
    /// A `?` placeholder (`index == 0`) or a PostgreSQL `$N` placeholder.
    Placeholder(usize),
}

/// Lexes a raw SQL fragment, distinguishing real placeholders from `?`/`$N`
/// that appear inside string literals, quoted identifiers, dollar-quoted
/// strings or comments.
///
/// `??` escapes a literal question mark (e.g. PostgreSQL's `jsonb ?` operator),
/// and `????` escapes two literal question marks.
fn lex_raw_fragment(sql: &str) -> EFResult<Vec<RawSegment>> {
    let bytes = sql.as_bytes();
    let mut segments: Vec<RawSegment> = Vec::new();
    let mut text = String::new();
    let mut i = 0usize;

    fn flush(text: &mut String, segments: &mut Vec<RawSegment>) {
        if !text.is_empty() {
            segments.push(RawSegment::Verbatim(std::mem::take(text)));
        }
    }

    // Copies the UTF-8 character starting at `*i` and advances past it. The
    // lexer walks byte offsets to recognise ASCII syntax, but raw fragments may
    // contain non-ASCII text (literals, quoted identifiers, comments); pushing
    // `bytes[i] as char` would split a multi-byte character into Latin-1 bytes.
    fn push_char(sql: &str, i: &mut usize, text: &mut String) {
        let ch = sql[*i..]
            .chars()
            .next()
            .expect("lexer offsets always sit on a character boundary");
        text.push(ch);
        *i += ch.len_utf8();
    }

    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' | b'`' => {
                let quote = bytes[i];
                text.push(quote as char);
                i += 1;
                let mut closed = false;
                while i < bytes.len() {
                    if bytes[i] == quote {
                        text.push(quote as char);
                        i += 1;
                        if i < bytes.len() && bytes[i] == quote {
                            text.push(quote as char);
                            i += 1;
                            continue;
                        }
                        closed = true;
                        break;
                    }
                    push_char(sql, &mut i, &mut text);
                }
                if !closed {
                    return Err(crate::error::EFError::configuration(
                        "unterminated quoted literal in a raw SQL fragment".to_string(),
                    ));
                }
            }
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    push_char(sql, &mut i, &mut text);
                }
            }
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                let mut depth = 1usize;
                text.push_str("/*");
                i += 2;
                while i < bytes.len() && depth > 0 {
                    if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                        depth += 1;
                        text.push_str("/*");
                        i += 2;
                    } else if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        depth -= 1;
                        text.push_str("*/");
                        i += 2;
                    } else {
                        push_char(sql, &mut i, &mut text);
                    }
                }
                if depth != 0 {
                    return Err(crate::error::EFError::configuration(
                        "unterminated block comment in a raw SQL fragment".to_string(),
                    ));
                }
            }
            b'?' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'?' {
                    // Escaped literal `?` (e.g. PostgreSQL `jsonb ?` operator).
                    text.push('?');
                    i += 2;
                } else {
                    flush(&mut text, &mut segments);
                    segments.push(RawSegment::Placeholder(0));
                    i += 1;
                }
            }
            b'$' => {
                // Dollar-quoted string (`$$...$$` / `$tag$...$tag$`)?
                let mut tag_end = i + 1;
                while tag_end < bytes.len()
                    && (bytes[tag_end].is_ascii_alphanumeric() || bytes[tag_end] == b'_')
                {
                    tag_end += 1;
                }
                if tag_end < bytes.len() && bytes[tag_end] == b'$' {
                    let tag = &sql[i..=tag_end];
                    text.push_str(tag);
                    i = tag_end + 1;
                    let closing = tag.to_string();
                    match sql[i..].find(&closing) {
                        Some(pos) => {
                            let end = i + pos + closing.len();
                            text.push_str(&sql[i..end]);
                            i = end;
                        }
                        None => {
                            return Err(crate::error::EFError::configuration(
                                "unterminated dollar-quoted string in a raw SQL fragment"
                                    .to_string(),
                            ));
                        }
                    }
                } else if tag_end > i + 1 && tag_end <= bytes.len() {
                    // A numeric placeholder: `$` followed by digits.
                    match sql[i + 1..tag_end].parse::<usize>() {
                        Ok(index) if index > 0 => {
                            flush(&mut text, &mut segments);
                            segments.push(RawSegment::Placeholder(index));
                            i = tag_end;
                        }
                        _ => {
                            text.push('$');
                            i += 1;
                        }
                    }
                } else {
                    text.push('$');
                    i += 1;
                }
            }
            _ => {
                let ch_len = sql[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                text.push_str(&sql[i..i + ch_len]);
                i += ch_len;
            }
        }
    }
    flush(&mut text, &mut segments);
    Ok(segments)
}

/// Validates a pre-compiled raw SQL fragment against its parameter values.
///
/// Placeholders are recognised outside literals/comments only (see
/// [`lex_raw_fragment`]); `??` is a literal question mark. Supported shapes are
/// `?` placeholders (count must equal the value count) or provider-compiled
/// `$N` placeholders whose highest index must equal the value count. Mixing the
/// two styles is rejected. A fragment without bound values may contain any text
/// (e.g. a PostgreSQL `jsonb ?` operator written as `??` or a raw literal).
pub(crate) fn validate_raw_fragment(sql: &str, values: &[DbValue], context: &str) -> EFResult<()> {
    let segments = lex_raw_fragment(sql)?;
    if values.is_empty() {
        return Ok(());
    }
    let mut portable = 0usize;
    let mut highest = 0usize;
    let mut numbered = false;
    for segment in &segments {
        match segment {
            RawSegment::Placeholder(0) => portable += 1,
            RawSegment::Placeholder(index) => {
                numbered = true;
                highest = highest.max(*index);
            }
            RawSegment::Verbatim(_) => {}
        }
    }
    if numbered && portable > 0 {
        return Err(crate::error::EFError::configuration(format!(
            "{context} mixes `?` and `$N` placeholders; use one placeholder style"
        )));
    }
    if numbered {
        if highest != values.len() {
            return Err(crate::error::EFError::configuration(format!(
                "{context} has {highest} numbered placeholder(s) but {} bound value(s)",
                values.len()
            )));
        }
    } else if portable != values.len() {
        return Err(crate::error::EFError::configuration(format!(
            "{context} has {portable} `?` placeholder(s) but {} bound value(s)",
            values.len()
        )));
    }
    Ok(())
}

/// Emits a pre-compiled SQL fragment and its parameter values, adapting
/// placeholders to the provider.
///
/// The fragment is lexed so placeholders inside string literals, quoted
/// identifiers, dollar-quoted strings and comments are left untouched. `??`
/// decodes to a literal `?`. Portable `?` fragments are substituted with the
/// provider's placeholder syntax; provider-compiled `$N` fragments are shifted
/// past the parameters already emitted.
pub(crate) fn emit_raw_fragment(
    sql: &str,
    values: &[DbValue],
    gen: &dyn crate::provider::ISqlGenerator,
    param_idx: &mut usize,
    params: &mut Vec<DbValue>,
    context: &str,
) -> String {
    let segments = match lex_raw_fragment(sql) {
        Ok(segments) => segments,
        Err(error) => {
            debug_assert!(false, "{context}: {error}");
            return sql.to_string();
        }
    };
    if values.is_empty() {
        *param_idx += 0;
        return segments
            .into_iter()
            .map(|segment| match segment {
                RawSegment::Verbatim(text) => text,
                RawSegment::Placeholder(0) => "?".to_string(),
                RawSegment::Placeholder(index) => format!("${index}"),
            })
            .collect();
    }

    let mut out = String::with_capacity(sql.len());
    let mut used = 0usize;
    let numbered_offset = param_idx.saturating_sub(1);
    for segment in segments {
        match segment {
            RawSegment::Verbatim(text) => out.push_str(&text),
            RawSegment::Placeholder(0) => {
                if used < values.len() {
                    out.push_str(&gen.parameter_placeholder(*param_idx));
                    *param_idx += 1;
                    params.push(values[used].clone());
                    used += 1;
                } else {
                    out.push('?');
                }
            }
            RawSegment::Placeholder(index) => {
                if gen.uses_numbered_placeholders() {
                    out.push('$');
                    out.push_str(&(index + numbered_offset).to_string());
                } else {
                    // A `$N` fragment on a non-numbered provider is a caller
                    // error; validation runs in the checked compile path.
                    out.push('$');
                    out.push_str(&index.to_string());
                }
            }
        }
    }
    if gen.uses_numbered_placeholders() {
        // A provider-compiled `$N` fragment: all values belong to it and were
        // already shifted by `numbered_offset`.
        if used == 0 {
            *param_idx += values.len();
            params.extend(values.iter().cloned());
        } else {
            debug_assert_eq!(used, values.len(), "{context}: placeholder/value mismatch");
        }
    } else {
        *param_idx += values.len().saturating_sub(used);
        params.extend(values[used..].iter().cloned());
    }
    out
}

// ---------------------------------------------------------------------------
// Subquery resolution
// ---------------------------------------------------------------------------

/// G5: Resolves `SubquerySpec` table/column fields by looking up navigation
/// metadata from the outer entity's `EntityTypeMeta`. Must be called before
/// `compile_bool_expr` when the expression tree contains `Exists`/`NotExists`.
pub(crate) fn resolve_subqueries(expr: &mut BoolExpr, outer_meta: &EntityTypeMeta) {
    match expr {
        BoolExpr::And(a, b) | BoolExpr::Or(a, b) => {
            resolve_subqueries(a, outer_meta);
            resolve_subqueries(b, outer_meta);
        }
        BoolExpr::Not(inner) => resolve_subqueries(inner, outer_meta),
        BoolExpr::Exists(spec) | BoolExpr::NotExists(spec) => {
            resolve_subquery_spec(spec, outer_meta);
            if let Some(pred) = &mut spec.predicate {
                // The predicate references the related entity, but we don't
                // have its metadata here. Predicate fields are compiled as
                // raw column names (e.g. "published"), which works for
                // single-table subqueries.
                let _ = pred;
            }
        }
        _ => {}
    }
}

/// Returns `true` if the expression tree contains any correlated subquery node
/// (`EXISTS` / `IN (SELECT ...)`).
pub(crate) fn has_any_subquery(expr: &BoolExpr) -> bool {
    match expr {
        BoolExpr::Exists(_)
        | BoolExpr::NotExists(_)
        | BoolExpr::InSubquery(_)
        | BoolExpr::NotInSubquery(_) => true,
        BoolExpr::And(a, b) | BoolExpr::Or(a, b) => has_any_subquery(a) || has_any_subquery(b),
        BoolExpr::Not(inner) => has_any_subquery(inner),
        // Scalar comparisons cannot contain subquery nodes.
        BoolExpr::Compare { .. } => false,
        BoolExpr::Filter(_) | BoolExpr::Raw(_) => false,
    }
}

/// Returns `true` if the expression tree contains any `Exists`/`NotExists`
/// subquery nodes. Used to avoid the `T::entity_meta()` call (which may be
/// `unimplemented!()` in unit tests) when no subqueries are present.
pub(crate) fn has_subqueries(expr: &BoolExpr) -> bool {
    match expr {
        BoolExpr::Exists(_) | BoolExpr::NotExists(_) => true,
        BoolExpr::And(a, b) | BoolExpr::Or(a, b) => has_subqueries(a) || has_subqueries(b),
        BoolExpr::Not(inner) => has_subqueries(inner),
        _ => false,
    }
}

fn resolve_subquery_spec(spec: &mut SubquerySpec, outer_meta: &EntityTypeMeta) {
    // Find the navigation by field name
    let nav = outer_meta
        .navigations
        .iter()
        .find(|n| n.field_name.as_ref() == spec.navigation_field);

    if let Some(nav) = nav {
        // Related table: prefer the navigation's `related_table` (actual DB
        // table name, e.g. "posts"); fall back to the related type name.
        spec.related_table = nav
            .related_table
            .as_ref()
            .map(|s| s.as_ref().to_string())
            .unwrap_or_else(|| spec.related_type_name.clone());

        // FK column on the related/dependent table (e.g. "blog_id").
        if let Some(fk) = &nav.fk_column {
            spec.fk_column = fk.as_ref().to_string();
        } else if let Some(fk) = &nav.foreign_key_field {
            spec.fk_column = fk.as_ref().to_string();
        }

        // Outer PK: prefer the navigation's `referenced_key_column`, fall
        // back to the outer entity's first primary key.
        if let Some(pk) = &nav.referenced_key_column {
            spec.outer_pk_column = pk.as_ref().to_string();
        } else if let Some(pk) = outer_meta.primary_keys.first() {
            spec.outer_pk_column = pk.as_ref().to_string();
        }

        // Outer table: from the outer entity's table_name
        spec.outer_table = outer_meta.table_name.to_string();
    }
}

/// Fills in the resolved table/column fields of correlated subqueries in
/// `state.where_expr` (mutates in place).
///
/// Must run before compiling a WHERE clause that may contain `EXISTS` /
/// `IN (SELECT ...)` nodes — every entry point that compiles SQL for execution
/// (`to_sql`, `first`, `count`, `execute_update`, `execute_delete`) goes
/// through it. It is a no-op when the predicate has no subqueries, so entities
/// whose `entity_meta()` is `unimplemented!()` remain usable for SQL-shape tests.
pub(crate) fn resolve_state_subqueries<T: crate::entity::IEntityType>(state: &mut QueryState) {
    let needs_outer_meta = state.where_expr.as_ref().is_some_and(has_subqueries)
        || state
            .ctes
            .iter()
            .filter(|cte| !cte.table.is_empty())
            .filter_map(|cte| cte.where_expr.as_ref())
            .any(has_subqueries);
    if !needs_outer_meta {
        return;
    }
    let outer_meta = T::entity_meta();
    if let Some(expr) = &mut state.where_expr {
        if has_subqueries(expr) {
            resolve_subqueries(expr, &outer_meta);
        }
    }
    // Typed CTE bodies are compiled by the same pipeline; their EXISTS/IN
    // predicates must be resolved before compilation too.
    for cte in &mut state.ctes {
        if cte.table.is_empty() {
            continue;
        }
        if let Some(expr) = &mut cte.where_expr {
            if has_subqueries(expr) {
                resolve_subqueries(expr, &outer_meta);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ISqlGenerator;

    #[test]
    fn raw_sql_fragment_compiles_unchanged() {
        let expr = BoolExpr::raw("is_deleted = 0");
        let gen = PortablePlaceholderGenerator;
        let mut idx = 1;
        let sql = compile_bool_expr(&expr, &gen, &mut idx);
        assert_eq!(sql, "is_deleted = 0");
        assert_eq!(idx, 1);
    }

    #[test]
    fn quote_identifier_escapes_embedded_quotes() {
        let gen = PortablePlaceholderGenerator;
        assert_eq!(gen.quote_identifier("plain"), "\"plain\"");
        assert_eq!(gen.quote_identifier("a.b"), "\"a\".\"b\"");
        assert_eq!(gen.quote_identifier("we\"ird"), "\"we\"\"ird\"");
    }
}
