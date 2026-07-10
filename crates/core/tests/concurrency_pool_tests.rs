//! Verifies E5: concurrent DbContext operations with shared connection pool.
//!
//! Multiple `tokio::spawn` tasks each create their own `DbContext` from the
//! same `DbContextOptions` (shared provider cache / connection pool). Verifies
//! pool reuse works without contention or data corruption.

mod common;

use common::TestItem;
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[tokio::test]
async fn concurrent_dbcontext_operations_share_pool() {
    // Build shared options — the provider cache (connection pool) is created
    // once and reused across all DbContext instances.
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();

    // Ensure the schema exists before concurrent tasks.
    {
        let mut ctx = DbContext::from_options(&options).expect("ctx");
        ctx.set::<TestItem>();
        ctx.ensure_created().await.expect("ensure_created");
    }

    // Spawn 8 concurrent tasks, each doing INSERT + QUERY.
    let task_count = 8usize;
    let mut handles = Vec::with_capacity(task_count);
    for i in 0..task_count {
        let opts = options.clone();
        handles.push(tokio::spawn(async move {
            let mut ctx = DbContext::from_options(&opts).expect("ctx");
            ctx.add::<TestItem>(TestItem {
                id: 0,
                name: format!("Task-{i}"),
                value: i as f64,
            });
            ctx.save_changes().await.expect("save");

            let rows = ctx
                .set::<TestItem>()
                .query()
                .to_list()
                .await
                .expect("query");
            (i, rows.len())
        }));
    }

    // All tasks should complete successfully.
    let mut results = Vec::with_capacity(task_count);
    for handle in handles {
        results.push(handle.await.expect("task panicked"));
    }

    // Each task should have seen at least its own insert (plus others' due to
    // shared in-memory DB). The key assertion: no errors, no panics.
    for (i, row_count) in &results {
        assert!(
            *row_count > 0,
            "task {i} saw 0 rows — concurrent insert failed"
        );
    }

    // Final consistency check: total rows should equal task_count.
    let mut ctx = DbContext::from_options(&options).expect("ctx");
    let final_rows = ctx
        .set::<TestItem>()
        .query()
        .to_list()
        .await
        .expect("final query");
    assert_eq!(
        final_rows.len(),
        task_count,
        "expected {task_count} rows after concurrent inserts, got {}",
        final_rows.len()
    );
}
