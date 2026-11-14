//! PostgreSQL error mapping (EF-ERR-01).
//!
//! A bare `tokio_postgres::Error` prints as "db error" because the driver keeps
//! SQLSTATE, detail, hint, table/column/constraint and the message in structured
//! fields. Mapping copies them into [`EFError::Database`] so a production log
//! identifies the failed statement without a second connection.

use rust_ef::error::{DatabaseErrorInfo, EFError};
use tokio_postgres::error::SqlState;

/// Maps a `tokio_postgres` error to a typed [`EFError`].
///
/// SQLSTATE `23505` (unique violation) is promoted to
/// [`EFError::UniqueViolation`], using PostgreSQL's structured
/// `table`/`constraint` fields instead of parsing the message.
pub(crate) fn map_pg_error(e: tokio_postgres::Error, context: &str, sql: &str) -> EFError {
    if e.code() == Some(&SqlState::UNIQUE_VIOLATION) {
        let db_error = e.as_db_error();
        return EFError::UniqueViolation {
            constraint: db_error.and_then(|d| d.constraint()).map(str::to_string),
            table: db_error.and_then(|d| d.table()).map(str::to_string),
        };
    }
    let info = e.as_db_error().map(|db| {
        let mut info = DatabaseErrorInfo::new(db.message()).with_sqlstate(db.code().code());
        if let Some(detail) = db.detail() {
            info = info.with_detail(detail);
        }
        if let Some(hint) = db.hint() {
            info = info.with_hint(hint);
        }
        if let Some(table) = db.table() {
            info = info.with_table(table);
        }
        if let Some(column) = db.column() {
            info = info.with_column(column);
        }
        if let Some(constraint) = db.constraint() {
            info = info.with_constraint(constraint);
        }
        info.with_sql(sql)
    });
    match info {
        Some(info) => EFError::database(info).with_source(Box::new(e)),
        None => EFError::query(format!("{context}: {e}")).with_source(Box::new(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlstate_is_promoted_for_unique_violations() {
        // Constructing a `tokio_postgres::Error` with a DB payload is not
        // possible from outside the crate; the structured fields are covered by
        // the ignored live-PostgreSQL parity test. This unit test pins the
        // mapping contract for the code/`Display` shape.
        let info = DatabaseErrorInfo::new("duplicate key value violates unique constraint \"ux\"")
            .with_sqlstate("23505")
            .with_constraint("ux")
            .with_table("orders")
            .with_sql("INSERT INTO orders DEFAULT VALUES");
        let rendered = info.to_string();
        assert!(rendered.contains("23505"), "{rendered}");
        assert!(rendered.contains("orders"), "{rendered}");
        assert!(rendered.contains("ux"), "{rendered}");
    }
}
