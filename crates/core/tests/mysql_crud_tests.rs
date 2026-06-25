//! MySQL integration tests.
//!
//! Set `RUST_EF_MYSQL_URL` (default in CI: `mysql://root:test@localhost:3306/rust_ef_test`).

mod common;

use common::run_crud_lifecycle;
use rust_ef_mysql::MySqlProvider;
use std::sync::Arc;

fn mysql_url() -> Option<String> {
    std::env::var("RUST_EF_MYSQL_URL").ok().or_else(|| {
        if std::env::var("CI").is_ok() {
            Some("mysql://root:test@localhost:3306/rust_ef_test".into())
        } else {
            None
        }
    })
}

#[tokio::test]
async fn test_mysql_crud_lifecycle() {
    let Some(url) = mysql_url() else {
        eprintln!("skip test_mysql_crud_lifecycle: RUST_EF_MYSQL_URL not set");
        return;
    };

    let provider = match MySqlProvider::new(&url).await {
        Ok(p) => Arc::new(p) as Arc<dyn rust_ef::provider::IDatabaseProvider>,
        Err(e) => {
            eprintln!("skip test_mysql_crud_lifecycle: {e}");
            return;
        }
    };

    run_crud_lifecycle(provider, rust_ef::migration::MigrationDialect::MySql)
        .await
        .expect("mysql CRUD lifecycle");
}
