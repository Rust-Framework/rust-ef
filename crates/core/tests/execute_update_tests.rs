//! EF-3: `ExecuteUpdateBuilder::set_column` is a public, semver-stable entry
//! point for dynamic bulk updates (it used to be `#[doc(hidden)]
//! set_column_internal`).
//!
//! Covered here:
//! - the public method builds the expected `UPDATE ... SET ... WHERE ...` SQL;
//! - a compare-and-swap guard (status/version) plus version bump is a single
//!   statement, and a stale guard reports `0` affected rows;
//! - the deprecated `set_column_internal` alias still works for one release.

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[derive(Debug, Clone, EntityType)]
#[table("ef3_orders")]
struct Ef3Order {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    status: String,
    version: i32,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).unwrap()
}

/// Seeds two `pending` orders at version 1.
async fn seed() -> DbContext {
    let mut ctx = make_ctx();
    ctx.set::<Ef3Order>();
    ctx.ensure_created().await.unwrap();
    for id in 1..=2 {
        ctx.add::<Ef3Order>(Ef3Order {
            id,
            status: "pending".into(),
            version: 1,
        });
    }
    ctx.save_changes().await.unwrap();
    ctx
}

#[tokio::test]
async fn public_set_column_builds_update_sql() {
    let mut ctx = make_ctx();
    ctx.set::<Ef3Order>();

    let sql = ctx
        .set::<Ef3Order>()
        .query()
        .filter(linq!(|o: Ef3Order| o.id == 1))
        .execute_update()
        .set_column(Ef3Order::COLUMN_STATUS, "paid")
        .set_column(Ef3Order::COLUMN_VERSION, 2)
        .to_sql();

    assert!(
        sql.starts_with("UPDATE ef3_orders SET status = ?, version = ?"),
        "unexpected UPDATE SQL: {sql}"
    );
    assert!(sql.contains("WHERE"), "expected a WHERE clause: {sql}");
}

#[tokio::test]
async fn cas_guard_and_version_bump_are_one_statement() {
    let mut ctx = seed().await;

    // Winner: version guard still matches, bump it in the same statement.
    let affected = ctx
        .set::<Ef3Order>()
        .query()
        .filter(linq!(|o: Ef3Order| o.id == 1 && o.version == 1))
        .execute_update()
        .set_column(Ef3Order::COLUMN_STATUS, "paid")
        .set_column(Ef3Order::COLUMN_VERSION, 2)
        .execute()
        .await
        .unwrap();
    assert_eq!(affected, 1);

    // Loser: the stale guard no longer matches → 0 rows, no partial update.
    let affected = ctx
        .set::<Ef3Order>()
        .query()
        .filter(linq!(|o: Ef3Order| o.id == 1 && o.version == 1))
        .execute_update()
        .set_column(Ef3Order::COLUMN_STATUS, "refunded")
        .execute()
        .await
        .unwrap();
    assert_eq!(affected, 0, "stale CAS guard must not update the row");

    let order = ctx
        .set::<Ef3Order>()
        .query()
        .filter(linq!(|o: Ef3Order| o.id == 1))
        .first_or_default()
        .await
        .unwrap()
        .expect("order 1 exists");
    assert_eq!(order.status, "paid");
    assert_eq!(order.version, 2);
}

#[tokio::test]
#[allow(deprecated)]
async fn deprecated_set_column_internal_alias_still_works() {
    let mut ctx = seed().await;

    let affected = ctx
        .set::<Ef3Order>()
        .query()
        .filter(linq!(|o: Ef3Order| o.id == 2))
        .execute_update()
        .set_column_internal(Ef3Order::COLUMN_STATUS, "alias")
        .execute()
        .await
        .unwrap();
    assert_eq!(affected, 1);

    let order = ctx
        .set::<Ef3Order>()
        .query()
        .filter(linq!(|o: Ef3Order| o.id == 2))
        .first_or_default()
        .await
        .unwrap()
        .expect("order 2 exists");
    assert_eq!(order.status, "alias");
}
