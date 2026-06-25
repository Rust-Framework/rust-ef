//! PostgreSQL integration tests.
//!
//! Set `RUST_EF_PG_URL` (default in CI: `postgres://test:test@localhost:5432/rust_ef_test`).

mod common;

use common::run_crud_lifecycle;
use rust_ef_postgres::PostgresProvider;
use std::sync::Arc;

fn pg_url() -> Option<String> {
    std::env::var("RUST_EF_PG_URL").ok().or_else(|| {
        if std::env::var("CI").is_ok() {
            Some("postgres://test:test@localhost:5432/rust_ef_test".into())
        } else {
            None
        }
    })
}

#[tokio::test]
async fn test_postgres_crud_lifecycle() {
    let Some(url) = pg_url() else {
        eprintln!("skip test_postgres_crud_lifecycle: RUST_EF_PG_URL not set");
        return;
    };

    let provider = match PostgresProvider::new(&url, 5) {
        Ok(p) => Arc::new(p) as Arc<dyn rust_ef::provider::IDatabaseProvider>,
        Err(e) => {
            eprintln!("skip test_postgres_crud_lifecycle: {e}");
            return;
        }
    };

    run_crud_lifecycle(
        provider,
        rust_ef::migration::MigrationDialect::Postgres,
    )
    .await
    .expect("postgres CRUD lifecycle");
}
