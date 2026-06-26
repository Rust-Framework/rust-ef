//! Integration tests for:
//! - Transaction rollback: `save_changes` must roll back ALL DbSet writes when
//!   any single set fails at the DB level (atomic across entity types).
//! - Composite primary key CRUD lifecycle (insert / find / update / delete).

use rust_ef::db_context::{DbContext, DbContextOptionsBuilder, IDbContext};
use rust_ef::prelude::*;
use rust_ef::provider::DbValue;
use rust_ef_sqlite::DbContextOptionsBuilderExt as _;

// -----------------------------------------------------------------------
// Entities
// -----------------------------------------------------------------------

/// A "good" entity whose insert should be rolled back when the sibling set fails.
#[derive(Debug, Clone, EntityType)]
#[table("good_items")]
struct GoodItem {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[required]
    name: String,
}

/// An entity with a UNIQUE constraint. Inserting a duplicate value triggers a
/// DB-level error inside `save_changes`, forcing a rollback of all prior writes
/// in the same transaction.
#[derive(Debug, Clone, EntityType)]
#[table("unique_items")]
struct UniqueItem {
    #[primary_key]
    #[auto_increment]
    id: i32,
    #[unique]
    #[required]
    code: String,
}

/// Composite-PK entity for the CRUD lifecycle test.
#[derive(Debug, Clone, EntityType)]
#[table("enrollments")]
struct Enrollment {
    #[primary_key]
    student_id: i32,
    #[primary_key]
    course_id: i32,
    #[required]
    grade: String,
}

fn make_ctx() -> DbContext {
    let mut builder = DbContextOptionsBuilder::new();
    builder.use_sqlite_in_memory();
    let options = builder.build();
    DbContext::from_options(&options).unwrap()
}

// -----------------------------------------------------------------------
// Transaction rollback
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_save_changes_rolls_back_on_error() {
    let mut ctx = make_ctx();
    ctx.set::<UniqueItem>();
    ctx.set::<GoodItem>();
    ctx.ensure_created().await.unwrap();

    // ensure_created only emits CREATE TABLE (not CREATE INDEX), so the
    // #[unique] attribute doesn't produce a DB-level constraint yet.
    // Manually create the UNIQUE index to enforce it for this test.
    ctx.provider()
        .execute_migration_command(
            "CREATE UNIQUE INDEX IF NOT EXISTS ix_unique_items_code ON unique_items (code)",
        )
        .await
        .unwrap();

    // Phase 1: pre-seed a UniqueItem with code "X" (this commits).
    ctx.set::<UniqueItem>().add(UniqueItem {
        id: 0,
        code: "X".into(),
    });
    ctx.save_changes().await.expect("pre-seed should commit");

    // Phase 2: add a GoodItem AND a duplicate UniqueItem in the SAME
    // save_changes call. The GoodItem insert runs first (succeeds within the
    // transaction), then the duplicate UniqueItem insert hits the UNIQUE
    // constraint → save_changes rolls back the entire transaction.
    ctx.set::<GoodItem>().add(GoodItem {
        id: 0,
        name: "should-be-rolled-back".into(),
    });
    ctx.set::<UniqueItem>().add(UniqueItem {
        id: 0,
        code: "X".into(), // duplicate — triggers UNIQUE violation
    });

    let result = ctx.save_changes().await;
    assert!(
        result.is_err(),
        "save_changes should fail when a UNIQUE constraint is violated"
    );

    // The GoodItem insert must have been rolled back.
    let good_count = ctx
        .set::<GoodItem>()
        .query()
        .count()
        .await
        .expect("count good_items");
    assert_eq!(
        good_count, 0,
        "GoodItem insert should have been rolled back (atomic transaction)"
    );

    // The original UniqueItem row should still be the only one.
    let unique_count = ctx
        .set::<UniqueItem>()
        .query()
        .count()
        .await
        .expect("count unique_items");
    assert_eq!(
        unique_count, 1,
        "only the pre-seeded UniqueItem should remain"
    );
}

#[tokio::test]
async fn test_save_changes_commits_all_on_success() {
    let mut ctx = make_ctx();
    ctx.set::<GoodItem>();
    ctx.set::<UniqueItem>();
    ctx.ensure_created().await.unwrap();

    ctx.set::<GoodItem>().add(GoodItem {
        id: 0,
        name: "alpha".into(),
    });
    ctx.set::<UniqueItem>().add(UniqueItem {
        id: 0,
        code: "Y".into(),
    });

    ctx.save_changes().await.expect("save should succeed");

    let good_count = ctx.set::<GoodItem>().query().count().await.unwrap();
    let unique_count = ctx.set::<UniqueItem>().query().count().await.unwrap();
    assert_eq!(good_count, 1, "GoodItem committed");
    assert_eq!(unique_count, 1, "UniqueItem committed");
}

