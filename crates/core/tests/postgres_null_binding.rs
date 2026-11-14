//! EF-PG-01 acceptance: a `DbValue::Null` parameter must be writable to every
//! nullable column type, not only to `TEXT`.
//!
//! The upstream defect: `DbValue::Null` was bound as `None::<String>`, which
//! describes a `TEXT` parameter. PostgreSQL infers a parameter's type from its
//! SQL context (`$1` assigned to a `BIGINT` column is `INT8`), so binding the
//! null to `BIGINT`/`NUMERIC`/`BOOLEAN`/`DATE`/`TIMESTAMP`/`TIMESTAMPTZ`/
//! `UUID`/`BYTEA` failed with "error serializing parameter N" before the
//! statement ran. A real NULL carries no bytes at all and must serialize for
//! every inferred type.
//!
//! This test needs a live server and is `#[ignore]` by default:
//!
//! ```bash
//! export RUST_EF_PG_URL='postgres://test:test@127.0.0.1:5432/rust_ef_test'
//! cargo test -p rust-ef --test postgres_null_binding -- --ignored --nocapture
//! ```
//!
//! The scratch table is named with the process id and a nanosecond timestamp so
//! parallel runs never collide, and it is dropped even when the assertions
//! below fail.

use std::time::{SystemTime, UNIX_EPOCH};

use rust_ef::provider::{DbValue, IDatabaseProvider};
use rust_ef_postgres::PostgresProvider;

/// Nullable columns under test; `id` is the surrogate key used for ordering.
const COLUMNS: [&str; 9] = [
    "c_big", "c_num", "c_bool", "c_date", "c_ts", "c_tstz", "c_uuid", "c_text", "c_bytes",
];

struct Outcome {
    /// The row inserted with [`DbValue::Null`] for every column.
    null_row: Vec<DbValue>,
    /// The row inserted with non-null values (read before the UPDATE).
    some_row: Vec<DbValue>,
    /// `(column IS NOT NULL)` reported by the server for the non-null row.
    some_present: Vec<bool>,
    /// The same row after `UPDATE ... SET <every column> = NULL`.
    updated_row: Vec<DbValue>,
    /// `(column IS NOT NULL)` for the updated row; all must be `false`.
    updated_present: Vec<bool>,
}

fn require_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} must point at a PostgreSQL test database; see the module docs")
    })
}

fn unique_table_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    format!("rust_ef_pg_null_{}_{nanos}", std::process::id())
}

fn error_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn create_table_sql(table: &str) -> String {
    format!(
        "CREATE TABLE \"{table}\" (\
             id BIGSERIAL PRIMARY KEY, \
             c_big BIGINT NULL, \
             c_num NUMERIC(18,6) NULL, \
             c_bool BOOLEAN NULL, \
             c_date DATE NULL, \
             c_ts TIMESTAMP NULL, \
             c_tstz TIMESTAMPTZ NULL, \
             c_uuid UUID NULL, \
             c_text TEXT NULL, \
             c_bytes BYTEA NULL)"
    )
}

fn column_list() -> String {
    COLUMNS.join(", ")
}

/// `INSERT` with no casts: PostgreSQL infers each parameter type from the
/// target column, which is exactly the type the null parameter must serialize
/// for.
fn insert_nulls_sql(table: &str) -> String {
    let placeholders: Vec<String> = (1..=COLUMNS.len())
        .map(|index| format!("${index}"))
        .collect();
    format!(
        "INSERT INTO \"{table}\" ({}) VALUES ({})",
        column_list(),
        placeholders.join(", ")
    )
}

/// `INSERT` of concrete values. Numeric/temporal/UUID text is cast explicitly
/// so the test does not depend on the `chrono`/`uuid`/`decimal` features being
/// enabled for this test target.
fn insert_some_sql(table: &str) -> String {
    format!(
        "INSERT INTO \"{table}\" ({}) VALUES (\
             $1, $2::text::numeric, $3, $4::text::date, $5::text::timestamp, \
             $6::text::timestamptz, $7::text::uuid, $8, $9)",
        column_list()
    )
}

/// `UPDATE` that sets every nullable column to a parameter, so the null-binding
/// matrix also covers the UPDATE path (the upstream report reproduced both
/// INSERT and UPDATE failures).
fn update_nulls_sql(table: &str) -> String {
    let assignments: Vec<String> = COLUMNS
        .iter()
        .enumerate()
        .map(|(index, column)| format!("{column} = ${}", index + 1))
        .collect();
    format!(
        "UPDATE \"{table}\" SET {} WHERE id = (SELECT MAX(id) FROM \"{table}\")",
        assignments.join(", ")
    )
}

fn some_values() -> Vec<DbValue> {
    vec![
        DbValue::I64(42),
        DbValue::String("1234.567890".to_string()),
        DbValue::Bool(true),
        DbValue::String("2026-01-02".to_string()),
        DbValue::String("2026-01-02 03:04:05".to_string()),
        DbValue::String("2026-01-02 03:04:05+00".to_string()),
        DbValue::String("123e4567-e89b-12d3-a456-426614174000".to_string()),
        DbValue::String("hello null".to_string()),
        DbValue::Bytes(vec![0x00, 0x01, 0x02, 0xff]),
    ]
}

