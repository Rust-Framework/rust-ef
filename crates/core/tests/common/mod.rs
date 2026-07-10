//! Shared CRUD test entity and helpers for multi-provider integration tests.

#![allow(dead_code, unused_imports)]

mod setup;
mod test_item;

pub use setup::{
    db_context_with_provider, reset_schema, run_aggregation_queries, run_count_and_any,
    run_crud_lifecycle, run_empty_result_handling, run_ensure_created_and_deleted,
    run_filter_with_in_operator, run_has_data_seed, run_limit_and_offset,
};
pub use test_item::TestItem;