// -----------------------------------------------------------------------
// Composite primary key CRUD lifecycle
// -----------------------------------------------------------------------

#[tokio::test]
async fn test_composite_pk_insert_and_find_by_key() {
    let mut ctx = make_ctx();
    ctx.set::<Enrollment>();
    ctx.ensure_created().await.unwrap();

    ctx.set::<Enrollment>().add(Enrollment {
        student_id: 100,
        course_id: 200,
        grade: "A".into(),
    });
    ctx.save_changes().await.unwrap();

    let found = ctx
        .set::<Enrollment>()
        .query()
        .find_by_key(&[
            ("student_id", DbValue::I32(100)),
            ("course_id", DbValue::I32(200)),
        ])
        .await
        .expect("find_by_key")
        .expect("enrollment should exist");
    assert_eq!(found.student_id, 100);
    assert_eq!(found.course_id, 200);
    assert_eq!(found.grade, "A");
}

#[tokio::test]
async fn test_composite_pk_find_by_key_returns_none_for_partial_match() {
    let mut ctx = make_ctx();
    ctx.set::<Enrollment>();
    ctx.ensure_created().await.unwrap();

    ctx.set::<Enrollment>().add(Enrollment {
        student_id: 100,
        course_id: 200,
        grade: "A".into(),
    });
    ctx.save_changes().await.unwrap();

    // Wrong course_id — should not find.
    let found = ctx
        .set::<Enrollment>()
        .query()
        .find_by_key(&[
            ("student_id", DbValue::I32(100)),
            ("course_id", DbValue::I32(999)),
        ])
        .await
        .expect("find_by_key");
    assert!(
        found.is_none(),
        "partial composite key match should return None"
    );
}

#[tokio::test]
async fn test_composite_pk_update() {
    let mut ctx = make_ctx();
    ctx.set::<Enrollment>();
    ctx.ensure_created().await.unwrap();

    // Insert
    ctx.set::<Enrollment>().add(Enrollment {
        student_id: 100,
        course_id: 200,
        grade: "B".into(),
    });
    ctx.save_changes().await.unwrap();

    // Load, modify, save (must reload via load_all to track for update)
    ctx.set::<Enrollment>().load_all().await.unwrap();
    ctx.set::<Enrollment>()
        .tracked_entries_mut()
        .next()
        .unwrap()
        .grade = "A+".into();
    ctx.save_changes().await.unwrap();

    // Verify the update persisted
    let found = ctx
        .set::<Enrollment>()
        .query()
        .find_by_key(&[
            ("student_id", DbValue::I32(100)),
            ("course_id", DbValue::I32(200)),
        ])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.grade, "A+", "composite PK row should be updated");
}

#[tokio::test]
async fn test_composite_pk_delete() {
    let mut ctx = make_ctx();
    ctx.set::<Enrollment>();
    ctx.ensure_created().await.unwrap();

    // Insert two rows
    ctx.set::<Enrollment>().add(Enrollment {
        student_id: 100,
        course_id: 200,
        grade: "A".into(),
    });
    ctx.set::<Enrollment>().add(Enrollment {
        student_id: 100,
        course_id: 300,
        grade: "B".into(),
    });
    ctx.save_changes().await.unwrap();

    // Load and delete one
    ctx.set::<Enrollment>().load_all().await.unwrap();
    let idx = ctx
        .set::<Enrollment>()
        .tracked_entries()
        .position(|e| e.course_id == 200)
        .expect("find row to delete");
    ctx.set::<Enrollment>()
        .remove_at(idx)
        .expect("remove_at marks as Deleted");
    ctx.save_changes().await.unwrap();

    // Verify: one row remains (course 300), the other is gone
    let remaining = ctx.set::<Enrollment>().query().to_list().await.unwrap();
    assert_eq!(
        remaining.len(),
        1,
        "one enrollment should remain after delete"
    );
    assert_eq!(remaining[0].course_id, 300);

    let exists_deleted = ctx
        .set::<Enrollment>()
        .query()
        .exists_by_key(&[
            ("student_id", DbValue::I32(100)),
            ("course_id", DbValue::I32(200)),
        ])
        .await
        .unwrap();
    assert!(!exists_deleted, "deleted composite PK row should not exist");
}
