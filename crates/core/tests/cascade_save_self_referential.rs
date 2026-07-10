//! Cascade save tests: self-referential tree insert and cascade delete.

mod common;

use common::*;
use rust_ef::db_context::{DbContext, DbContextOptionsBuilder};
use rust_ef::prelude::*;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

#[tokio::test]
async fn cascade_insert_self_referential_tree() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).unwrap();

    ctx.set::<CascadeCategory>();
    ctx.ensure_created().await.unwrap();

    let root = CascadeCategory {
        category_id: 0,
        name: "Root".into(),
        parent_id: 0,
        children: HasMany::with(vec![
            CascadeCategory {
                category_id: 0,
                name: "Child A".into(),
                parent_id: 0,
                children: HasMany::new(),
            },
            CascadeCategory {
                category_id: 0,
                name: "Child B".into(),
                parent_id: 0,
                children: HasMany::new(),
            },
        ]),
    };
    ctx.add::<CascadeCategory>(root);
    ctx.save_changes().await.unwrap();

    let categories = ctx
        .set::<CascadeCategory>()
        .query()
        .to_list()
        .await
        .unwrap();
    assert_eq!(categories.len(), 3, "Root + 2 children");

    let root = categories
        .iter()
        .find(|c| c.name == "Root")
        .expect("Root should exist");
    assert!(root.category_id > 0, "Root PK should be backfilled");

    let children: Vec<&CascadeCategory> = categories
        .iter()
        .filter(|c| c.name == "Child A" || c.name == "Child B")
        .collect();
    assert_eq!(children.len(), 2);
    for child in &children {
        assert_eq!(
            child.parent_id, root.category_id,
            "Child parent_id should be fixed up via self-ref UPDATE"
        );
    }
}

#[tokio::test]
async fn cascade_delete_self_referential() {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    let mut ctx = DbContext::from_options(&options).unwrap();

    ctx.set::<CascadeCategory>();
    ctx.ensure_created().await.unwrap();

    let root = CascadeCategory {
        category_id: 0,
        name: "Root".into(),
        parent_id: 0,
        children: HasMany::with(vec![
            CascadeCategory {
                category_id: 0,
                name: "Child A".into(),
                parent_id: 0,
                children: HasMany::new(),
            },
            CascadeCategory {
                category_id: 0,
                name: "Child B".into(),
                parent_id: 0,
                children: HasMany::new(),
            },
        ]),
    };
    ctx.add::<CascadeCategory>(root);
    ctx.save_changes().await.unwrap();

    // Re-query with include to populate children, then mark Deleted
    ctx.set::<CascadeCategory>().clear_entries();
    let loaded = ctx
        .set::<CascadeCategory>()
        .query()
        .include_internal("children")
        .to_list()
        .await
        .unwrap();
    let root_loaded = loaded
        .iter()
        .find(|c| c.name == "Root")
        .expect("Root should exist");
    assert_eq!(
        root_loaded.children.len(),
        2,
        "Children should be loaded via include"
    );

    let root_idx = loaded.iter().position(|c| c.name == "Root").unwrap();
    let root_entity = loaded.into_iter().nth(root_idx).unwrap();
    ctx.attach::<CascadeCategory>(root_entity);
    ctx.remove_at::<CascadeCategory>(0).unwrap();
    ctx.save_changes().await.unwrap();

    let categories = ctx
        .set::<CascadeCategory>()
        .query()
        .to_list()
        .await
        .unwrap();
    assert!(
        categories.is_empty(),
        "All categories should be deleted (cascade)"
    );
}
