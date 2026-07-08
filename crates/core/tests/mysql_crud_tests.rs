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

#[tokio::test]
async fn test_mysql_filter_with_in_operator() {
    let Some(url) = mysql_url() else {
        eprintln!("skip test_mysql_filter_with_in_operator: RUST_EF_MYSQL_URL not set");
        return;
    };
    let provider = match MySqlProvider::new(&url).await {
        Ok(p) => Arc::new(p) as Arc<dyn rust_ef::provider::IDatabaseProvider>,
        Err(e) => {
            eprintln!("skip test_mysql_filter_with_in_operator: {e}");
            return;
        }
    };
    common::run_filter_with_in_operator(provider, rust_ef::migration::MigrationDialect::MySql)
        .await
        .expect("mysql filter with IN operator");
}

#[tokio::test]
async fn test_mysql_limit_and_offset() {
    let Some(url) = mysql_url() else {
        eprintln!("skip test_mysql_limit_and_offset: RUST_EF_MYSQL_URL not set");
        return;
    };
    let provider = match MySqlProvider::new(&url).await {
        Ok(p) => Arc::new(p) as Arc<dyn rust_ef::provider::IDatabaseProvider>,
        Err(e) => {
            eprintln!("skip test_mysql_limit_and_offset: {e}");
            return;
        }
    };
    common::run_limit_and_offset(provider, rust_ef::migration::MigrationDialect::MySql)
        .await
        .expect("mysql limit and offset");
}

#[tokio::test]
async fn test_mysql_count_and_any() {
    let Some(url) = mysql_url() else {
        eprintln!("skip test_mysql_count_and_any: RUST_EF_MYSQL_URL not set");
        return;
    };
    let provider = match MySqlProvider::new(&url).await {
        Ok(p) => Arc::new(p) as Arc<dyn rust_ef::provider::IDatabaseProvider>,
        Err(e) => {
            eprintln!("skip test_mysql_count_and_any: {e}");
            return;
        }
    };
    common::run_count_and_any(provider, rust_ef::migration::MigrationDialect::MySql)
        .await
        .expect("mysql count and any");
}

#[tokio::test]
async fn test_mysql_aggregation_queries() {
    let Some(url) = mysql_url() else {
        eprintln!("skip test_mysql_aggregation_queries: RUST_EF_MYSQL_URL not set");
        return;
    };
    let provider = match MySqlProvider::new(&url).await {
        Ok(p) => Arc::new(p) as Arc<dyn rust_ef::provider::IDatabaseProvider>,
        Err(e) => {
            eprintln!("skip test_mysql_aggregation_queries: {e}");
            return;
        }
    };
    common::run_aggregation_queries(provider, rust_ef::migration::MigrationDialect::MySql)
        .await
        .expect("mysql aggregation queries");
}

#[tokio::test]
async fn test_mysql_empty_result_handling() {
    let Some(url) = mysql_url() else {
        eprintln!("skip test_mysql_empty_result_handling: RUST_EF_MYSQL_URL not set");
        return;
    };
    let provider = match MySqlProvider::new(&url).await {
        Ok(p) => Arc::new(p) as Arc<dyn rust_ef::provider::IDatabaseProvider>,
        Err(e) => {
            eprintln!("skip test_mysql_empty_result_handling: {e}");
            return;
        }
    };
    common::run_empty_result_handling(provider, rust_ef::migration::MigrationDialect::MySql)
        .await
        .expect("mysql empty result handling");
}

#[tokio::test]
async fn test_mysql_ensure_created_and_deleted() {
    let Some(url) = mysql_url() else {
        eprintln!("skip test_mysql_ensure_created_and_deleted: RUST_EF_MYSQL_URL not set");
        return;
    };
    let provider = match MySqlProvider::new(&url).await {
        Ok(p) => Arc::new(p) as Arc<dyn rust_ef::provider::IDatabaseProvider>,
        Err(e) => {
            eprintln!("skip test_mysql_ensure_created_and_deleted: {e}");
            return;
        }
    };
    common::run_ensure_created_and_deleted(provider, rust_ef::migration::MigrationDialect::MySql)
        .await
        .expect("mysql ensure_created and ensure_deleted");
}

#[tokio::test]
async fn test_mysql_has_data_seed() {
    let Some(url) = mysql_url() else {
        eprintln!("skip test_mysql_has_data_seed: RUST_EF_MYSQL_URL not set");
        return;
    };
    let provider = match MySqlProvider::new(&url).await {
        Ok(p) => Arc::new(p) as Arc<dyn rust_ef::provider::IDatabaseProvider>,
        Err(e) => {
            eprintln!("skip test_mysql_has_data_seed: {e}");
            return;
        }
    };
    common::run_has_data_seed(provider, rust_ef::migration::MigrationDialect::MySql)
        .await
        .expect("mysql has_data seed");
}
