//! `DbContext` creation shares the process-level `Arc<BuiltMetadata>`.
//!
//! 2.8 removed the per-context deep clone of every entity meta/config; this
//! test locks the observable consequence (a metadata-cache hit yields two
//! contexts with the same `Arc` pointers).

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[test]
fn contexts_from_same_options_share_built_metadata() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();

    let first = DbContext::from_options(&options).expect("first context");
    let second = DbContext::from_options(&options).expect("second context");

    assert!(
        first.shares_metadata_with(&second),
        "contexts from the same options must share Arc<BuiltMetadata>"
    );
}