/// Runs the whole matrix, returning the rows to assert on. Errors are returned
/// instead of panicking so the caller can always drop the scratch table first.
async fn run_case(provider: &dyn IDatabaseProvider, table: &str) -> Result<Outcome, String> {
    let mut connection = provider.get_connection().await.map_err(error_string)?;
    connection
        .execute(&create_table_sql(table), &[])
        .await
        .map_err(error_string)?;

    let null_params = vec![DbValue::Null; COLUMNS.len()];
    connection
        .execute(&insert_nulls_sql(table), &null_params)
        .await
        .map_err(|error| {
            format!(
                "EF-PG-01: DbValue::Null failed for a nullable column set \
                 ({COLUMNS:?}) on '{table}': {error}"
            )
        })?;

    connection
        .execute(&insert_some_sql(table), &some_values())
        .await
        .map_err(|error| format!("control insert with non-null values failed: {error}"))?;

    let select = format!("SELECT {} FROM \"{table}\" ORDER BY id", column_list());
    let rows = connection.query(&select, &[]).await.map_err(error_string)?;
    if rows.len() != 2 {
        return Err(format!("expected 2 rows in '{table}', got {}", rows.len()));
    }

    let flags_select = format!(
        "SELECT {} FROM \"{table}\" ORDER BY id",
        COLUMNS
            .iter()
            .map(|column| format!("({column} IS NOT NULL)"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let flag_rows = connection
        .query(&flags_select, &[])
        .await
        .map_err(error_string)?;
    if flag_rows.len() != 2 {
        return Err(format!(
            "expected 2 presence rows in '{table}', got {}",
            flag_rows.len()
        ));
    }
    let some_present = flag_rows[1]
        .iter()
        .map(|value| bool::try_from(value.clone()).map_err(error_string))
        .collect::<Result<Vec<bool>, String>>()?;

    // Same nullable columns, UPDATE path.
    connection
        .execute(&update_nulls_sql(table), &null_params)
        .await
        .map_err(|error| {
            format!("EF-PG-01: UPDATE with DbValue::Null failed on '{table}': {error}")
        })?;

    let updated_rows = connection.query(&select, &[]).await.map_err(error_string)?;
    if updated_rows.len() != 2 {
        return Err(format!(
            "expected 2 rows after the UPDATE on '{table}', got {}",
            updated_rows.len()
        ));
    }
    let updated_flags = connection
        .query(&flags_select, &[])
        .await
        .map_err(error_string)?;
    let updated_present = updated_flags[1]
        .iter()
        .map(|value| bool::try_from(value.clone()).map_err(error_string))
        .collect::<Result<Vec<bool>, String>>()?;

    Ok(Outcome {
        null_row: rows[0].clone(),
        some_row: rows[1].clone(),
        some_present,
        updated_row: updated_rows[1].clone(),
        updated_present,
    })
}

async fn drop_table(provider: &dyn IDatabaseProvider, table: &str) -> Result<(), String> {
    let mut connection = provider.get_connection().await.map_err(error_string)?;
    connection
        .execute(&format!("DROP TABLE IF EXISTS \"{table}\""), &[])
        .await
        .map_err(error_string)?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a PostgreSQL server (RUST_EF_PG_URL)"]
async fn postgres_binds_null_for_every_nullable_column_type() {
    let url = require_env("RUST_EF_PG_URL");
    let provider = PostgresProvider::new_insecure(&url, 1).expect("PostgreSQL provider");
    let table = unique_table_name();

    let outcome = run_case(&provider, &table).await;
    // Drop the scratch table before asserting so a failed assertion cannot leak
    // it; a failed setup is surfaced below with its own message.
    let cleanup = drop_table(&provider, &table).await;
    let outcome = outcome.unwrap_or_else(|error| panic!("EF-PG-01 case failed: {error}"));
    cleanup.unwrap_or_else(|error| panic!("dropping scratch table '{table}' failed: {error}"));

    // --- DbValue::Null must round-trip for every column type -----------------
    assert_eq!(
        outcome.null_row.len(),
        COLUMNS.len(),
        "the NULL insert must return every column: {:?}",
        outcome.null_row
    );
    for (column, value) in COLUMNS.iter().zip(outcome.null_row.iter()) {
        assert_eq!(
            *value,
            DbValue::Null,
            "EF-PG-01: a NULL written to nullable column '{column}' must read \
             back as DbValue::Null, got {value:?}"
        );
    }

    // --- The same columns must accept NULL through UPDATE -------------------
    for (column, value) in COLUMNS.iter().zip(outcome.updated_row.iter()) {
        assert_eq!(
            *value,
            DbValue::Null,
            "EF-PG-01: UPDATE ... SET {column} = NULL must read back as \
             DbValue::Null, got {value:?}"
        );
    }
    assert_eq!(
        outcome.updated_present,
        vec![false; COLUMNS.len()],
        "the server must report every updated column as NULL: {:?}",
        outcome.updated_present
    );

    // --- The control row must really be non-null on the server --------------
    assert_eq!(
        outcome.some_present,
        vec![true; COLUMNS.len()],
        "the non-null control row must store non-null values for every column: {:?}",
        outcome.some_present
    );

    let some = &outcome.some_row;
    assert_eq!(
        some[0],
        DbValue::I64(42),
        "BIGINT control value: {:?}",
        some[0]
    );
    assert_eq!(
        some[2],
        DbValue::Bool(true),
        "BOOLEAN control value: {:?}",
        some[2]
    );
    assert_eq!(
        some[7],
        DbValue::String("hello null".to_string()),
        "TEXT control value: {:?}",
        some[7]
    );
    assert_eq!(
        some[8],
        DbValue::Bytes(vec![0x00, 0x01, 0x02, 0xff]),
        "BYTEA control value: {:?}",
        some[8]
    );
    // The non-NULL NUMERIC must decode to `DbValue::Decimal`, not to NULL.
    // (`DbValue::Decimal` is enabled by the provider's `decimal` feature; the
    // value itself is checked through the `matches!` pattern because the test
    // target does not depend on `rust_decimal` directly.)
    assert!(
        matches!(some[1], DbValue::Decimal(_)),
        "NUMERIC control value must decode to DbValue::Decimal, got {:?}",
        some[1]
    );
}
